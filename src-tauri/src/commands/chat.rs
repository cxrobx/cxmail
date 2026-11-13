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

    let (cli, login_path) = locate_cli().await?;
    // One small lock scope for every read (gotcha #11).
    let inputs = {
        let conn = state.db.safe_lock();
        gather_launch_inputs(&conn, seed.as_ref())?
    };
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::General(format!("failed to resolve app_data_dir: {e}")))?;
    // One fixed home, not one per chat: Claude Code files sessions under the
    // working directory, and "Continue in terminal" resumes from the same one.
    let prepared = assemble_launch(cli, login_path, inputs, seed.as_ref(), &app_dir.join("claude-chat"), model.as_deref())?;
    let mut child = spawn_chat(&prepared)?;
    let Prepared { cwd, add_dirs, servers, repo_lines, seed_repo, missing_repo_path, seed_subject, .. } = prepared;

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
        seed_subject,
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

/// Everything one chat launch needs. Built the same way for the desktop panel
/// (`chat_start`) and for CXMail Remote's bridge (`run_remote_chat`), so the
/// flags that carry the chat's security (gotcha #64) cannot drift between them.
pub(crate) struct Prepared {
    cli: PathBuf,
    login_path: Option<String>,
    args: Vec<String>,
    cwd: PathBuf,
    add_dirs: Vec<PathBuf>,
    servers: Vec<String>,
    repo_lines: Vec<chat_agent::RepoLine>,
    seed_repo: Option<ResolvedRepo>,
    missing_repo_path: Option<String>,
    seed_subject: Option<String>,
}

type LaunchInputs = (Vec<(String, String)>, Option<(db::messages::MessageRow, Option<ResolvedRepo>)>);

async fn locate_cli() -> Result<(PathBuf, Option<String>), AppError> {
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
    Ok((cli, login_path))
}

/// The database reads a launch needs: every repo mapping, and the seed
/// message with the repo it maps to.
fn gather_launch_inputs(conn: &rusqlite::Connection, seed: Option<&ChatSeed>) -> Result<LaunchInputs, AppError> {
    let mut mappings = Vec::new();
    for m in db::claude_repos::list(conn)? {
        let label = db::claude_repos::describe(conn, &m);
        mappings.push((label, m.repo_path));
    }
    let seed_info = match seed {
        Some(s) => {
            let row = db::messages::get_by_uid(conn, &s.account_id, &s.folder, s.uid)?
                .ok_or_else(|| {
                    AppError::NotFound(format!("Message uid {} not found in {}", s.uid, s.folder))
                })?;
            let repo = db::claude_repos::resolve_for_message(conn, &s.account_id, &s.folder, s.uid)?;
            Some((row, repo))
        }
        None => None,
    };
    Ok((mappings, seed_info))
}

fn assemble_launch(
    cli: PathBuf,
    login_path: Option<String>,
    (mappings, seed_info): LaunchInputs,
    seed: Option<&ChatSeed>,
    chat_home: &Path,
    model: Option<&str>,
) -> Result<Prepared, AppError> {
    std::fs::create_dir_all(chat_home)?;

    let (seed_repo, missing_repo_path) = match &seed_info {
        Some((_, repo)) => claude_handoff::split_on_existence(repo.clone()),
        None => (None, None),
    };
    let cwd = seed_repo
        .as_ref()
        .map(|r| PathBuf::from(&r.repo_path))
        .unwrap_or_else(|| chat_home.to_path_buf());

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

    let seed_message = seed_info.as_ref().zip(seed).map(|((row, _), s)| {
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
        model,
        add_dirs: &add_dirs,
    });
    Ok(Prepared {
        cli,
        login_path,
        args,
        cwd,
        add_dirs,
        servers,
        repo_lines,
        seed_repo,
        missing_repo_path,
        seed_subject: seed_message.map(|m| m.subject),
    })
}

fn spawn_chat(p: &Prepared) -> Result<tokio::process::Child, AppError> {
    let mut cmd = Command::new(&p.cli);
    cmd.args(&p.args)
        .current_dir(&p.cwd)
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
    if let Some(path) = &p.login_path {
        cmd.env("PATH", path);
    }
    cmd.spawn()
        .map_err(|e| AppError::General(format!("Failed to start Claude ({}): {e}", p.cli.display())))
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
    pump_stdout(
        stdout,
        tx,
        shared,
        |event| emit(&app, gen, event),
        |input| permission_context(&app, input),
        // A backgrounded window must still say it is waiting on you.
        || request_attention(&app),
    )
    .await
}

