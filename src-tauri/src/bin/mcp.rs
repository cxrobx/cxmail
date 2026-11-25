//! CXMail MCP Server — stdio transport for Claude CLI integration.
//!
//! Usage:
//!   cxmail-mcp               stdio server (the default; one process per client)
//!   cxmail-mcp --http <port> one shared streamable-HTTP server at
//!                            http://127.0.0.1:<port>/mcp, loopback only, a fresh
//!                            handler per MCP session (for a LaunchAgent)
//!   cxmail-mcp remote preview-draft <account_id> <folder> <uid>
//!   cxmail-mcp remote send-draft <account_id> <folder> <uid> <digest>
//!                            CXMail Remote's send path (`remote/`): one JSON
//!                            object on stdout, exit 1 with {"error"} on failure.
//!                            NOT MCP tools, on purpose: the MCP never sends.
//!   cxmail-mcp remote chat [<account_id> <folder> <uid>] [--model <id>]
//!                            CXMail Remote's chat: the desktop chat panel's
//!                            launch and permission gate over JSON lines.
//! Register with Claude Code via the repo-root `.mcp.json`:
//!   { "mcpServers": { "cxmail": { "command": "./src-tauri/target/release/cxmail-mcp" } } }
//! or globally with: claude mcp add cxmail /abs/path/to/cxmail-mcp
//! See docs/mcp-setup.md for the full guide.

use cxmail_lib::mcp::server::CxMailMcp;
use rmcp::{transport::stdio, ServiceExt};

