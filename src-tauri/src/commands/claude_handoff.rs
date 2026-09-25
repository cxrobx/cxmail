use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tauri::{AppHandle, Manager, State};

use crate::db;
use crate::db::claude_repos::ResolvedRepo;
use crate::email::{imap, parser};
use crate::error::AppError;
use crate::{AppState, LockExt};

/// What the handoff actually did, so the UI can say where it landed.
///
/// The mapping is invisible from the outside — the window that opens looks the
/// same wherever it `cd`ed — so a landing nobody can see is a landing nobody
/// can correct. `repo.source` names the row that decided it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClaudeHandoff {
    /// Where the session actually landed.
    pub working_dir: String,
    /// The mapping that chose it, when one applied and its directory was there.
    pub repo: Option<ResolvedRepo>,
    /// A mapping resolved, but its directory is gone (moved? unmounted?), so
    /// this session fell back to the scratch directory. Never a refusal —
    /// getting the mail in front of Claude matters more than the cwd.
    pub missing_repo_path: Option<String>,
}

#[tauri::command]
pub async fn open_email_in_claude(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    folder_name: String,
    uid: u32,
) -> Result<ClaudeHandoff, AppError> {
    let ghostty_app = find_ghostty_app().ok_or_else(|| {
        AppError::General(
            "Ghostty is not installed. Install it from https://ghostty.org/download and try again."
                .into(),
        )
    })?;
    let claude_cli = find_claude_cli().ok_or_else(|| {
        AppError::General(
            "Claude Code CLI (`claude`) was not found on PATH. Install it (e.g. \
             `npm install -g @anthropic-ai/claude-code`) and try again."
                .into(),
        )
    })?;

    let (row, account, attachments_meta, mapped_repo) = {
        let conn = state.db.safe_lock();
        let row = db::messages::get_by_uid(&conn, &account_id, &folder_name, uid)?
            .ok_or_else(|| {
                AppError::NotFound(format!("Message uid {} not found in {}", uid, folder_name))
            })?;
        let account = db::accounts::get_by_id(&conn, &account_id)?
            .ok_or_else(|| AppError::NotFound("Account not found".into()))?;
        let atts = db::messages::get_attachments(&conn, &account_id, &folder_name, uid)?;
        // Same lock scope as the rest of the read: this is four small indexed
        // lookups, and taking the mutex a second time during a sync is how
        // `fetchBody` ends up stuck on "Loading message…" (gotcha #11).
        let repo = db::claude_repos::resolve_for_message(&conn, &account_id, &folder_name, uid)?;
        (row, account, atts, repo)
    };

    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::General(format!("failed to resolve app_data_dir: {e}")))?;
    let scratch_id = format!(
        "{}-{}-{}",
        sanitize_id(&account_id),
        sanitize_id(&folder_name),
        uid
    );
    let scratch = app_dir.join("claude-sessions").join(&scratch_id);
    fs::create_dir_all(&scratch)?;

    let mut saved_attachments: Vec<(String, u64)> = Vec::new();
    if row.has_attachments && !attachments_meta.is_empty() {
        let att_dir = scratch.join("attachments");
        if let Err(e) = fs::create_dir_all(&att_dir) {
            log::warn!("claude_handoff: could not create attachments dir: {e}");
        } else {
            match fetch_all_attachments(
                &account,
                &folder_name,
                uid,
                attachments_meta.len(),
            )
            .await
            {
                Ok(items) => {
                    for (filename, bytes) in items {
                        let safe = sanitize_filename(&filename);
                        let out = att_dir.join(&safe);
                        if let Err(e) = fs::write(&out, &bytes) {
                            log::warn!("claude_handoff: could not write attachment {safe}: {e}");
                        } else {
                            saved_attachments.push((safe, bytes.len() as u64));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("claude_handoff: attachment fetch failed: {e}");
                }
            }
        }
    }

    let email_json = serde_json::json!({
        "account_id": account_id,
        "folder": folder_name,
        "uid": uid,
        "subject": row.subject,
        "from_name": row.from_name,
        "from_email": row.from_email,
        "date": row.date,
        "has_attachments": row.has_attachments,
        "attachments": saved_attachments
            .iter()
            .map(|(f, s)| serde_json::json!({ "filename": f, "size_bytes": s }))
            .collect::<Vec<_>>(),
    });
    fs::write(
        scratch.join("email.json"),
        serde_json::to_string_pretty(&email_json).unwrap_or_default(),
    )?;

    // Where this session lands. A mapping whose directory is gone — repo moved,
    // volume unmounted — falls back to the scratch dir and SAYS so, in the
    // prompt and in the return value. Refusing to open would be the wrong
    // trade: the point of the handoff is getting the mail in front of Claude.
    let (repo, missing_repo_path) = split_on_existence(mapped_repo);
    let landing_dir = repo
        .as_ref()
        .map(|r| PathBuf::from(&r.repo_path))
        .unwrap_or_else(|| scratch.clone());

    let prompt = build_prompt(
        &account_id,
        &folder_name,
        uid,
        &row,
        &saved_attachments,
        &scratch,
        repo.as_ref(),
        missing_repo_path.as_deref(),
    );
    let prompt_path = scratch.join("prompt.txt");
    fs::write(&prompt_path, &prompt)?;

    let launch_path = scratch.join("launch.sh");
    fs::write(
        &launch_path,
        build_launch_script(&claude_cli, &landing_dir, &scratch, &prompt_path),
    )?;
    let mut perms = fs::metadata(&launch_path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&launch_path, perms)?;

    launch_in_ghostty(ghostty_app, landing_dir.clone(), launch_path).await?;

    log::info!(
        "claude_handoff: launched Ghostty session in {:?} ({})",
        landing_dir,
        match (&repo, &missing_repo_path) {
            (Some(r), _) => format!("{} {}", r.scope.as_str(), r.source),
            (None, Some(missing)) => format!("mapped repo missing: {missing}"),
            (None, None) => "no repo mapped".to_string(),
        }
    );
    Ok(ClaudeHandoff {
        working_dir: landing_dir.to_string_lossy().to_string(),
        repo,
        missing_repo_path,
    })
}

/// Open a Ghostty window in `working_dir` running `launch_path`.
///
/// Shared by the email handoff and the chat panel's "Continue in terminal",
/// so both get the running-instance route and its fallback (gotchas #24, #35).
pub(crate) async fn launch_in_ghostty(
    ghostty_app: PathBuf,
    working_dir: PathBuf,
    launch_path: PathBuf,
) -> Result<(), AppError> {
    let working_dir_arg = format!("--working-directory={}", working_dir.display());
    let ghostty_app_clone = ghostty_app;
    let launch_path_for_err = launch_path;
    let scratch_for_spawn = working_dir;
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        // Preferred: ask the Ghostty the user ALREADY has for a new window,
        // over AppleScript. Without it every "Open in Claude" leaves a second
        // whole Ghostty application behind — its own Dock icon, its own
        // Cmd-Tab entry. See gotcha #35, and note it corrects #24's conclusion
        // that AppleScript could not carry the working dir and the script.
        match open_in_running_ghostty(&scratch_for_spawn, &launch_path_for_err) {
            Ok(()) => return Ok(()),
            // Fail OPEN, always: Ghostty quit, `macos-applescript = false`,
            // declined automation consent, or a pre-1.3 install. None of those
            // is a reason to refuse the handoff, and each leaves the fallback
            // below working exactly as it did before.
            Err(reason) => log::info!(
                "claude_handoff: AppleScript route unavailable ({reason}); \
                 falling back to a new Ghostty instance"
            ),
        }

        // Fallback: `open -na` forces a NEW Ghostty instance (required so our
        // `-e` command is honored even when Ghostty is already running). On
        // macOS a fresh instance also RESTORES the previous session's saved
        // windows, so the user got the restored window(s) *plus* our command
        // window — i.e. duplicate tabs (observed: restored-count + 1). Passing
        // `--window-save-state=never` as a per-launch CLI override tells this
        // instance not to restore (and not to save) state, so exactly one
        // window opens. This does not touch the user's config or their primary
        // Ghostty instance. See gotchas #24 / ghostty-org/ghostty#9612.
        Command::new("/usr/bin/open")
            .arg("-na")
            .arg(&ghostty_app_clone)
            .arg("--args")
            .arg("--window-save-state=never")
            .arg(&working_dir_arg)
            .arg("-e")
            .arg(&launch_path_for_err)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                AppError::General(format!(
                    "Failed to launch Ghostty ({}): {e}",
                    launch_path_for_err.display()
                ))
            })?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::General(format!("spawn_blocking join error: {e}")))?
}

