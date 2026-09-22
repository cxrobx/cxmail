//! CXMail MCP Server — stdio transport for Claude CLI integration.
//!
//! Usage: cxmail-mcp   (argless stdio server — takes no flags)
//! Register with Claude Code via the repo-root `.mcp.json`:
//!   { "mcpServers": { "cxmail": { "command": "./src-tauri/target/release/cxmail-mcp" } } }
//! or globally with: claude mcp add cxmail /abs/path/to/cxmail-mcp
//! See docs/mcp-setup.md for the full guide.

use cxmail_lib::mcp::server::CxMailMcp;
use rmcp::{transport::stdio, ServiceExt};

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

    let server = CxMailMcp::new(db_path.to_string_lossy().to_string());

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