/// Parse `--http <port>` / `--http=<port>`. Anything else (including no args)
/// means stdio, exactly as before the flag existed.
fn http_port_from_args() -> anyhow::Result<Option<u16>> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = if arg == "--http" {
            args.next()
                .ok_or_else(|| anyhow::anyhow!("--http needs a port, e.g. --http 18704"))?
        } else if let Some(v) = arg.strip_prefix("--http=") {
            v.to_string()
        } else {
            continue;
        };
        let port: u16 = value
            .parse()
            .map_err(|_| anyhow::anyhow!("--http: not a port: {value:?}"))?;
        if port == 0 {
            anyhow::bail!("--http: port must be non-zero");
        }
        return Ok(Some(port));
    }
    Ok(None)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Logging to stderr — stdout is the MCP wire protocol
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("cxmail=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    // Resolve the database path (same as Tauri app)
    let db_path = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.cxmail.app")
        .join("cxmail.db");

    // Measurement mode, before the startup migration below: it opens the
    // database read-only and must not write to it (cxmail_mcp::risk_eval).
    if std::env::args().any(|a| a == "--set-openai-key-from-env") {
        return cxmail_mcp::risk_eval::set_openai_key_from_env();
    }
    if std::env::args().any(|a| a == "--risk-eval") {
        return cxmail_mcp::risk_eval::run(&db_path).await;
    }

    // Search is read-only, including startup: the resident app owns migrations.
    let args: Vec<String> = std::env::args().collect();
    if args.len()==3 && args[1]=="remote" && matches!(args[2].as_str(), "enable-local-search" | "index-search") {
        let result = if args[2]=="enable-local-search" {
            cxmail_lib::email::hybrid_search::enable_local(&db_path).map(|_|serde_json::json!({"ok":true}))
        } else { cxmail_lib::email::hybrid_search::index_local(&db_path).await };
        match result {
            Ok(value) => println!("{value}"),
            Err(e) => { println!("{}",serde_json::json!({"error":e.to_string()}));std::process::exit(1); }
        }
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("remote") && matches!(args.get(2).map(String::as_str), Some("search" | "search-keywords")) {
        let result = if (4..=6).contains(&args.len()) {
            match args.get(5).map_or(Ok(0), |s| s.parse::<usize>()) {
                Ok(offset) => cxmail_lib::email::hybrid_search::search(&db_path, &args[3], args.get(4).map(String::as_str).filter(|s| !s.is_empty()), offset, args[2]=="search").await,
                Err(_) => Err(cxmail_lib::error::AppError::General("Invalid search offset".into())),
            }
        } else { Err(cxmail_lib::error::AppError::General("Invalid search arguments".into())) };
        match result {
            Ok(page) => println!("{}", serde_json::to_string(&page)?),
            Err(e) => { println!("{}", serde_json::json!({"error": e.to_string()})); std::process::exit(1); }
        }
        return Ok(());
    }

    // Readiness only: never reads mail, calls a provider, or prints credentials.
    if args.len() == 3 && args[1] == "remote" && args[2] == "notification-status" {
        use cxmail_lib::email::inference::{InferenceClient, ProviderKind};
        let ready = InferenceClient::load()
            .map(|client| client.provider_kind() == ProviderKind::Openai)
            .unwrap_or(false);
        println!("{}", serde_json::json!({"model_ready": ready}));
        return Ok(());
    }

    // Notification inference is read-only, before migrations, and never an MCP tool.
    if args.get(1).map(String::as_str) == Some("remote") && args.get(2).map(String::as_str) == Some("notification-verdict") {
        let result = if args.len() == 6 {
            match args[5].parse::<u32>() {
                Ok(uid) if uid > 0 => notification_verdict(&db_path, &args[3], &args[4], uid).await,
                _ => Err(anyhow::anyhow!("Invalid UID")),
            }
        } else { Err(anyhow::anyhow!("Invalid notification arguments")) };
        match result {
            Ok(value) => println!("{value}"),
            Err(_) => { println!("{}", serde_json::json!({"error":"Notification inference unavailable"})); std::process::exit(1); }
        }
        return Ok(());
    }

    // Signature lookup is read-only even during startup.
    if args.get(1).map(String::as_str) == Some("remote") && args.get(2).map(String::as_str) == Some("compose-identity") {
        let (code, out) = run_remote(&db_path.to_string_lossy(), &args[2..]).await;
        println!("{out}");
        std::process::exit(code);
    }

    tracing::info!("CXMail MCP server starting, DB: {:?}", db_path);

    // Run schema migrations. The Tauri app also runs these on startup, but the
    // MCP binary may be invoked first by an external Claude session — without
    // this, calls to new tables (e.g. voice_profiles_recipient) would fail with
    // "no such table". Migrations are idempotent (version-gated) and use
    // CREATE TABLE IF NOT EXISTS, so it's safe to run alongside the Tauri app.
    if db_path.exists() {
        match rusqlite::Connection::open(&db_path) {
            Ok(conn) => {
                if let Err(e) = cxmail_lib::db::schema::initialize(&conn) {
                    tracing::warn!("Schema migration on MCP startup failed: {}", e);
                }
            }
            Err(e) => {
                tracing::warn!("Could not open DB for migrations: {}", e);
            }
        }
    }

    let db_path = db_path.to_string_lossy().to_string();

    if std::env::args().nth(1).as_deref() == Some("remote") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        if args.first().map(String::as_str) == Some("chat") {
            let mut rest: Vec<String> = args[1..].to_vec();
            let mut model = None;
            if let Some(i) = rest.iter().position(|a| a == "--model") {
                model = rest.get(i + 1).cloned();
                rest.drain(i..(i + 2).min(rest.len()));
            }
            let seed = match rest.as_slice() {
                [] => None,
                [a, f, u] => match u.parse::<u32>() {
                    Ok(uid) => Some(cxmail_lib::ChatSeed { account_id: a.clone(), folder: f.clone(), uid }),
                    Err(_) => {
                        println!("{}", serde_json::json!({ "error": "uid must be a number" }));
                        std::process::exit(1);
                    }
                },
                _ => {
                    println!("{}", serde_json::json!({ "error": "usage: cxmail-mcp remote chat [<account> <folder> <uid>] [--model <id>]" }));
                    std::process::exit(1);
                }
            };
            std::process::exit(cxmail_lib::run_remote_chat(&db_path, seed, model).await);
        }
        let (code, out) = run_remote(&db_path, &args).await;
        println!("{out}");
        std::process::exit(code);
    }

    if let Some(port) = http_port_from_args()? {
        // No orphan watchdog here: under launchd our parent IS pid 1, and a
        // shared server is meant to sit idle between sessions.
        return serve_http(db_path, port).await;
    }

    let server = CxMailMcp::new(db_path);

    // Self-reap so abandoned processes don't accumulate (see fn docs below).
    spawn_orphan_watchdog(server.claimed_flag());

    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("MCP init error: {:?}", e))?;

    tracing::info!("MCP server running");
    let waited = service.waiting().await;
    // Held IMAP sessions live in a `static`, which is never dropped: log them
    // out now or they close without LOGOUT (gotcha #69).
    cxmail_lib::email::body_fetch::shared().close_all().await;
    waited?;
    Ok(())
}