/// Split a resolved mapping on whether its directory is actually there.
///
/// Both prompt builders need the same answer and must not disagree: a `cd` into
/// a directory that no longer exists is worse than no `cd` at all — it turns a
/// mapping that merely went stale into an error in the user's session. Checked
/// at use rather than stored, because a repo on an unmounted volume comes back
/// without anybody editing the row.
pub(crate) fn split_on_existence(mapped: Option<ResolvedRepo>) -> (Option<ResolvedRepo>, Option<String>) {
    match mapped {
        Some(r) if Path::new(&r.repo_path).is_dir() => (Some(r), None),
        Some(r) => {
            log::warn!(
                "claude_handoff: mapped repo {:?} (from {} {}) is not a directory",
                r.repo_path,
                r.scope.as_str(),
                r.source
            );
            (None, Some(r.repo_path))
        }
        None => (None, None),
    }
}

/// The script Ghostty runs.
///
/// Two things here are load-bearing:
///
/// 1. **The prompt path is absolute.** The cwd is now the repo, not the
///    directory `prompt.txt` lives in, so the old relative `$(cat prompt.txt)`
///    would read nothing and start a session with an empty prompt.
/// 2. **`--add-dir` goes LAST, after the positional prompt.** It is declared
///    variadic (`--add-dir <directories...>`), so `--add-dir <scratch> "$(cat
///    …)"` hands the prompt to it as a second directory. Trailing it is
///    unambiguous under any parser — do not "tidy" the flags to the front.
///
/// `--add-dir` is omitted entirely when the session lands in the scratch dir,
/// which is both unnecessary and the pre-mapping behaviour, byte for byte.
fn build_launch_script(claude: &Path, landing: &Path, scratch: &Path, prompt: &Path) -> String {
    let add_dir = if landing == scratch {
        String::new()
    } else {
        format!(" --add-dir {}", shell_quote(&scratch.to_string_lossy()))
    };
    format!(
        "#!/bin/zsh -l\nset -e\ncd {dir}\nexec {claude} \"$(cat {prompt})\"{add_dir}\n",
        dir = shell_quote(&landing.to_string_lossy()),
        claude = shell_quote(&claude.to_string_lossy()),
        prompt = shell_quote(&prompt.to_string_lossy()),
    )
}

