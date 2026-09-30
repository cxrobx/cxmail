//! The in-app chat: one long-lived `claude -p` process per conversation.
//!
//! The pure half — argv, tool tiers, MCP config, standing instructions, the
//! stream parser — is `email::chat_agent`, where it is unit-tested. This file
//! owns the process: spawn it, feed its stdin, turn its stdout into
//! `chat-event`s, and answer its permission questions with whatever the user
//! clicks.
//!
//! Three tasks per session, and who owns what matters:
//! - **writer** owns stdin. Everything that talks to the CLI (a message, a
//!   permission answer, an interrupt) goes through its channel, so two
//!   writers can never interleave half-lines. When every sender is dropped,
//!   stdin closes and the CLI exits on EOF — which is also what happens if
//!   CXMail itself quits, so no exit hook is needed to reap it.
//! - **reader** owns stdout and the parser.
//! - **supervisor** owns the `Child`, so a kill never waits on a lock the
//!   `wait()` is holding, and it emits `exited` exactly once.
//!
//! Every event carries the session's generation: after "New chat", a late line
//! from the old process must not land in the new conversation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::commands::claude_handoff;
use crate::db;
use crate::db::claude_repos::ResolvedRepo;
use crate::email::chat_agent::{self, ChatEvent, Inbound};
use crate::error::AppError;
use crate::{AppState, LockExt};

/// How long the CLI gets to answer the `initialize` handshake. It answers in
/// well under a second; silence means the control protocol is not being
/// spoken, and running on without it would mean running without the gate.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// A message longer than this is a paste accident, not a chat turn.
const MAX_MESSAGE_CHARS: usize = 50_000;

const INIT_REQUEST_ID: &str = "cxmail-init";

struct Shared {
    session_id: StdMutex<Option<String>>,
    /// Tools the user said "allow for this chat" to.
    always_allow: StdMutex<HashSet<String>>,
    /// Open permission questions: request id → (tool name, input as sent).
    pending: StdMutex<HashMap<String, (String, Value)>>,
    handshake_ok: AtomicBool,
}

struct ChatSession {
    gen: u64,
    tx: mpsc::UnboundedSender<String>,
    kill: Option<oneshot::Sender<()>>,
    /// Fires once the process is gone — "Continue in terminal" waits on it so
    /// two processes never write the same session file.
    exited: Option<oneshot::Receiver<()>>,
    shared: Arc<Shared>,
    cwd: PathBuf,
    add_dirs: Vec<PathBuf>,
}

static CHAT: LazyLock<tokio::sync::Mutex<Option<ChatSession>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Clone)]
struct Envelope<'a> {
    gen: u64,
    event: &'a ChatEvent,
}

fn emit(app: &AppHandle, gen: u64, event: &ChatEvent) {
    if let Err(e) = app.emit("chat-event", Envelope { gen, event }) {
        log::warn!("chat: emit failed: {e}");
    }
}

#[derive(Debug, Deserialize)]
pub struct ChatSeed {
    pub account_id: String,
    pub folder: String,
    pub uid: u32,
}

#[derive(Debug, Serialize)]
pub struct ChatStarted {
    pub gen: u64,
    /// Where the session runs.
    pub cwd: String,
    /// The mapping that put it there, when it was opened from an email.
    pub repo: Option<ResolvedRepo>,
    /// That email's mapping pointed at a directory that is not there.
    pub missing_repo_path: Option<String>,
    /// Every project directory the session can read.
    pub readable_repos: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub seed_subject: Option<String>,
}

