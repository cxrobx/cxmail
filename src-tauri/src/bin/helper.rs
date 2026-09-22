use cxmail_lib::db;
use cxmail_lib::email::{imap, sync};
use cxmail_lib::LockExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const IDLE_KEEPALIVE: Duration = Duration::from_secs(4 * 60);
const RECONNECT_DELAY: Duration = Duration::from_secs(30);
const LOCK_CHECK_INTERVAL: Duration = Duration::from_secs(30);

fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.cxmail.app")
}

fn lock_file_path() -> PathBuf {
    app_data_dir().join("app.lock")
}

/// Check if the main CXMail app is running by reading the lock file PID.
fn is_main_app_running() -> bool {
    let lock_path = lock_file_path();
    if let Ok(contents) = std::fs::read_to_string(&lock_path) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            // kill(pid, 0) checks if process exists without sending a signal
            unsafe { libc::kill(pid, 0) == 0 }
        } else {
            false
        }
    } else {
        false
    }
}

/// Send a macOS notification attributed to CXMail (shows CXMail icon).
fn send_notification(title: &str, body: &str) {
    let _ = notify_rust::set_application("com.cxmail.app");
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .sound_name("default")
        .show();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    log::info!("cxmail-helper starting");

    // Check for --sync flag (required by LaunchAgent plist)
    let args: Vec<String> = std::env::args().collect();
    if !args.iter().any(|a| a == "--sync") {
        eprintln!("Usage: cxmail-helper --sync");
        std::process::exit(1);
    }

    let db_path = app_data_dir().join("cxmail.db");
    if !db_path.exists() {
        log::info!("Database not found at {:?}, waiting for main app to create it", db_path);
        // Wait for main app to create the DB
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if db_path.exists() {
                break;
            }
        }
    }

    let conn = rusqlite::Connection::open(&db_path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    db::schema::initialize(&conn)?;
    log::info!("Database opened at {:?}", db_path);

    let db = Arc::new(Mutex::new(conn));
    let shutdown = Arc::new(AtomicBool::new(false));

    loop {
        // Wait while main app is running — avoid duplicate IDLE connections
        while is_main_app_running() {
            log::info!("Main app is running, standing by...");
            tokio::time::sleep(LOCK_CHECK_INTERVAL).await;
        }
        log::info!("Main app not running, starting IDLE watchers");

        // Get active accounts
        let accounts = {
            let conn = db.safe_lock();
            db::accounts::list(&conn).unwrap_or_default()
        };

        if accounts.is_empty() {
            log::info!("No accounts configured, waiting...");
            tokio::time::sleep(Duration::from_secs(60)).await;
            continue;
        }

        // Spawn per-account IDLE loops
        let mut handles = Vec::new();
        shutdown.store(false, Ordering::Relaxed);

        for account in &accounts {
            if !account.is_active {
                continue;
            }
            let db = db.clone();
            let shutdown = shutdown.clone();
            let account = account.clone();

            let handle = tokio::spawn(async move {
                idle_loop(db, shutdown, account).await;
            });
            handles.push(handle);
        }

        // Monitor lock file — if main app starts, shut down IDLE loops
        loop {
            tokio::time::sleep(LOCK_CHECK_INTERVAL).await;
            if is_main_app_running() {
                log::info!("Main app started, stopping IDLE watchers");
                shutdown.store(true, Ordering::Relaxed);
                // Wait for all IDLE loops to notice and exit
                for handle in handles {
                    let _ = handle.await;
                }
                break;
            }
        }
    }
}