/// Build the Claude seed prompt without launching anything. Used by the
/// "Copy Claude Prompt" context-menu action so the text can be pasted
/// into a Claude session the user opens themselves.
#[tauri::command]
pub async fn get_claude_prompt(
    state: State<'_, AppState>,
    account_id: String,
    folder_name: String,
    uid: u32,
) -> Result<String, AppError> {
    let (row, attachments_meta, repo) = {
        let conn = state.db.safe_lock();
        let row = db::messages::get_by_uid(&conn, &account_id, &folder_name, uid)?
            .ok_or_else(|| {
                AppError::NotFound(format!("Message uid {} not found in {}", uid, folder_name))
            })?;
        let atts = db::messages::get_attachments(&conn, &account_id, &folder_name, uid)?;
        let repo = db::claude_repos::resolve_for_message(&conn, &account_id, &folder_name, uid)?;
        (row, atts, repo)
    };

    let (repo, missing_repo_path) = split_on_existence(repo);
    Ok(build_copy_prompt(
        &account_id,
        &folder_name,
        uid,
        &row,
        &attachments_meta,
        repo.as_ref(),
        missing_repo_path.as_deref(),
    ))
}

/// The `command` value for a Ghostty surface configuration: the launch script,
/// **shell-quoted**.
///
/// Ghostty does not exec this string — it runs it through a login shell,
/// `login -flp <user> /bin/bash --noprofile --norc -c exec -l <command>`, so
/// **bash word-splits it**. Every handoff hits this, because the scratch dir
/// lives under `~/Library/Application Support/…` and that space is not
/// optional: unquoted, bash tries to exec `~/Library/Application` and Ghostty
/// paints a red "failed to launch the requested command".
///
/// Note the asymmetry with `initial working directory`, which Ghostty applies
/// directly and which must therefore be passed RAW — quoting that one puts
/// literal `'` characters into the path and breaks a `cd` that works today. One
/// field reaches a shell and the other doesn't; they are not interchangeable
/// however similar they look sitting next to each other in the record.
fn ghostty_command(launch_path: &std::path::Path) -> String {
    shell_quote(&launch_path.to_string_lossy())
}

