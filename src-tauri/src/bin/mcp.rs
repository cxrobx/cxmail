//! CXMail MCP Server — stdio transport for Claude CLI integration.
//!
//! Usage:
//!   cxmail-mcp               stdio server (the default; one process per client)
//!   cxmail-mcp --http <port> one shared streamable-HTTP server at
//!                            http://127.0.0.1:<port>/mcp, loopback only, a fresh
//!                            handler per MCP session (for a LaunchAgent)
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
    service.waiting().await?;
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

    axum::serve(listener, router)
        .with_graceful_shutdown(async move { ct.cancelled_owned().await })
        .await?;
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