/// Start a new conversation, ending any current one.
#[tauri::command]
pub async fn chat_start(
    app: AppHandle,
    state: State<'_, AppState>,
    seed: Option<ChatSeed>,
    model: Option<String>,
) -> Result<ChatStarted, AppError> {
    let model = model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
    if let Some(m) = &model {
        chat_agent::validate_model(m).map_err(AppError::General)?;
    }
    stop_current().await;

    let (cli, login_path) = tokio::task::spawn_blocking(|| {
        (claude_handoff::find_claude_cli(), login_shell_path())
    })
    .await
    .map_err(|e| AppError::General(format!("spawn_blocking join error: {e}")))?;
    let cli = cli.ok_or_else(|| {
        AppError::General(
            "Claude Code CLI (`claude`) was not found. Install it and sign in, then try again."
                .into(),
        )
    })?;

    // One small lock scope for every read (gotcha #11).
    let (mappings, seed_info) = {
        let conn = state.db.safe_lock();
        let mut mappings = Vec::new();
        for m in db::claude_repos::list(&conn)? {
            let label = db::claude_repos::describe(&conn, &m);
            mappings.push((label, m.repo_path));
        }
        let seed_info = match &seed {
            Some(s) => {
                let row = db::messages::get_by_uid(&conn, &s.account_id, &s.folder, s.uid)?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("Message uid {} not found in {}", s.uid, s.folder))
                    })?;
                let repo =
                    db::claude_repos::resolve_for_message(&conn, &s.account_id, &s.folder, s.uid)?;
                Some((row, repo))
            }
            None => None,
        };
        (mappings, seed_info)
    };

    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::General(format!("failed to resolve app_data_dir: {e}")))?;
    // One fixed home, not one per chat: Claude Code files sessions under the
    // working directory, and "Continue in terminal" resumes from the same one.
    let chat_home = app_dir.join("claude-chat");
    std::fs::create_dir_all(&chat_home)?;

    let (seed_repo, missing_repo_path) = match &seed_info {
        Some((_, repo)) => claude_handoff::split_on_existence(repo.clone()),
        None => (None, None),
    };
    let cwd = seed_repo
        .as_ref()
        .map(|r| PathBuf::from(&r.repo_path))
        .unwrap_or_else(|| chat_home.clone());

    // Readable directories: every mapped repo that is actually there, one line
    // per directory however many mappings name it.
    let mut repo_lines: Vec<chat_agent::RepoLine> = Vec::new();
    for (label, path) in mappings {
        if !Path::new(&path).is_dir() {
            continue;
        }
        match repo_lines.iter_mut().find(|r| r.path == path) {
            Some(existing) => existing.label = format!("{}, {label}", existing.label),
            None => repo_lines.push(chat_agent::RepoLine { label, path }),
        }
    }
    let add_dirs: Vec<PathBuf> = repo_lines
        .iter()
        .map(|r| PathBuf::from(&r.path))
        .filter(|p| *p != cwd)
        .collect();

    let claude_json = dirs::home_dir()
        .and_then(|h| std::fs::read_to_string(h.join(".claude.json")).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let (mcp_config, servers) =
        chat_agent::build_mcp_config(claude_json.as_ref(), bundled_mcp_binary().as_deref());
    if !servers.iter().any(|s| s == "cxmail") {
        return Err(AppError::General(
            "No cxmail MCP binary found for the chat — neither beside the app nor registered \
             in ~/.claude.json."
                .into(),
        ));
    }
    let mcp_path = chat_home.join("mcp.json");
    write_private(&mcp_path, &serde_json::to_string_pretty(&mcp_config).unwrap_or_default())?;

    let seed_message = seed_info.as_ref().zip(seed.as_ref()).map(|((row, _), s)| {
        chat_agent::SeedMessage {
            account_id: s.account_id.clone(),
            folder: s.folder.clone(),
            uid: s.uid,
            subject: row.subject.clone().unwrap_or_else(|| "(no subject)".into()),
            from: format!(
                "{} <{}>",
                row.from_name.as_deref().unwrap_or(""),
                row.from_email.as_deref().unwrap_or("")
            ),
        }
    });
    let prompt = chat_agent::build_system_prompt(
        &repo_lines,
        seed_message.as_ref(),
        seed_repo.as_ref().map(|r| r.repo_path.as_str()),
        servers.iter().any(|s| s == "vault"),
        servers.iter().any(|s| s == "cxtasks"),
    );
    let args = chat_agent::build_args(&chat_agent::ChatLaunch {
        mcp_config: &mcp_path,
        servers: &servers,
        system_prompt: &prompt,
        model: model.as_deref(),
        add_dirs: &add_dirs,
    });

    let mut cmd = Command::new(&cli);
    cmd.args(&args)
        .current_dir(&cwd)
        // The user's hooks stay on — the secret-exposure guard among them —
        // but a chat turn ending is not a reason to ding or post a banner.
        .env("CLAUDE_HOOK_SOUND", "0")
        .env("CLAUDE_HOOK_BANNER", "0")
        .env("CXMAIL_CHAT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // A GUI app inherits a bare PATH; hooks and stdio MCP servers expect the
    // login shell's (jq, sqlite3, node, uvx).
    if let Some(path) = login_path {
        cmd.env("PATH", path);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::General(format!("Failed to start Claude ({}): {e}", cli.display())))?;

    let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let shared = Arc::new(Shared {
        session_id: StdMutex::new(None),
        always_allow: StdMutex::new(HashSet::new()),
        pending: StdMutex::new(HashMap::new()),
        handshake_ok: AtomicBool::new(false),
    });
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    let (exited_tx, exited_rx) = oneshot::channel::<()>();

    let stdin = child.stdin.take().ok_or_else(|| AppError::General("no stdin".into()))?;
    let stdout = child.stdout.take().ok_or_else(|| AppError::General("no stdout".into()))?;
    let stderr = child.stderr.take().ok_or_else(|| AppError::General("no stderr".into()))?;

    tokio::spawn(write_loop(stdin, rx));
    let reader = tokio::spawn(read_loop(app.clone(), gen, stdout, tx.clone(), shared.clone()));
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = BufReader::new(stderr).read_to_end(&mut buf).await;
        let text = String::from_utf8_lossy(&buf);
        // The tail is what explains a crash; the head is usually a banner.
        let tail: String = text.chars().rev().take(2000).collect::<Vec<_>>().into_iter().rev().collect();
        tail
    });
    {
        let app = app.clone();
        tokio::spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s.ok(),
                _ = kill_rx => {
                    let _ = child.kill().await;
                    child.wait().await.ok()
                }
            };
            let _ = reader.await;
            let stderr_tail = stderr_task.await.unwrap_or_default();
            if !stderr_tail.trim().is_empty() {
                log::info!("chat: claude exited ({status:?}); stderr tail: {stderr_tail}");
            }
            emit(
                &app,
                gen,
                &ChatEvent::Exited {
                    code: status.and_then(|s| s.code()),
                    stderr_tail,
                },
            );
            let _ = exited_tx.send(());
        });
    }

    let _ = tx.send(chat_agent::initialize_line(INIT_REQUEST_ID));
    {
        let app = app.clone();
        let shared = shared.clone();
        tokio::spawn(async move {
            tokio::time::sleep(HANDSHAKE_TIMEOUT).await;
            if shared.handshake_ok.load(Ordering::SeqCst) {
                return;
            }
            let mut guard = CHAT.lock().await;
            if guard.as_ref().map(|s| s.gen) == Some(gen) {
                emit(
                    &app,
                    gen,
                    &ChatEvent::Notice {
                        text: "Claude Code did not answer the chat handshake, so it was stopped \
                               rather than run without CXMail's permission gate. Is the CLI up \
                               to date and signed in?"
                            .into(),
                    },
                );
                if let Some(mut s) = guard.take() {
                    if let Some(k) = s.kill.take() {
                        let _ = k.send(());
                    }
                }
            }
        });
    }

    log::info!(
        "chat: started gen {gen} in {:?} (servers {:?}, {} readable repos)",
        cwd,
        servers,
        repo_lines.len()
    );
    let started = ChatStarted {
        gen,
        cwd: cwd.to_string_lossy().into_owned(),
        repo: seed_repo,
        missing_repo_path,
        readable_repos: repo_lines.iter().map(|r| r.path.clone()).collect(),
        mcp_servers: servers,
        seed_subject: seed_message.map(|m| m.subject),
    };
    *CHAT.lock().await = Some(ChatSession {
        gen,
        tx,
        kill: Some(kill_tx),
        exited: Some(exited_rx),
        shared,
        cwd,
        add_dirs,
    });
    Ok(started)
}