/// Ask an ALREADY-RUNNING Ghostty for a new window running `launch_path`.
///
/// The good path: a window inside the instance the user already has, rather
/// than a whole second Ghostty in the Dock. Returns `Err(reason)` whenever the
/// route isn't available so the caller can fall back to `open -na`.
///
/// Ghostty 1.3 exposes an AppleScript dictionary (`Ghostty.sdef`, gated on the
/// config key `macos-applescript`, default true). It is `new window` and NOT
/// `new tab` — `new tab` answers `errAEEventNotHandled` in 1.3.1 even with no
/// arguments, so there is nothing to pass differently.
fn open_in_running_ghostty(
    working_dir: &std::path::Path,
    launch_path: &std::path::Path,
) -> Result<(), String> {
    let command = ghostty_command(launch_path);
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg("on run argv")
        // `is running` is answered by Launch Services rather than by an Apple
        // Event, so this asks WITHOUT launching Ghostty as a side effect. That
        // is the whole guard: launching it cold restores its saved windows,
        // which is exactly the mess gotcha #24 needed
        // `--window-save-state=never` to avoid. A cold start therefore belongs
        // on the fallback, which handles it and leaves one clean instance for
        // every later handoff to reuse through this path.
        .arg("-e")
        .arg(
            r#"if application id "com.mitchellh.ghostty" is not running then error "Ghostty isn't running" number -128"#,
        )
        .arg("-e")
        .arg(r#"tell application id "com.mitchellh.ghostty""#)
        // `wait after command:false` deliberately MATCHES the fallback: the
        // window closes when the session ends, exactly as it does today.
        .arg("-e")
        .arg(
            "new window with configuration {initial working directory:(item 1 of argv), \
             command:(item 2 of argv), wait after command:false}",
        )
        .arg("-e")
        .arg("end tell")
        .arg("-e")
        .arg("end run")
        // The paths travel as ARGUMENTS, never interpolated into the script
        // source: AppleScript string literals escape nothing the way a shell
        // does, so a `format!`-ed path holding a quote or backslash is a syntax
        // error inside osascript. Both are absolute, so neither can be mistaken
        // for an osascript option. Note the two are quoted DIFFERENTLY and must
        // stay that way — see `ghostty_command`.
        .arg(working_dir)
        .arg(&command)
        .output()
        .map_err(|e| format!("could not run osascript: {e}"))?;

    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        format!("osascript exited {}", output.status)
    } else {
        stderr
    })
}

pub(crate) fn find_ghostty_app() -> Option<PathBuf> {
    let p = PathBuf::from("/Applications/Ghostty.app");
    if p.exists() {
        return Some(p);
    }
    if let Some(home) = dirs::home_dir() {
        let user_p = home.join("Applications/Ghostty.app");
        if user_p.exists() {
            return Some(user_p);
        }
    }
    None
}