/// The CLI's stdout, turned into events for whoever hosts the chat. One
/// implementation for the desktop panel and CXMail Remote's bridge, so the
/// permission gate (remembered tools, open questions, refused control
/// requests) is the same everywhere.
async fn pump_stdout(
    stdout: tokio::process::ChildStdout,
    tx: mpsc::UnboundedSender<String>,
    shared: Arc<Shared>,
    sink: impl Fn(&ChatEvent),
    context: impl Fn(&Value) -> Option<chat_agent::PermissionContext>,
    attention: impl Fn(),
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
                    sink(&event);
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
                    let ctx = context(&input);
                    sink(&ChatEvent::PermissionRequest {
                        request_id,
                        tool_name,
                        display_name,
                        input,
                        rememberable,
                        context: ctx,
                    });
                    attention();
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
                            sink(&ChatEvent::Notice {
                                text: format!(
                                    "Claude Code refused the chat handshake: {}",
                                    error.unwrap_or_default()
                                ),
                            });
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
    let (account_id, _, uids) = chat_agent::message_refs(input);
    if account_id.is_none() && uids.is_empty() {
        return None;
    }
    let conn = app.state::<AppState>().open_read_conn().ok()?;
    permission_context_from(&conn, input)
}

fn permission_context_from(conn: &rusqlite::Connection, input: &Value) -> Option<chat_agent::PermissionContext> {
    let (account_id, folder, uids) = chat_agent::message_refs(input);
    if account_id.is_none() && uids.is_empty() {
        return None;
    }
    let account = account_id
        .as_deref()
        .and_then(|id| db::accounts::get_by_id(conn, id).ok().flatten())
        .map(|a| a.email);
    let messages = match (account_id.as_deref(), folder.as_deref()) {
        (Some(acct), Some(f)) => uids
            .iter()
            .take(chat_agent::PERMISSION_MESSAGE_PREVIEW)
            .filter_map(|uid| db::messages::get_by_uid(conn, acct, f, *uid).ok().flatten())
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
    let line = user_line(&text, images.unwrap_or_default())?;
    let guard = CHAT.lock().await;
    let session = guard
        .as_ref()
        .ok_or_else(|| AppError::General("No chat is running — start a new one.".into()))?;
    session
        .tx
        .send(line)
        .map_err(|_| AppError::General("The chat has ended — start a new one.".into()))
}

/// One user message as a stream-json line, after the same checks wherever it
/// came from.
fn user_line(text: &str, images: Vec<String>) -> Result<String, AppError> {
    let text = text.trim();
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
    Ok(chat_agent::user_message_line(text, &images))
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
    let line = answer_line(&session.shared, &request_id, allow, remember)?;
    session
        .tx
        .send(line)
        .map_err(|_| AppError::General("The chat has ended.".into()))
}

/// The answer to one open permission question, consuming it.
fn answer_line(shared: &Shared, request_id: &str, allow: bool, remember: bool) -> Result<String, AppError> {
    let (tool, input) = shared
        .pending
        .lock()
        .ok()
        .and_then(|mut p| p.remove(request_id))
        .ok_or_else(|| AppError::NotFound("That request was already answered.".into()))?;
    Ok(if allow {
        // A CXTasks write asks every time, whatever the client sent.
        if remember && chat_agent::can_remember(&tool) {
            if let Ok(mut set) = shared.always_allow.lock() {
                set.insert(tool);
            }
        }
        chat_agent::allow_line(request_id, &input)
    } else {
        chat_agent::deny_line(
            request_id,
            "The user declined this in CXMail. Do not retry it unless they ask.",
        )
    })
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
///
/// With a Herdr host the transcript has to travel too: see `prepare_host_resume`.
#[tauri::command]
pub async fn chat_continue_in_terminal(app: AppHandle) -> Result<(), AppError> {
    let ghostty = claude_handoff::find_ghostty_app()
        .ok_or_else(|| AppError::General("Ghostty is not installed.".into()))?;
    // Before the stop, so an unreadable setting costs nothing.
    let herdr_host = claude_handoff::herdr_host(&app)?;
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
    let host = herdr_host.clone();
    let cli = tokio::task::spawn_blocking(move || claude_handoff::handoff_claude_cli(host.as_deref()))
        .await
        .map_err(|e| AppError::General(format!("spawn_blocking join error: {e}")))?
        .ok_or_else(|| AppError::General("Claude Code CLI not found.".into()))?;

    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::General(format!("failed to resolve app_data_dir: {e}")))?;
    let chat_home = app_dir.join("claude-chat");
    let script_path = if herdr_host.is_some() {
        let projects = dirs::home_dir()
            .ok_or_else(|| AppError::General("Could not find the home folder.".into()))?
            .join(".claude/projects");
        prepare_host_resume(
            &chat_home.join("handoffs").join(&session_id),
            &projects,
            &cli,
            &session.cwd,
            &session_id,
            &session.add_dirs,
        )?
    } else {
        let script_path = chat_home.join("resume.sh");
        let script = resume_script(&cli, &session.cwd, &session_id, &session.add_dirs);
        write_executable(&script_path, &script)?;
        script_path
    };

    claude_handoff::launch_in_ghostty(ghostty, session.cwd.clone(), script_path, None, herdr_host)
        .await
}

fn write_executable(path: &Path, contents: &str) -> Result<(), AppError> {
    std::fs::write(path, contents)?;
    let mut perms = std::fs::metadata(path)?.permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

/// Build what `claude --resume` needs on another Mac, in a folder of its own.
///
/// The conversation is a file under this Mac's `~/.claude/projects`, so resuming
/// by id on the host finds nothing unless the file goes with it. And the folder
/// is not optional: the helper copies everything beside the launch script, and
/// `claude-chat` itself holds `mcp.json`, whose servers can carry credentials.
///
/// The transcript is looked up by id across every project folder, which spares
/// this Mac from re-deriving Claude Code's folder naming. The host derives it,
/// because only the host knows its own real path (`host_resume_script`).
fn prepare_host_resume(
    folder: &Path,
    projects: &Path,
    cli: &Path,
    cwd: &Path,
    session_id: &str,
    add_dirs: &[PathBuf],
) -> Result<PathBuf, AppError> {
    let file_name = format!("{session_id}.jsonl");
    let transcript = std::fs::read_dir(projects)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path().join(&file_name))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            AppError::General(
                "This chat's transcript wasn't found, so it can't move to the Herdr host. \
                 Nothing was opened."
                    .into(),
            )
        })?;
    std::fs::create_dir_all(folder)?;
    let copied = folder.join(&file_name);
    std::fs::copy(transcript, &copied)?;
    let script_path = folder.join("resume.sh");
    write_executable(
        &script_path,
        &host_resume_script(cli, cwd, session_id, &copied, add_dirs),
    )?;
    Ok(script_path)
}