async fn write_loop(mut stdin: tokio::process::ChildStdin, mut rx: mpsc::UnboundedReceiver<String>) {
    while let Some(line) = rx.recv().await {
        let ok = stdin.write_all(line.as_bytes()).await.is_ok()
            && stdin.write_all(b"\n").await.is_ok()
            && stdin.flush().await.is_ok();
        if !ok {
            break;
        }
    }
    // Dropping stdin here is the EOF that ends the CLI.
}

async fn read_loop(
    app: AppHandle,
    gen: u64,
    stdout: tokio::process::ChildStdout,
    tx: mpsc::UnboundedSender<String>,
    shared: Arc<Shared>,
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut parser = chat_agent::StreamParser::default();
    while let Ok(Some(line)) = lines.next_line().await {
        for inbound in parser.feed(&line) {
            match inbound {
                Inbound::Event(event) => {
                    if let ChatEvent::Ready { session_id, .. } = &event {
                        *shared.session_id.lock().unwrap_or_else(|p| p.into_inner()) =
                            Some(session_id.clone());
                    }
                    emit(&app, gen, &event);
                }
                Inbound::CanUseTool {
                    request_id,
                    tool_name,
                    display_name,
                    input,
                } => {
                    let rememberable = chat_agent::can_remember(&tool_name);
                    let remembered = rememberable
                        && shared
                            .always_allow
                            .lock()
                            .map(|s| s.contains(&tool_name))
                            .unwrap_or(false);
                    if remembered {
                        let _ = tx.send(chat_agent::allow_line(&request_id, &input));
                        continue;
                    }
                    if let Ok(mut pending) = shared.pending.lock() {
                        pending.insert(request_id.clone(), (tool_name.clone(), input.clone()));
                    }
                    let context = permission_context(&app, &input);
                    emit(
                        &app,
                        gen,
                        &ChatEvent::PermissionRequest {
                            request_id,
                            tool_name,
                            display_name,
                            input,
                            rememberable,
                            context,
                        },
                    );
                    // A backgrounded window must still say it is waiting on you.
                    request_attention(&app);
                }
                Inbound::UnknownControl { request_id, subtype } => {
                    log::info!("chat: refusing unsupported control request {subtype:?}");
                    let _ = tx.send(chat_agent::control_error_line(
                        &request_id,
                        &format!("CXMail does not implement {subtype}"),
                    ));
                }
                Inbound::ControlResponse { request_id, ok, error } => {
                    if request_id == INIT_REQUEST_ID {
                        shared.handshake_ok.store(ok, Ordering::SeqCst);
                        if !ok {
                            emit(
                                &app,
                                gen,
                                &ChatEvent::Notice {
                                    text: format!(
                                        "Claude Code refused the chat handshake: {}",
                                        error.unwrap_or_default()
                                    ),
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Turn a permission question's ids into what a person can check: the
/// account's address and each message's subject and sender.
///
/// A read-only connection, so a sync holding the main mutex cannot stall the
/// question (gotcha #11). Every lookup that fails just leaves its field empty:
/// the card falls back to the raw input, and the question still gets asked.
fn permission_context(app: &AppHandle, input: &Value) -> Option<chat_agent::PermissionContext> {
    let (account_id, folder, uids) = chat_agent::message_refs(input);
    if account_id.is_none() && uids.is_empty() {
        return None;
    }
    let conn = app.state::<AppState>().open_read_conn().ok()?;
    let account = account_id
        .as_deref()
        .and_then(|id| db::accounts::get_by_id(&conn, id).ok().flatten())
        .map(|a| a.email);
    let messages = match (account_id.as_deref(), folder.as_deref()) {
        (Some(acct), Some(f)) => uids
            .iter()
            .take(chat_agent::PERMISSION_MESSAGE_PREVIEW)
            .filter_map(|uid| db::messages::get_by_uid(&conn, acct, f, *uid).ok().flatten())
            .map(|row| chat_agent::MessageBrief {
                subject: row.subject.unwrap_or_else(|| "(no subject)".into()),
                from: row
                    .from_name
                    .filter(|n| !n.trim().is_empty())
                    .or(row.from_email)
                    .unwrap_or_default(),
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(chat_agent::PermissionContext {
        account,
        folder,
        messages,
        message_count: uids.len(),
    })
}

fn request_attention(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

/// Send one message. The first message of a session is sent the same way —
/// the CLI queues it behind its own startup.
#[tauri::command]
pub async fn chat_send(text: String, images: Option<Vec<String>>) -> Result<(), AppError> {
    let text = text.trim();
    let images = images.unwrap_or_default();
    if text.is_empty() && images.is_empty() {
        return Err(AppError::General("Nothing to send.".into()));
    }
    if images.len() > chat_agent::MAX_IMAGES_PER_MESSAGE {
        return Err(AppError::General(format!(
            "A message can carry at most {} pictures.",
            chat_agent::MAX_IMAGES_PER_MESSAGE
        )));
    }
    let images = images
        .iter()
        .map(|i| chat_agent::validate_image(i))
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::General)?;
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(AppError::General(format!(
            "That message is over {MAX_MESSAGE_CHARS} characters."
        )));
    }
    let guard = CHAT.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| AppError::General("No chat is running — start a new one.".into()))?;
    session
        .tx
        .send(chat_agent::user_message_line(text, &images))
        .map_err(|_| AppError::General("The chat has ended — start a new one.".into()))
}

/// Answer a permission question. `remember` allows that tool for the rest of
/// this conversation (never across conversations).
#[tauri::command]
pub async fn chat_answer_permission(
    request_id: String,
    allow: bool,
    remember: bool,
) -> Result<(), AppError> {
    let guard = CHAT.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| AppError::General("No chat is running.".into()))?;
    let (tool, input) = session
        .shared
        .pending
        .lock()
        .ok()
        .and_then(|mut p| p.remove(&request_id))
        .ok_or_else(|| AppError::NotFound("That request was already answered.".into()))?;
    let line = if allow {
        // A CXTasks write asks every time, whatever the webview sent.
        if remember && chat_agent::can_remember(&tool) {
            if let Ok(mut set) = session.shared.always_allow.lock() {
                set.insert(tool);
            }
        }
        chat_agent::allow_line(&request_id, &input)
    } else {
        chat_agent::deny_line(
            &request_id,
            "The user declined this in CXMail. Do not retry it unless they ask.",
        )
    };
    session
        .tx
        .send(line)
        .map_err(|_| AppError::General("The chat has ended.".into()))
}

/// Stop the current turn; the conversation stays open.
#[tauri::command]
pub async fn chat_interrupt() -> Result<(), AppError> {
    let guard = CHAT.lock().await;
    if let Some(session) = guard.as_ref() {
        let id = format!("cxmail-interrupt-{}", uuid::Uuid::new_v4());
        let _ = session.tx.send(chat_agent::interrupt_line(&id));
    }
    Ok(())
}

/// End the conversation and its process.
#[tauri::command]
pub async fn chat_stop() -> Result<(), AppError> {
    stop_current().await;
    Ok(())
}

async fn stop_current() -> Option<ChatSession> {
    let mut session = CHAT.lock().await.take()?;
    if let Some(k) = session.kill.take() {
        let _ = k.send(());
    }
    Some(session)
}

/// Hand the conversation to a terminal: stop the in-app process, then open
/// `claude --resume <id>` in Ghostty from the same directory.
///
/// Stop FIRST and wait for the exit — two processes appending to one session
/// file is a corrupted transcript. And the same directory, because Claude Code
/// files sessions under the working directory they started in.
#[tauri::command]
pub async fn chat_continue_in_terminal(app: AppHandle) -> Result<(), AppError> {
    let ghostty = claude_handoff::find_ghostty_app()
        .ok_or_else(|| AppError::General("Ghostty is not installed.".into()))?;
    let session_id = {
        let guard = CHAT.lock().await;
        let session = guard
            .as_ref()
            .ok_or_else(|| AppError::General("No chat is running.".into()))?;
        let id = session.shared.session_id.lock().ok().and_then(|s| s.clone());
        id.filter(|id| id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
            .ok_or_else(|| {
                AppError::General("Send a message first — the chat has no session yet.".into())
            })?
    };
    let mut session = stop_current()
        .await
        .ok_or_else(|| AppError::General("No chat is running.".into()))?;
    if let Some(exited) = session.exited.take() {
        let _ = tokio::time::timeout(Duration::from_secs(5), exited).await;
    }
    let cli = tokio::task::spawn_blocking(claude_handoff::find_claude_cli)
        .await
        .map_err(|e| AppError::General(format!("spawn_blocking join error: {e}")))?
        .ok_or_else(|| AppError::General("Claude Code CLI not found.".into()))?;

    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::General(format!("failed to resolve app_data_dir: {e}")))?;
    let script_path = app_dir.join("claude-chat").join("resume.sh");
    std::fs::write(
        &script_path,
        resume_script(&cli, &session.cwd, &session_id, &session.add_dirs),
    )?;
    let mut perms = std::fs::metadata(&script_path)?.permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&script_path, perms)?;

    claude_handoff::launch_in_ghostty(ghostty, session.cwd.clone(), script_path).await
}

/// `--add-dir` last, as everywhere else (gotcha #49).
fn resume_script(cli: &Path, cwd: &Path, session_id: &str, add_dirs: &[PathBuf]) -> String {
    let q = |p: &Path| claude_handoff::shell_quote(&p.to_string_lossy());
    let dirs = if add_dirs.is_empty() {
        String::new()
    } else {
        format!(
            " --add-dir {}",
            add_dirs.iter().map(|d| q(d)).collect::<Vec<_>>().join(" ")
        )
    };
    format!(
        "#!/bin/zsh -l\nset -e\ncd {}\nexec {} --resume {}{dirs}\n",
        q(cwd),
        q(cli),
        claude_handoff::shell_quote(session_id),
    )
}

/// The `cxmail-mcp` bundled beside the running app — only inside a `.app`,
/// where it is signed and named in the Keychain ACL (gotcha #31). A loose dev
/// build's sibling is neither, so dev falls back to the registered binary.
fn bundled_mcp_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if !exe.to_string_lossy().contains(".app/Contents/MacOS/") {
        return None;
    }
    let sibling = exe.parent()?.join("cxmail-mcp");
    sibling.is_file().then_some(sibling)
}

/// The login shell's PATH, resolved once per run.
fn login_shell_path() -> Option<String> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        let out = std::process::Command::new("/bin/zsh")
            .arg("-lc")
            .arg("printf %s \"$PATH\"")
            .output()
            .ok()?;
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !path.is_empty()).then_some(path)
    })
    .clone()
}

/// The MCP config can name servers whose registrations carry env; keep it
/// readable by this user only.
fn write_private(path: &Path, contents: &str) -> Result<(), AppError> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resume script must survive the shell with every path intact —
    /// the chat home sits under `Application Support`, so the space is certain.
    #[test]
    fn the_resume_script_quotes_every_path_and_puts_add_dir_last() {
        let script = resume_script(
            Path::new("/Users/x/.local/bin/claude"),
            Path::new("/Users/x/Library/Application Support/com.cxmail.app/claude-chat"),
            "7415e5f1-37b5-4500-82c1-6e1493d7695c",
            &[PathBuf::from("/Users/x/Projects/it's a repo")],
        );
        let exec = script.lines().find(|l| l.starts_with("exec ")).expect("exec line");
        let probe = exec.replacen("exec ", "printf '%s\\n' ", 1);
        let out = std::process::Command::new("/bin/bash")
            .args(["--noprofile", "--norc", "-c", &probe])
            .output()
            .expect("bash");
        let words: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
        assert_eq!(
            words,
            vec![
                "/Users/x/.local/bin/claude",
                "--resume",
                "7415e5f1-37b5-4500-82c1-6e1493d7695c",
                "--add-dir",
                "/Users/x/Projects/it's a repo",
            ]
        );
        assert!(
            script.contains("cd '/Users/x/Library/Application Support/com.cxmail.app/claude-chat'"),
            "{script}"
        );
    }
}