pub(crate) fn find_claude_cli() -> Option<PathBuf> {
    // GUI apps on macOS don't inherit the shell's PATH, so `/usr/bin/which`
    // runs against a bare-bones PATH and usually misses user installs.
    // Ask a login shell to resolve it instead — that picks up whatever the
    // user has configured in .zshrc / .zprofile / .bash_profile.
    if let Ok(output) = Command::new("/bin/zsh")
        .arg("-lc")
        .arg("command -v claude")
        .output()
    {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                let p = PathBuf::from(&s);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }

    // Fallback: probe common install locations directly.
    for candidate in &[
        "/opt/homebrew/bin/claude",
        "/usr/local/bin/claude",
    ] {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(home) = dirs::home_dir() {
        for rel in &[
            ".local/bin/claude",
            ".claude/local/claude",
            ".volta/bin/claude",
            ".bun/bin/claude",
        ] {
            let p = home.join(rel);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn sanitize_id(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    out
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == '\0' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.').to_string();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed
    }
}

/// Single-quote for a POSIX shell. Safe for any bytes: wraps in '...' and
/// escapes embedded single quotes as '\''.
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

async fn fetch_all_attachments(
    account: &db::accounts::Account,
    folder: &str,
    uid: u32,
    count: usize,
) -> Result<Vec<(String, Vec<u8>)>, AppError> {
    let mut session = imap::connect_for_account(account).await?;
    let _ = imap::select_folder(&mut session, folder).await?;
    let raw = imap::fetch_body(&mut session, uid).await?;
    let _ = imap::disconnect(session, &account.email).await;

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        if let Some((name, data)) = parser::extract_attachment(&raw, i) {
            out.push((name, data));
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn build_prompt(
    account_id: &str,
    folder: &str,
    uid: u32,
    row: &db::messages::MessageRow,
    saved_attachments: &[(String, u64)],
    scratch: &Path,
    repo: Option<&ResolvedRepo>,
    missing_repo_path: Option<&str>,
) -> String {
    let subject = row.subject.as_deref().unwrap_or("(no subject)");
    let from_name = row.from_name.as_deref().unwrap_or("");
    let from_email = row.from_email.as_deref().unwrap_or("");

    let mut s = String::new();
    s.push_str(
        "I'm in a CXMail-spawned Claude session scoped to one specific email. \
Help me think through it — summarize, draft replies, whatever I ask.\n\n",
    );
    if let Some(repo) = repo {
        // Name the row that chose this directory. When mail lands somewhere
        // surprising, "which rule did that?" is the only question worth
        // answering, and it should not need a log.
        s.push_str(&format!(
            "You are in {repo_path} — the repo CXMail maps this email to \
({scope} {source}). Its CLAUDE.md and code apply.\n\n",
            repo_path = repo.repo_path,
            scope = repo.scope.as_str(),
            source = repo.source,
        ));
    }
    if let Some(missing) = missing_repo_path {
        s.push_str(&format!(
            "Note: this email is mapped to {missing}, but that directory isn't \
there right now (moved, or on an unmounted volume), so this session started in \
its scratch directory instead.\n\n"
        ));
    }
    s.push_str("Email reference:\n");
    s.push_str(&format!("  account_id: {account_id}\n"));
    s.push_str(&format!("  folder:     {folder}\n"));
    s.push_str(&format!("  uid:        {uid}\n"));
    s.push_str(&format!("  subject:    {subject}\n"));
    s.push_str(&format!("  from:       {from_name} <{from_email}>\n\n"));
    s.push_str(
        "Use the `cxmail` MCP server (tool: read_email) with those account_id / folder / uid \
values to load the full body before responding.\n",
    );
    // Absolute, and reachable: once the cwd is the repo, "this directory" means
    // the repo, and the scratch dir is only readable because the launch script
    // passes it to --add-dir.
    s.push_str(&format!(
        "{email_json} has the same identifiers if you need to re-read them.\n",
        email_json = scratch.join("email.json").display(),
    ));
    if saved_attachments.is_empty() {
        if row.has_attachments {
            s.push_str(
                "\nThis email has attachments, but pre-fetching them failed. If I ask about them, \
tell me so I can retry or download from CXMail directly.\n",
            );
        }
    } else {
        s.push_str(&format!(
            "\nAttachments are already downloaded to {}/:\n",
            scratch.join("attachments").display()
        ));
        for (name, size) in saved_attachments {
            s.push_str(&format!("  - {name} ({size} bytes)\n"));
        }
        s.push_str("Read them with your Read tool if I ask about them.\n");
    }
    s.push_str(
        "\nPlease do NOT take write actions (send_email, delete, move, flag, archive, \
compose_draft) unless I explicitly ask for them.\n",
    );
    s
}

fn build_copy_prompt(
    account_id: &str,
    folder: &str,
    uid: u32,
    row: &db::messages::MessageRow,
    attachments: &[crate::email::parser::AttachmentMeta],
    repo: Option<&ResolvedRepo>,
    missing_repo_path: Option<&str>,
) -> String {
    let subject = row.subject.as_deref().unwrap_or("(no subject)");
    let from_name = row.from_name.as_deref().unwrap_or("");
    let from_email = row.from_email.as_deref().unwrap_or("");

    let mut s = String::new();
    s.push_str(
        "I want help thinking through a specific email from my CXMail inbox — \
summarize, draft replies, whatever I ask.\n\n",
    );
    s.push_str("Email reference:\n");
    s.push_str(&format!("  account_id: {account_id}\n"));
    s.push_str(&format!("  folder:     {folder}\n"));
    s.push_str(&format!("  uid:        {uid}\n"));
    s.push_str(&format!("  subject:    {subject}\n"));
    s.push_str(&format!("  from:       {from_name} <{from_email}>\n"));
    s.push_str(&format!("  date:       {}\n", row.date));
    // This prompt is pasted into a session someone else started, in whatever
    // directory that session happens to be in — so unlike the launched handoff
    // (which is already there), the repo has to arrive as an instruction. The
    // path is shell-quoted: `Application Support` is not the only directory on
    // this machine with a space in it, and an unquoted cd fails in a way that
    // reads as a bad mapping rather than a bad quote.
    if let Some(repo) = repo {
        s.push_str(&format!(
            "  repo:       {repo_path}   (mapped from {scope} {source})\n\n",
            repo_path = repo.repo_path,
            scope = repo.scope.as_str(),
            source = repo.source,
        ));
        s.push_str(&format!(
            "That repo is the project this email is about. Unless you are already working \
there, start with:\n\n  cd {}\n\n",
            shell_quote(&repo.repo_path)
        ));
    } else if let Some(missing) = missing_repo_path {
        // Deliberately no `cd`. Naming it is still worth doing — it tells the
        // user which mapping to fix — but sending a session into a directory
        // that isn't there just produces a confusing error.
        s.push_str(&format!(
            "  repo:       {missing} — mapped, but that directory isn't there right now\n\n"
        ));
    } else {
        s.push('\n');
    }
    s.push_str(
        "If you have the `cxmail` MCP server available, call its `read_email` tool with \
the account_id / folder / uid above to load the full body. Otherwise, ask me to paste it.\n",
    );
    if !attachments.is_empty() {
        s.push_str("\nThis email has attachments:\n");
        for a in attachments {
            let name = a.filename.as_deref().unwrap_or("(unnamed)");
            s.push_str(&format!(
                "  - {} ({}, {} bytes)\n",
                name, a.content_type, a.size_bytes
            ));
        }
        s.push_str(
            "Ask me to download any of them from CXMail (or via the cxmail MCP) if you need the contents.\n",
        );
    }
    s.push_str(
        "\nPlease do NOT take write actions (send, delete, move, flag, archive, draft) \
unless I explicitly ask for them.\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Ghostty's `command` reaches a shell, so it is word-SPLIT — and the
    /// scratch dir always sits under `~/Library/Application Support/…`, so the
    /// space is guaranteed rather than exotic. Unquoted, bash tries to exec
    /// `~/Library/Application` and the handoff dies in a window reading
    /// "Ghostty failed to launch the requested command".
    ///
    /// This shipped broken in cxtasks first, because the probe used to develop
    /// it lived in a path with no spaces — so it passed repeatedly against a
    /// string that could never work in production. Hence the real
    /// `Application Support` shape here rather than a convenient one.
    ///
    /// Asserting "one word out, equal to the path in" rather than on the quoted
    /// string's shape: correct quoting produces `'…'` wrappers that any
    /// substring assertion would report as a failure on working code.
    #[test]
    fn the_ghostty_command_is_one_word_after_the_shell_splits_it() {
        for path in [
            // The shape that actually broke.
            "/Users/x/Library/Application Support/com.cxmail.app/claude-sessions/a-b-1/launch.sh",
            "/tmp/no-spaces/launch.sh",
            "/tmp/it's got a quote/launch.sh",
        ] {
            let command = ghostty_command(Path::new(path));
            // Exactly how Ghostty runs it: through bash, which splits.
            let out = Command::new("/bin/bash")
                .arg("--noprofile")
                .arg("--norc")
                .arg("-c")
                .arg(format!("printf '%s\\n' {command}"))
                .output()
                .expect("run bash");
            assert!(out.status.success(), "bash rejected {command}");
            let words: Vec<_> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_string)
                .collect();
            assert_eq!(
                words,
                vec![path.to_string()],
                "{path:?} did not survive the shell as a single argument"
            );
        }
    }

    fn a_row() -> db::messages::MessageRow {
        db::messages::MessageRow {
            uid: 7,
            account_id: Some("acct".into()),
            folder_name: Some("INBOX".into()),
            subject: Some("Re: scope".into()),
            from_name: Some("Dana".into()),
            from_email: Some("dana@northwind.example".into()),
            date: "2026-08-20T10:00:00Z".into(),
            snippet: None,
            is_read: true,
            is_flagged: false,
            has_attachments: false,
            size_bytes: 0,
            category: None,
            is_muted: false,
            is_pinned: false,
            thread_count: 1,
            thread_draft_count: 0,
            thread_root_id: None,
            thread_has_unread: false,
        }
    }

    fn resolved(path: &str) -> ResolvedRepo {
        ResolvedRepo {
            repo_path: path.into(),
            scope: crate::db::claude_repos::RepoScope::Contact,
            source: "northwind.example".into(),
        }
    }

    /// The copied prompt is pasted into a session that is already somewhere
    /// else, so — unlike the launched handoff, which is dropped into the repo —
    /// the repo has to arrive as an instruction it can act on.
    #[test]
    fn the_copied_prompt_carries_a_cd_into_the_mapped_repo() {
        let prompt = build_copy_prompt(
            "acct",
            "INBOX",
            7,
            &a_row(),
            &[],
            Some(&resolved("/Users/x/Projects/cxventures")),
            None,
        );
        assert!(
            prompt.contains("cd '/Users/x/Projects/cxventures'"),
            "no usable cd line:\n{prompt}"
        );
        assert!(
            prompt.contains("repo:       /Users/x/Projects/cxventures"),
            "the repo should also appear in the reference block:\n{prompt}"
        );
        assert!(prompt.contains("mapped from contact northwind.example"), "{prompt}");
    }

    /// A space in a repo path must not produce a `cd` that breaks on paste.
    #[test]
    fn the_copied_cd_is_shell_quoted() {
        let prompt = build_copy_prompt(
            "acct",
            "INBOX",
            7,
            &a_row(),
            &[],
            Some(&resolved("/Users/x/Dropbox (Work)/client repo")),
            None,
        );
        let cd_line = prompt
            .lines()
            .find(|l| l.trim_start().starts_with("cd "))
            .expect("a cd line");
        let out = Command::new("/bin/bash")
            .arg("--noprofile")
            .arg("--norc")
            .arg("-c")
            .arg(format!("printf '%s\\n' {}", cd_line.trim().trim_start_matches("cd ")))
            .output()
            .expect("run bash");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim_end(),
            "/Users/x/Dropbox (Work)/client repo",
            "the pasted cd would not survive a shell: {cd_line}"
        );
    }

    /// A mapping whose directory is gone must NOT produce a `cd` — that turns a
    /// stale mapping into an error in the user's own session. Naming it is
    /// still right: it says which mapping to fix.
    #[test]
    fn a_missing_repo_is_named_but_never_cd_into() {
        let prompt = build_copy_prompt(
            "acct",
            "INBOX",
            7,
            &a_row(),
            &[],
            None,
            Some("/Users/x/clients/voltworks-moved"),
        );
        assert!(!prompt.contains("cd "), "must not cd into a missing dir:\n{prompt}");
        assert!(prompt.contains("/Users/x/clients/voltworks-moved"), "{prompt}");
        assert!(prompt.contains("isn't there right now"), "{prompt}");
    }

    /// With nothing mapped the prompt is what it always was — no repo line, no
    /// cd, no mention of a mapping the user has not made.
    #[test]
    fn an_unmapped_email_copies_the_prompt_it_always_did() {
        let prompt = build_copy_prompt("acct", "INBOX", 7, &a_row(), &[], None, None);
        assert!(!prompt.contains("cd "), "{prompt}");
        assert!(!prompt.contains("repo:"), "{prompt}");
        assert!(prompt.contains("Email reference:"), "{prompt}");
    }

    /// The whole point of the mapping: land in the repo, and still be able to
    /// read the email material that lives somewhere else.
    ///
    /// **`--add-dir` must come after the positional prompt.** It is declared
    /// `--add-dir <directories...>` — variadic — so with the flag first, the
    /// prompt becomes its second directory and the session starts with no
    /// prompt at all. That failure is quiet (a normal-looking Claude window,
    /// just empty), which is exactly the kind that survives a manual smoke test.
    #[test]
    fn a_mapped_repo_becomes_the_cwd_and_the_scratch_dir_stays_readable() {
        let script = build_launch_script(
            Path::new("/opt/homebrew/bin/claude"),
            Path::new("/Users/x/Projects/cxventures"),
            Path::new("/Users/x/Library/Application Support/com.cxmail.app/claude-sessions/a-b-1"),
            Path::new(
                "/Users/x/Library/Application Support/com.cxmail.app/claude-sessions/a-b-1/prompt.txt",
            ),
        );
        assert!(
            script.contains("cd '/Users/x/Projects/cxventures'"),
            "should cd to the repo, not the scratch dir: {script}"
        );
        assert!(
            script.contains("--add-dir '/Users/x/Library/Application Support/com.cxmail.app/claude-sessions/a-b-1'"),
            "the scratch dir must be readable from the repo: {script}"
        );
        let exec_line = script.lines().find(|l| l.starts_with("exec ")).unwrap();
        let prompt_at = exec_line.find("$(cat").unwrap();
        let add_dir_at = exec_line.find("--add-dir").unwrap();
        assert!(
            prompt_at < add_dir_at,
            "--add-dir is variadic and would swallow the prompt: {exec_line}"
        );
    }

    /// The prompt file is read by absolute path, because the cwd is no longer
    /// the directory it sits in. A relative `prompt.txt` would silently expand
    /// to nothing under `$(cat …)` and open an empty session.
    #[test]
    fn the_prompt_is_read_by_absolute_path() {
        let script = build_launch_script(
            Path::new("/opt/homebrew/bin/claude"),
            Path::new("/Users/x/Projects/cxventures"),
            Path::new("/Users/x/scratch"),
            Path::new("/Users/x/scratch/prompt.txt"),
        );
        assert!(script.contains("\"$(cat '/Users/x/scratch/prompt.txt')\""), "{script}");
        assert!(
            !script.contains("$(cat prompt.txt)"),
            "a relative prompt path cannot survive the cd: {script}"
        );
    }

    /// With no mapping the script must be what it always was — same cd, and no
    /// `--add-dir` for a directory we are already standing in.
    #[test]
    fn with_no_repo_mapped_nothing_is_added_to_the_old_behaviour() {
        let scratch = Path::new("/Users/x/scratch");
        let script = build_launch_script(
            Path::new("/opt/homebrew/bin/claude"),
            scratch,
            scratch,
            &scratch.join("prompt.txt"),
        );
        assert!(script.contains("cd '/Users/x/scratch'"), "{script}");
        assert!(!script.contains("--add-dir"), "{script}");
    }

    /// Every path in the script is shell-quoted, and the ones that matter all
    /// contain a space in production (`Application Support`). Asserting the
    /// script survives `bash -c` as the right argv is stronger than any
    /// substring check on the quoting itself.
    #[test]
    fn every_path_in_the_script_survives_the_shell_intact() {
        let script = build_launch_script(
            Path::new("/opt/homebrew/bin/claude"),
            Path::new("/Users/x/Projects/it's a repo"),
            Path::new("/Users/x/Library/Application Support/com.cxmail.app/s"),
            Path::new("/Users/x/Library/Application Support/com.cxmail.app/s/prompt.txt"),
        );
        let exec_line = script.lines().find(|l| l.starts_with("exec ")).unwrap();
        // Replace the `exec` and the command substitution so the probe neither
        // execs claude nor needs the prompt file to exist.
        let probe = exec_line
            .replacen("exec ", "printf '%s\\n' ", 1)
            .replace("\"$(cat", "\"$(echo")
            .replace(")\"", ")\"");
        let out = Command::new("/bin/bash")
            .arg("--noprofile")
            .arg("--norc")
            .arg("-c")
            .arg(&probe)
            .output()
            .expect("run bash");
        assert!(out.status.success(), "bash rejected: {probe}");
        let words: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(
            words,
            vec![
                "/opt/homebrew/bin/claude".to_string(),
                "/Users/x/Library/Application Support/com.cxmail.app/s/prompt.txt".to_string(),
                "--add-dir".to_string(),
                "/Users/x/Library/Application Support/com.cxmail.app/s".to_string(),
            ],
            "a path was split or mangled by the shell"
        );
        assert!(
            script.contains("cd '/Users/x/Projects/it'\\''s a repo'"),
            "an apostrophe in a repo path must survive quoting: {script}"
        );
    }

    /// The other half: paths handed to `osascript` as ARGUMENTS arrive
    /// byte-identical, which is what lets `open_in_running_ghostty` skip
    /// AppleScript's own (entirely different) escaping rules. The guard against
    /// "simplifying" it into a `format!` on the script source. Deliberately
    /// inert — it echoes an argument and talks to no application, so it opens
    /// no windows and needs no automation consent.
    #[test]
    fn osascript_arguments_arrive_verbatim_however_they_are_quoted() {
        for evil in [
            "/tmp/a\"b",
            "/tmp/a\\b",
            "/tmp/a'b",
            "/tmp/a b",
            "/tmp/say \"hi\" & quit",
        ] {
            let out = Command::new("/usr/bin/osascript")
                .arg("-e")
                .arg("on run argv")
                .arg("-e")
                .arg("return item 1 of argv")
                .arg("-e")
                .arg("end run")
                .arg(evil)
                .output()
                .expect("run osascript");
            assert!(out.status.success(), "osascript rejected {evil:?}");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim_end_matches('\n'),
                evil,
                "osascript altered {evil:?} — it is not being passed as an argument"
            );
        }
    }
}