async fn idle_loop(
    db: Arc<Mutex<rusqlite::Connection>>,
    shutdown: Arc<AtomicBool>,
    account: cxmail_lib::db::accounts::Account,
) {
    let email = account.email.as_str();
    loop {
        if shutdown.load(Ordering::Relaxed) {
            log::info!("IDLE daemon: shutting down for {}", email);
            return;
        }

        match run_idle_session(&db, &shutdown, &account).await {
            Ok(()) => return, // Clean shutdown
            Err(e) => {
                log::warn!(
                    "IDLE daemon: connection lost for {}: {}. Reconnecting in {}s...",
                    email, e, RECONNECT_DELAY.as_secs()
                );
                for _ in 0..RECONNECT_DELAY.as_secs() {
                    if shutdown.load(Ordering::Relaxed) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }
}

async fn run_idle_session(
    db: &Arc<Mutex<rusqlite::Connection>>,
    shutdown: &AtomicBool,
    account: &cxmail_lib::db::accounts::Account,
) -> Result<(), cxmail_lib::error::AppError> {
    let account_id = account.id.as_str();
    let email = account.email.as_str();
    log::info!("IDLE daemon: connecting for {}", email);
    let mut session = imap::connect_for_account(account).await?;
    // Same accounting hole as `email/idle.rs` — every `?` below abandons the
    // session without a LOGOUT attempt. This binary keeps its own process-local
    // ledger, so it needs its own guard.
    let mut accounting = imap::SessionGuard::new(email);
    imap::select_folder(&mut session, "INBOX").await?;
    log::info!("IDLE daemon: entering IDLE for {}", email);

    loop {
        if shutdown.load(Ordering::Relaxed) {
            accounting.disarm();
            let _ = imap::disconnect(session, email).await;
            return Ok(());
        }

        let mut idle_handle = session.idle();
        idle_handle
            .init()
            .await
            .map_err(|e| cxmail_lib::error::AppError::Imap(format!("IDLE init failed: {}", e)))?;

        let (wait_future, _interrupt) = idle_handle.wait_with_timeout(IDLE_KEEPALIVE);
        let idle_result = wait_future
            .await
            .map_err(|e| cxmail_lib::error::AppError::Imap(format!("IDLE wait failed: {}", e)))?;

        let had_updates = matches!(
            idle_result,
            async_imap::extensions::idle::IdleResponse::NewData(_)
        );

        session = idle_handle
            .done()
            .await
            .map_err(|e| cxmail_lib::error::AppError::Imap(format!("IDLE done failed: {}", e)))?;

        if had_updates {
            log::info!("IDLE daemon: new mail for {}", email);
            mini_sync(db, &mut session, account_id, email).await;
        }
    }
}

/// Quick sync: fetch new headers per batch, insert into DB, send notification
/// for new unread messages as each batch lands.
async fn mini_sync(
    db: &Arc<Mutex<rusqlite::Connection>>,
    session: &mut imap::ImapSession,
    account_id: &str,
    email: &str,
) {
    let mut prepared = {
        let conn = db.safe_lock();
        match sync::prepare_folder_sync(&conn, account_id, "INBOX") {
            Ok(prepared) => prepared,
            Err(e) => {
                log::warn!("IDLE daemon mini_sync: failed to prepare sync state for {}: {}", email, e);
                return;
            }
        }
    };

    let plan = match sync::select_and_plan(session, "INBOX", &prepared.sync_state).await {
        Ok(plan) => plan,
        Err(e) => {
            log::warn!("IDLE daemon mini_sync: select_and_plan failed for {}: {}", email, e);
            return;
        }
    };

    let was_reset = {
        let conn = db.safe_lock();
        match sync::apply_reset_if_needed(&conn, account_id, "INBOX", &plan) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("IDLE daemon mini_sync: reset failed for {}: {}", email, e);
                return;
            }
        }
    };
    if was_reset {
        // Keep in-memory snapshot in sync with the now-empty cache so per-batch
        // notification filtering doesn't suppress legitimate new mail.
        prepared.existing_uids.clear();
    }

    let existing_uids = prepared.existing_uids;
    let mut total_new = 0usize;
    let mut final_last_uid = plan.initial_last_uid;
    // Single-batch syncs are real-time arrivals; multi-batch = catch-up after
    // a long disconnect. Suppress per-message OS notifications in the latter
    // case to avoid spamming the user during reconnection.
    let notify_per_batch = plan.batches.len() <= 1;

    for &(start_uid, end_uid) in &plan.batches {
        let batch_headers = match imap::fetch_headers(session, start_uid, end_uid).await {
            Ok(h) => h,
            Err(e) => {
                log::warn!(
                    "IDLE daemon mini_sync: fetch_headers {}..{} failed for {}: {}",
                    start_uid, end_uid, email, e
                );
                return;
            }
        };
        {
            let conn = db.safe_lock();
            if let Err(e) = sync::apply_header_batch(
                &conn,
                account_id,
                "INBOX",
                &batch_headers,
                end_uid,
                plan.uidvalidity,
            ) {
                log::warn!(
                    "IDLE daemon mini_sync: apply_header_batch {}..{} failed for {}: {}",
                    start_uid, end_uid, email, e
                );
                return;
            }
        }

        let new_in_batch: Vec<_> = batch_headers
            .iter()
            .filter(|h| !was_reset && !existing_uids.contains(&h.uid) && !h.is_read)
            .collect();
        if notify_per_batch {
            for msg in &new_in_batch {
                let title = msg
                    .from_name
                    .as_deref()
                    .or(msg.from_email.as_deref())
                    .unwrap_or("New Message");
                let body = msg.subject.as_deref().unwrap_or("(no subject)");
                send_notification(title, body);
            }
        }
        total_new += new_in_batch.len();
        final_last_uid = end_uid;
    }

    {
        let conn = db.safe_lock();
        if let Err(e) = sync::finalize_header_sync(
            &conn,
            account_id,
            "INBOX",
            final_last_uid,
            plan.uidvalidity,
            plan.uidnext,
        ) {
            log::warn!("IDLE daemon mini_sync: finalize failed for {}: {}", email, e);
            return;
        }
    }

    let reconciliation_plan = {
        let conn = db.safe_lock();
        match sync::prepare_reconciliation(&conn, account_id, "INBOX", sync::SyncMode::Limited) {
            Ok(plan) => plan,
            Err(e) => {
                log::warn!("IDLE daemon mini_sync: failed to prepare reconciliation for {}: {}", email, e);
                return;
            }
        }
    };
    let reconciliation = match sync::fetch_reconciliation(session, &reconciliation_plan).await {
        Ok(reconciliation) => reconciliation,
        Err(e) => {
            log::warn!("IDLE daemon mini_sync: reconciliation fetch failed for {}: {}", email, e);
            return;
        }
    };
    {
        let conn = db.safe_lock();
        if let Err(e) =
            sync::apply_reconciliation(&conn, account_id, "INBOX", &reconciliation_plan, reconciliation)
        {
            log::warn!("IDLE daemon mini_sync: failed to apply reconciliation for {}: {}", email, e);
            return;
        }
    }

    if total_new > 0 {
        log::info!(
            "IDLE daemon: {} new messages for {}, notifications sent",
            total_new,
            email
        );
    }
}