/// Background self-reaper so abandoned `cxmail-mcp` processes don't pile up.
///
/// A clean client exit already terminates us: closing stdin makes rmcp's
/// `serve(stdio())` loop finish and `main` return. This covers the two cases
/// that stdin-EOF does not, checked every 30s:
///
/// 1. **Orphaned** — `getppid() == 1` means the launching Claude session died
///    and we were reparented to launchd without stdin closing; exit.
/// 2. **Never claimed** — if no client completed `initialize` within the grace
///    window, we were spawned but never connected (e.g. a pre-warmed spare that
///    was abandoned across a version upgrade). Real clients initialize within
///    milliseconds, so a long-idle *claimed* session is never affected.
fn spawn_orphan_watchdog(claimed: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;

    // Grace before an unclaimed process reaps itself. Env-overridable so it can
    // be tuned or disabled (0 = watchdog off — a safety valve for a background
    // task that calls process::exit). The poll interval tracks the grace (capped
    // at 30s) so a small grace still reacts quickly.
    let grace_secs: u64 = std::env::var("CXMAIL_MCP_IDLE_GRACE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    if grace_secs == 0 {
        tracing::info!("cxmail-mcp: orphan watchdog disabled (CXMAIL_MCP_IDLE_GRACE_SECS=0)");
        return;
    }
    let check = std::time::Duration::from_secs(grace_secs.clamp(1, 30));

    tokio::spawn(async move {
        let mut elapsed: u64 = 0;
        loop {
            tokio::time::sleep(check).await;
            elapsed = elapsed.saturating_add(check.as_secs());

            // 1. Parent gone → reparented to launchd. SAFETY: getppid is always safe.
            if unsafe { libc::getppid() } == 1 {
                tracing::info!("cxmail-mcp: parent gone (reparented to launchd); exiting");
                std::process::exit(0);
            }

            // 2. Spawned but never claimed by a client within the grace window.
            if elapsed >= grace_secs && !claimed.load(Ordering::Relaxed) {
                tracing::info!(
                    "cxmail-mcp: no client initialized within {grace_secs}s; exiting abandoned spawn"
                );
                std::process::exit(0);
            }
        }
    });
}

/// Serve MCP over streamable HTTP on `127.0.0.1:<port>/mcp` until SIGTERM/SIGINT.
///
/// Stateful mode: each client `initialize` creates an MCP session with its own
/// `CxMailMcp` from the factory, so peer info and every other handler field are
/// per session. Tool calls open their own DB connection; nothing is cached.
///
/// Loopback binding alone does not stop a web page from reaching us (DNS
/// rebinding: a hostile name resolving to 127.0.0.1), and this server can send
/// mail. So every request must carry a loopback `Host`, and any `Origin` must
/// be loopback too — browsers always send `Origin` on cross-origin POSTs.
async fn serve_http(db_path: String, port: u16) -> anyhow::Result<()> {
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };
    use tokio_util::sync::CancellationToken;

    // A session whose client vanished without DELETE is dropped after this
    // much inactivity. Clients reinitialize on the 404 that follows.
    let idle_secs: u64 = std::env::var("CXMAIL_MCP_HTTP_SESSION_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24 * 60 * 60);

    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive =
        (idle_secs > 0).then(|| std::time::Duration::from_secs(idle_secs));

    let ct = CancellationToken::new();
    let config = StreamableHttpServerConfig::default().with_cancellation_token(ct.clone());
    let service: StreamableHttpService<CxMailMcp, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(CxMailMcp::new_http_session(db_path.clone())),
        std::sync::Arc::new(sessions),
        config,
    );

    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(move |req, next| {
            loopback_only(port, req, next)
        }));

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("cxmail-mcp: cannot bind {addr}: {e}"))?;
    tracing::info!("MCP server running (streamable HTTP) at http://{addr}/mcp");

    let shutdown = ct.clone();
    tokio::spawn(async move {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async { match term.as_mut() { Some(t) => { t.recv().await; } None => std::future::pending::<()>().await } } => {}
        }
        tracing::info!("cxmail-mcp: shutting down HTTP server");
        shutdown.cancel();
    });

    let served = axum::serve(listener, router)
        .with_graceful_shutdown(async move { ct.cancelled_owned().await })
        .await;
    cxmail_lib::email::body_fetch::shared().close_all().await;
    served?;
    Ok(())
}