/// `resume_script`, after putting the transcript where the host's Claude Code
/// will look: `~/.claude/projects/<the cwd with every non-alphanumeric as ->`.
///
/// `pwd -P`, not this Mac's path. Claude Code names the folder from the real
/// cwd, and a home folder that is a link on the host resolves to something else
/// there, so only a script running on the host can spell it.
fn host_resume_script(
    cli: &Path,
    cwd: &Path,
    session_id: &str,
    transcript: &Path,
    add_dirs: &[PathBuf],
) -> String {
    let install = format!(
        "sessions=\"$HOME/.claude/projects/$(pwd -P | LC_ALL=C sed 's/[^A-Za-z0-9]/-/g')\"\n\
         mkdir -p \"$sessions\"\n\
         cp {} \"$sessions/\"\n",
        claude_handoff::shell_quote(&transcript.to_string_lossy()),
    );
    resume_script_after(&install, cli, cwd, session_id, add_dirs)
}

fn resume_script(cli: &Path, cwd: &Path, session_id: &str, add_dirs: &[PathBuf]) -> String {
    resume_script_after("", cli, cwd, session_id, add_dirs)
}

/// `--add-dir` last, as everywhere else (gotcha #49).
fn resume_script_after(
    before_exec: &str,
    cli: &Path,
    cwd: &Path,
    session_id: &str,
    add_dirs: &[PathBuf],
) -> String {
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
        "#!/bin/zsh -l\nset -e\ncd {}\n{before_exec}exec {} --resume {}{dirs}\n",
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

/// CXMail Remote's chat: the desktop chat's launch and permission gate,
/// spoken as JSON lines instead of Tauri events (`cxmail-mcp remote chat`).
///
/// stdout, one object per line: `{"started": {...}}` once, then
/// `{"event": <ChatEvent>}` (the panel's own payload), `{"error": "..."}` for
/// a command it refused. stdin, one object per line: `{"cmd":"send","text":…}`,
/// `{"cmd":"answer","request_id":…,"allow":bool,"remember":bool}`,
/// `{"cmd":"interrupt"}`, `{"cmd":"stop"}`. EOF on stdin stops the chat, so a
/// dead host never leaves a CLI running. Returns the exit code.
pub async fn run_remote_chat(db_path: &str, seed: Option<ChatSeed>, model: Option<String>) -> i32 {
    fn out(v: Value) {
        use std::io::Write;
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
    }
    let event = |e: &ChatEvent| out(serde_json::json!({ "event": e }));
    let fail = |m: String| {
        out(serde_json::json!({ "error": m }));
        1
    };

    let model = model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
    if let Some(m) = &model {
        if let Err(e) = chat_agent::validate_model(m) {
            return fail(e);
        }
    }
    let (cli, login_path) = match locate_cli().await {
        Ok(v) => v,
        Err(e) => return fail(e.to_string()),
    };
    let read_conn = || -> Result<rusqlite::Connection, AppError> {
        let c = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        c.busy_timeout(Duration::from_secs(5))?;
        Ok(c)
    };
    let inputs = match read_conn().and_then(|c| gather_launch_inputs(&c, seed.as_ref())) {
        Ok(v) => v,
        Err(e) => return fail(e.to_string()),
    };
    // Its own home, so the phone's sessions never mix with the panel's
    // "Continue in terminal" history.
    let chat_home = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.cxmail.app")
        .join("claude-chat-remote");
    let p = match assemble_launch(cli, login_path, inputs, seed.as_ref(), &chat_home, model.as_deref()) {
        Ok(p) => p,
        Err(e) => return fail(e.to_string()),
    };
    let mut child = match spawn_chat(&p) {
        Ok(c) => c,
        Err(e) => return fail(e.to_string()),
    };
    out(serde_json::json!({ "started": {
        "cwd": p.cwd.to_string_lossy(),
        "repo": p.seed_repo,
        "missing_repo_path": p.missing_repo_path,
        "readable_repos": p.repo_lines.iter().map(|r| r.path.clone()).collect::<Vec<_>>(),
        "mcp_servers": p.servers,
        "seed_subject": p.seed_subject,
    }}));

    let shared = Arc::new(Shared {
        session_id: StdMutex::new(None),
        always_allow: StdMutex::new(HashSet::new()),
        pending: StdMutex::new(HashMap::new()),
        handshake_ok: AtomicBool::new(false),
    });
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        return fail("could not attach to the Claude process".into());
    };
    tokio::spawn(write_loop(stdin, rx));
    let reader = {
        let tx = tx.clone();
        let shared = shared.clone();
        let db_path = db_path.to_string();
        tokio::spawn(async move {
            pump_stdout(
                stdout,
                tx,
                shared,
                |e| out(serde_json::json!({ "event": e })),
                |input| {
                    rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                        .ok()
                        .and_then(|c| permission_context_from(&c, input))
                },
                || {},
            )
            .await
        })
    };
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = BufReader::new(stderr).read_to_end(&mut buf).await;
        let text = String::from_utf8_lossy(&buf);
        text.chars().rev().take(2000).collect::<Vec<_>>().into_iter().rev().collect::<String>()
    });

    let _ = tx.send(chat_agent::initialize_line(INIT_REQUEST_ID));
    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<&'static str>();
    {
        // Same rule as the panel: no permission gate, no chat.
        let shared = shared.clone();
        let stop_tx = stop_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(HANDSHAKE_TIMEOUT).await;
            if !shared.handshake_ok.load(Ordering::SeqCst) {
                let _ = stop_tx.send("handshake");
            }
        });
    }
    {
        let shared = shared.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(cmd) = serde_json::from_str::<Value>(&line) else {
                    out(serde_json::json!({ "error": "not JSON" }));
                    continue;
                };
                let str_of = |k: &str| cmd.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let result = match str_of("cmd").as_str() {
                    "send" => {
                        let images = cmd
                            .get("images")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                            .unwrap_or_default();
                        user_line(&str_of("text"), images).map(|l| {
                            let _ = tx.send(l);
                        })
                    }
                    "answer" => answer_line(
                        &shared,
                        &str_of("request_id"),
                        cmd.get("allow").and_then(Value::as_bool).unwrap_or(false),
                        cmd.get("remember").and_then(Value::as_bool).unwrap_or(false),
                    )
                    .map(|l| {
                        let _ = tx.send(l);
                    }),
                    "interrupt" => {
                        let id = format!("cxmail-interrupt-{}", uuid::Uuid::new_v4());
                        let _ = tx.send(chat_agent::interrupt_line(&id));
                        Ok(())
                    }
                    "stop" => {
                        let _ = stop_tx.send("stop");
                        Ok(())
                    }
                    other => Err(AppError::General(format!("unknown command {other:?}"))),
                };
                if let Err(e) = result {
                    out(serde_json::json!({ "error": e.to_string() }));
                }
            }
            // The host is gone: never leave the CLI running unattended.
            let _ = stop_tx.send("eof");
        });
    }

    let status = tokio::select! {
        s = child.wait() => s.ok(),
        why = stop_rx.recv() => {
            if why == Some("handshake") {
                event(&ChatEvent::Notice {
                    text: "Claude Code did not answer the chat handshake, so it was stopped \
                           rather than run without CXMail's permission gate. Is the CLI up \
                           to date and signed in?"
                        .into(),
                });
            }
            let _ = child.kill().await;
            child.wait().await.ok()
        }
    };
    let _ = reader.await;
    let stderr_tail = stderr_task.await.unwrap_or_default();
    event(&ChatEvent::Exited {
        code: status.and_then(|s| s.code()),
        stderr_tail,
    });
    0
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

    /// A blank Herdr host resumes the way it always did: no copying, no folder.
    #[test]
    fn the_local_resume_script_is_unchanged() {
        assert_eq!(
            resume_script(Path::new("/opt/claude"), Path::new("/Users/x/chat"), "abc-1", &[]),
            "#!/bin/zsh -l\nset -e\ncd '/Users/x/chat'\nexec '/opt/claude' --resume 'abc-1'\n"
        );
    }

    /// The whole route for real, minus the copy between Macs: this Mac's
    /// transcript goes into a folder, and the script run "on the host" must land
    /// it where Claude Code looks and then resume it.
    ///
    /// The cwd is reached through a link on purpose. A home folder can be one on
    /// the host, and Claude Code names its folder from the real path, so a slug
    /// taken from the path as written would install the transcript somewhere
    /// `--resume` never looks, and fail with "no conversation found".
    #[test]
    fn a_host_resume_lands_the_transcript_where_claude_will_look() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("cxmail-resume-{}", uuid::Uuid::new_v4()));
        let id = "7415e5f1-37b5-4500-82c1-6e1493d7695c";

        // This Mac: some other session's folder, then this chat's.
        let projects = root.join("here/.claude/projects");
        std::fs::create_dir_all(projects.join("-some-other-folder")).unwrap();
        let session_dir = projects.join("-chat-home");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(session_dir.join(format!("{id}.jsonl")), "{\"the\":\"conversation\"}\n").unwrap();

        // The host: a real folder under a name with a dot and a space, the link
        // this Mac's path goes through, and a `claude` that reports its argv.
        let real_cwd = root.join("real/com.cxmail.app/claude chat");
        std::fs::create_dir_all(&real_cwd).unwrap();
        let linked_root = root.join("linked");
        std::os::unix::fs::symlink(root.join("real"), &linked_root).unwrap();
        let linked_cwd = linked_root.join("com.cxmail.app/claude chat");
        let home = root.join("host-home");
        let bin = root.join("bin");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        write_executable(&bin.join("claude"), "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();

        let folder = root.join("handoff");
        let script = prepare_host_resume(
            &folder,
            &projects,
            Path::new("claude"),
            &linked_cwd,
            id,
            &[PathBuf::from("/Users/x/Projects/it's a repo")],
        )
        .unwrap();

        // Only what the host needs travels; the helper copies this whole folder.
        let mut names: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec![format!("{id}.jsonl"), "resume.sh".to_string()]);

        let out = std::process::Command::new("/bin/zsh")
            .arg(&script)
            .env("HOME", &home)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).lines().collect::<Vec<_>>(),
            ["--resume", id, "--add-dir", "/Users/x/Projects/it's a repo"]
        );

        let slug: String = real_cwd
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let landed = home.join(".claude/projects").join(slug).join(format!("{id}.jsonl"));
        assert_eq!(
            std::fs::read_to_string(&landed).unwrap_or_default(),
            "{\"the\":\"conversation\"}\n",
            "not at {landed:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Nothing to resume on the host means nothing should open there.
    #[test]
    fn a_missing_transcript_stops_the_handoff() {
        let root = std::env::temp_dir().join(format!("cxmail-resume-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("projects/-a-folder")).unwrap();
        let err = prepare_host_resume(
            &root.join("handoff"),
            &root.join("projects"),
            Path::new("claude"),
            Path::new("/Users/x/chat"),
            "7415e5f1-37b5-4500-82c1-6e1493d7695c",
            &[],
        )
        .unwrap_err();
        assert!(err.to_string().contains("Nothing was opened"), "{err}");
        assert!(!root.join("handoff").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
