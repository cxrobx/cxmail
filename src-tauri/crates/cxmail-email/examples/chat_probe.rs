//! Drive the in-app chat's exact `claude -p` launch headless, and print what
//! the panel would receive.
//!
//! `cargo run -p cxmail-email --example chat_probe -- <copy.db> "<message>" [--allow] [--model haiku]`
//!
//! The argv, tool tiers, MCP config and standing instructions come from
//! `email::chat_agent` — the same functions `commands::chat` calls — so this
//! answers what no unit test can: does the real CLI, with the real `cxmail`
//! MCP, do what the tiers say? Permission questions are DENIED unless
//! `--allow` is passed, so a probe can never mutate mail by accident.
//!
//! `<copy.db>` is read only for the repo mappings; point it at a `.backup`
//! copy (gotcha #12). The MCP the CLI spawns reads the live database, exactly
//! as it does for the app.

use cxmail_db::db::claude_repos;
use cxmail_email::email::chat_agent::{self, ChatEvent, Inbound};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let db = args.first().expect("usage: chat_probe <copy.db> \"<message>\" [--allow] [--model m]");
    let message = args.get(1).expect("a message to send");
    let allow = args.iter().any(|a| a == "--allow");
    let model = args
        .iter()
        .position(|a| a == "--model")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let conn = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open db");
    let mut repos: Vec<chat_agent::RepoLine> = Vec::new();
    for m in claude_repos::list(&conn).expect("list repos") {
        if !Path::new(&m.repo_path).is_dir() {
            continue;
        }
        let label = claude_repos::describe(&conn, &m);
        match repos.iter_mut().find(|r| r.path == m.repo_path) {
            Some(r) => r.label = format!("{}, {label}", r.label),
            None => repos.push(chat_agent::RepoLine { label, path: m.repo_path }),
        }
    }

    let home = std::env::temp_dir().join("cxmail-chat-probe");
    std::fs::create_dir_all(&home).expect("probe dir");
    let claude_json: Option<serde_json::Value> = std::env::var("HOME")
        .ok()
        .and_then(|h| std::fs::read_to_string(Path::new(&h).join(".claude.json")).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let (config, servers) = chat_agent::build_mcp_config(claude_json.as_ref(), None);
    let mcp_path = home.join("mcp.json");
    std::fs::write(&mcp_path, config.to_string()).expect("write mcp.json");
    let prompt = chat_agent::build_system_prompt(&repos, None, None, servers.iter().any(|s| s == "vault"));
    let add_dirs: Vec<PathBuf> = repos.iter().map(|r| PathBuf::from(&r.path)).collect();
    let argv = chat_agent::build_args(&chat_agent::ChatLaunch {
        mcp_config: &mcp_path,
        servers: &servers,
        system_prompt: &prompt,
        model: model.as_deref(),
        add_dirs: &add_dirs,
    });

    let cli = std::env::var("CLAUDE_BIN").unwrap_or_else(|_| {
        let out = Command::new("/bin/zsh").args(["-lc", "whence -p claude"]).output().expect("zsh");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    });
    eprintln!("servers {servers:?}, {} readable repos, cli {cli}", repos.len());

    let mut child = Command::new(&cli)
        .args(&argv)
        .current_dir(&home)
        .env("CLAUDE_HOOK_SOUND", "0")
        .env("CLAUDE_HOOK_BANNER", "0")
        .env("CXMAIL_CHAT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn claude");
    // Dropping stdin after the turn is the EOF that ends the CLI.
    let mut stdin = child.stdin.take();
    let send = |stdin: &mut Option<std::process::ChildStdin>, line: String| {
        if let Some(s) = stdin.as_mut() {
            let _ = writeln!(s, "{line}");
            let _ = s.flush();
        }
    };
    send(&mut stdin, chat_agent::initialize_line("probe-init"));
    send(&mut stdin, chat_agent::user_message_line(message));

    let mut parser = chat_agent::StreamParser::default();
    let mut streaming = false;
    for line in BufReader::new(child.stdout.take().expect("stdout")).lines() {
        let Ok(line) = line else { break };
        for inbound in parser.feed(&line) {
            match inbound {
                Inbound::Event(ChatEvent::TextDelta { text }) => {
                    streaming = true;
                    print!("{text}");
                    let _ = std::io::stdout().flush();
                }
                Inbound::Event(ChatEvent::AssistantText { .. }) if streaming => {
                    streaming = false;
                    println!();
                }
                Inbound::Event(ChatEvent::ToolUse { name, input, .. }) => {
                    println!("  → {name} {}", excerpt(&input.to_string(), 140));
                }
                Inbound::Event(ChatEvent::ToolResult { is_error, text, draft, .. }) => {
                    println!(
                        "  ← {}{} {}",
                        if is_error { "ERROR " } else { "" },
                        draft.map(|d| format!("[draft {} uid {}]", d.folder, d.uid)).unwrap_or_default(),
                        excerpt(&text, 160)
                    );
                }
                Inbound::Event(ChatEvent::Ready { session_id, model, mcp_servers }) => {
                    println!("[ready {model} session {session_id} mcp {mcp_servers:?}]");
                }
                Inbound::Event(ChatEvent::TurnDone { is_error, subtype, cost_usd, .. }) => {
                    println!("\n[turn done: {subtype} error={is_error} cost={cost_usd:?}]");
                    stdin = None;
                }
                Inbound::CanUseTool { request_id, tool_name, input, .. } => {
                    println!("  ? PERMISSION {tool_name} → {}", if allow { "allow" } else { "deny" });
                    if allow {
                        send(&mut stdin, chat_agent::allow_line(&request_id, &input));
                    } else {
                        send(&mut stdin, chat_agent::deny_line(&request_id, "Denied by chat_probe."));
                    }
                }
                Inbound::UnknownControl { request_id, subtype } => {
                    println!("  ? unknown control {subtype}");
                    send(&mut stdin, chat_agent::control_error_line(&request_id, "unsupported"));
                }
                Inbound::ControlResponse { request_id, ok, error } => {
                    println!("[control {request_id} ok={ok} {}]", error.unwrap_or_default());
                }
                Inbound::Event(_) => {}
            }
        }
    }
    let status = child.wait().expect("wait");
    println!("[exit {status}]");
}

fn excerpt(s: &str, n: usize) -> String {
    let flat = s.replace('\n', " ");
    match flat.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}