/// Reject requests whose `Host` or `Origin` is not this loopback server.
async fn loopback_only(
    port: u16,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    let allowed_host = |h: &str| {
        let h = h.to_ascii_lowercase();
        [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")]
            .iter()
            .any(|ok| *ok == h)
    };
    let host_ok = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(allowed_host);
    let origin_ok = match req.headers().get(header::ORIGIN) {
        None => true,
        Some(v) => v
            .to_str()
            .ok()
            .and_then(|o| o.strip_prefix("http://"))
            .is_some_and(allowed_host),
    };
    if !host_ok || !origin_ok {
        tracing::warn!("cxmail-mcp: refused request with non-loopback Host/Origin");
        return (StatusCode::FORBIDDEN, "cxmail-mcp: loopback clients only").into_response();
    }
    next.run(req).await
}

/// `cxmail-mcp remote …` — see the module docs. Returns (exit code, JSON).
async fn run_remote(db_path: &str, args: &[String]) -> (i32, String) {
    use cxmail_lib::mcp_remote_send as rs;
    let err = |m: String| (1, serde_json::json!({ "error": m }).to_string());
    let uid = |s: Option<&String>| s.and_then(|s| s.parse::<u32>().ok());
    match args.first().map(String::as_str) {
        Some("compose-identity") if args.len() == 2 || args.len() == 4 => {
            let reply = if args.len() == 4 {
                let Some(u) = uid(args.get(3)).filter(|u| *u > 0) else {
                    return err("Invalid reply UID".into());
                };
                Some((args[2].as_str(), u))
            } else { None };
            match rs::compose_identity(db_path, &args[1], reply) {
                Ok(identity) => (0, identity.to_string()),
                Err(e) => err(e.to_string()),
            }
        }
        Some("image-policy") if args.len() == 4 => match uid(args.get(3)) {
            Some(u) => match cxmail_lib::email::image_policy::load(std::path::Path::new(db_path), &args[1], &args[2], u).await {
                Ok(p) => (0, serde_json::to_string(&p).unwrap_or_default()),
                Err(e) => err(e.to_string()),
            },
            None => err("uid must be a number".into()),
        },
        Some("trust-images" | "untrust-images") if args.len() == 4 => match uid(args.get(3)) {
            Some(u) => match cxmail_lib::email::image_policy::set_trusted(std::path::Path::new(db_path), &args[1], &args[2], u, args[0] == "trust-images") {
                Ok(()) => (0, serde_json::json!({"ok": true}).to_string()),
                Err(e) => err(e.to_string()),
            },
            None => err("uid must be a number".into()),
        },
        Some("preview-draft") if args.len() == 4 => match uid(args.get(3)) {
            Some(u) => match rs::preview_draft(db_path, &args[1], &args[2], u).await {
                Ok(p) => (0, serde_json::to_string(&p).unwrap_or_default()),
                Err(e) => err(e.to_string()),
            },
            None => err("uid must be a number".into()),
        },
        Some("draft-attachment") if args.len() == 6 => {
            match (uid(args.get(3)), args[5].parse::<usize>()) {
                (Some(u), Ok(index)) => match rs::preview_attachment(db_path, &args[1], &args[2], u, &args[4], index).await {
                    Ok(bytes) => {
                        use base64::Engine;
                        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                        (0, serde_json::json!({ "data": encoded }).to_string())
                    }
                    Err(e) => err(e.to_string()),
                },
                _ => err("Invalid draft UID or attachment index".into()),
            }
        }
        Some("edit-draft") if args.len() == 5 => match uid(args.get(3)) {
            Some(u) => match serde_json::from_reader(std::io::stdin().lock()) {
                Ok(edit) => match rs::edit_reviewed_draft(db_path, &args[1], &args[2], u, &args[4], edit).await {
                    Ok(saved) => (0, serde_json::to_string(&saved).unwrap_or_default()),
                    Err(e) => err(e.to_string()),
                },
                Err(_) => err("Invalid draft edit".into()),
            },
            None => err("uid must be a number".into()),
        },
        Some("send-draft") if args.len() == 5 => match uid(args.get(3)) {
            Some(u) => match rs::send_draft(db_path, &args[1], &args[2], u, &args[4]).await {
                Ok(o) => (0, serde_json::to_string(&o).unwrap_or_default()),
                Err(e) => err(e.to_string()),
            },
            None => err("uid must be a number".into()),
        },
        _ => err("usage: cxmail-mcp remote preview-draft <account> <folder> <uid> | send-draft <account> <folder> <uid> <digest>".into()),
    }
}

/// The companion's explicit opt-in owns this separate path. Global desktop
/// triage settings remain unchanged. The shared gate still owns every byte.
async fn notification_verdict(path: &std::path::Path, account: &str, folder: &str, uid: u32) -> anyhow::Result<serde_json::Value> {
    use cxmail_lib::email::{inference::InferenceClient, triage, triage_gate::{self, Gate, MessageForTriage, TriageMode}};
    let (from, name, to, cc, date, subject, body) = {
        let c = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        c.query_row("SELECT m.from_email,m.from_name,m.to_list,m.cc_list,m.date,m.subject,b.plain_text
            FROM messages m JOIN accounts a ON a.id=m.account_id
            JOIN folders f ON f.account_id=m.account_id AND f.name=m.folder_name
            LEFT JOIN message_bodies b ON b.account_id=m.account_id AND b.folder_name=m.folder_name AND b.uid=m.uid
            WHERE m.account_id=?1 AND m.folder_name=?2 AND m.uid=?3 AND m.is_read=0
            AND a.is_active=1 AND a.hidden_from_aggregates=0 AND f.folder_type='inbox'
            AND julianday(m.date)>julianday('now','-1 day')", rusqlite::params![account,folder,uid], |r| {
                Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?,
                    r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?))
            })?
    };
    if body.as_deref().is_none_or(|b| b.trim().is_empty()) {
        return Ok(serde_json::json!({"pending":true}));
    }
    let addrs = |raw: Option<&str>| -> Vec<String> {
        raw.and_then(|s|serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v|v.as_array().cloned()).unwrap_or_default().iter()
            .filter_map(|v|v["email"].as_str().map(str::to_owned)).collect()
    };
    let to=addrs(to.as_deref()); let cc=addrs(cc.as_deref());
    let safe = match triage_gate::gate(TriageMode::On, &MessageForTriage {
        from_email:&from, from_name:name.as_deref(), to:&to, cc:&cc, date:&date,
        subject:subject.as_deref(), body:body.as_deref(),
    }) {
        Gate::Send(safe) => safe,
        Gate::Withheld(_) => return Ok(serde_json::json!({"withheld":true})),
    };
    let client=InferenceClient::load()?;
    let result=triage::classify(&client,&safe,triage::DECISIONS_ENGINE,"none").await?;
    Ok(serde_json::to_value(result.verdict)?)
}

#[cfg(test)]
mod notification_tests {
    use super::notification_verdict;

    struct FixtureDir(std::path::PathBuf);
    impl Drop for FixtureDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn fixture() -> (FixtureDir, std::path::PathBuf) {
        let dir = FixtureDir(std::env::temp_dir().join(format!("cxmail-notify-test-{}",uuid::Uuid::new_v4())));
        std::fs::create_dir(&dir.0).unwrap();
        let path = dir.0.join("mail.sqlite");
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE messages(account_id TEXT,folder_name TEXT,uid INTEGER,
            from_email TEXT,from_name TEXT,to_list TEXT,cc_list TEXT,date TEXT,subject TEXT,is_read INTEGER);
            CREATE TABLE accounts(id TEXT,is_active INTEGER,hidden_from_aggregates INTEGER);
            CREATE TABLE folders(account_id TEXT,name TEXT,folder_type TEXT);
            CREATE TABLE message_bodies(account_id TEXT,folder_name TEXT,uid INTEGER,plain_text TEXT);
            INSERT INTO accounts VALUES ('fiction',1,0);
            INSERT INTO folders VALUES ('fiction','INBOX','inbox');
            INSERT INTO messages VALUES ('fiction','INBOX',1,'person@example.org',NULL,'[]','[]',datetime('now'),'Hello',0);")
            .unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn missing_body_stays_pending_without_credentials_or_inference() {
        let (_dir,path)=fixture();
        let value=notification_verdict(&path,"fiction","INBOX",1).await.unwrap();
        assert_eq!(value,serde_json::json!({"pending":true}));
    }

    #[tokio::test]
    async fn hidden_read_and_non_inbox_never_reach_credentials() {
        for sql in ["UPDATE accounts SET hidden_from_aggregates=1", "UPDATE messages SET is_read=1",
                    "UPDATE folders SET folder_type='junk'", "UPDATE accounts SET is_active=0"] {
            let (_dir,path)=fixture();
            rusqlite::Connection::open(&path).unwrap().execute_batch(sql).unwrap();
            assert!(notification_verdict(&path,"fiction","INBOX",1).await.is_err());
        }
    }
}
