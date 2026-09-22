use crate::db;
use crate::email::{imap, sync};
use crate::error::AppError;
use crate::LockExt;
use crate::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// IDLE keepalive: re-issue IDLE every 4 minutes (well under RFC 2177's 29-min max).
const IDLE_KEEPALIVE: Duration = Duration::from_secs(4 * 60);

/// Delay before the FIRST reconnection attempt after a failure. Subsequent
/// consecutive failures double it, up to `RECONNECT_DELAY_MAX`.
const RECONNECT_DELAY: Duration = Duration::from_secs(30);

/// A session that stayed up this long is treated as healthy, so the NEXT
/// failure starts the backoff over.
///
/// The signal has to be duration rather than "did connect succeed", because the
/// normal healthy lifecycle also ends in `Err`: the loop re-issues IDLE every 4
/// minutes and a keepalive that eventually fails is routine. Without this,
/// every account would ratchet to the 15-minute cap over a long session and
/// real mail would sit unfetched. One keepalive cycle is the natural threshold.
const SESSION_HEALTHY_AFTER: Duration = IDLE_KEEPALIVE;

/// Reconnect delay for the Nth consecutive failure (1-based).
///
/// Delegates to `imap::connect_backoff` so this loop and the periodic
/// `sync_all_inboxes` back off on ONE schedule. They are both reacting to the
/// same refused account, and two schedules would mean the slower one's restraint
/// is undone by the faster one's retries — which is exactly what happened when
/// only this loop had backoff.
fn backoff_delay(consecutive_failures: u32) -> Duration {
    imap::connect_backoff(consecutive_failures)
}

/// Per-operation timeout for IDLE control commands (init/done/select).
/// Prevents silent hangs when the IMAP socket is dead but TCP is alive.
const IDLE_OP_TIMEOUT: Duration = Duration::from_secs(15);

/// Start IMAP IDLE watchers for all active accounts.
/// Returns a shutdown flag — set it to true to stop all watchers.
pub fn start_idle_watchers(app_handle: AppHandle) -> Arc<AtomicBool> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let flag = shutdown.clone();

    tauri::async_runtime::spawn(async move {
        // Wait for app initialization and initial sync
        tokio::time::sleep(Duration::from_secs(10)).await;

        let Some(state) = app_handle.try_state::<AppState>() else {
            log::warn!("IDLE: AppState not available");
            return;
        };

        let accounts = {
            let conn = state.db.safe_lock();
            db::accounts::list(&conn).unwrap_or_default()
        };

        for account in accounts {
            if !account.is_active {
                continue;
            }
            log::info!("IDLE: Starting watcher for {} ({})", account.email, account.provider);
            tauri::async_runtime::spawn(idle_loop(app_handle.clone(), account, flag.clone()));
        }
    });

    shutdown
}

/// Persistent IDLE loop for a single account.
///
/// Holds the `Account` row for the life of the process, so server settings
/// edited via re-auth do not reach a running watcher until the next launch —
/// the same staleness the previous `(email, provider)` pair had.
async fn idle_loop(app: AppHandle, account: db::accounts::Account, shutdown: Arc<AtomicBool>) {
    let account_id = account.id.as_str();
    let email = account.email.as_str();
    let mut consecutive_failures: u32 = 0;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            log::info!("IDLE: Shutting down watcher for {}", email);
            return;
        }

        let started = std::time::Instant::now();
        match run_idle_session(&app, &account, &shutdown).await {
            Ok(()) => return, // Clean shutdown
            Err(e) => {
                let ran_for = started.elapsed();
                if ran_for >= SESSION_HEALTHY_AFTER {
                    consecutive_failures = 1;
                } else {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                }
                let delay = backoff_delay(consecutive_failures);
                log::warn!(
                    "IDLE: Connection lost for {} after {}s: {}. Reconnecting in {}s (consecutive failure #{})...",
                    email, ran_for.as_secs(), e, delay.as_secs(), consecutive_failures
                );
                let _ = app.emit(
                    "idle-status",
                    serde_json::json!({
                        "account_id": account_id,
                        "status": "disconnected",
                        "error": format!("{}", e),
                    }),
                );
                // Wait before reconnecting, checking shutdown periodically
                for _ in 0..delay.as_secs() {
                    if shutdown.load(Ordering::Relaxed) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{backoff_delay, RECONNECT_DELAY};

    /// Mirror of `imap::CONNECT_BACKOFF_MAX`, which is private to that module.
    const RECONNECT_DELAY_MAX: std::time::Duration = std::time::Duration::from_secs(15 * 60);

    #[test]
    fn the_first_failure_retries_at_the_old_flat_delay() {
        // Backoff must not slow down the common case: one dropped keepalive
        // should still reconnect in 30s, exactly as before.
        assert_eq!(backoff_delay(1), RECONNECT_DELAY);
    }

    #[test]
    fn consecutive_failures_double_the_delay() {
        assert_eq!(backoff_delay(2).as_secs(), 60);
        assert_eq!(backoff_delay(3).as_secs(), 120);
        assert_eq!(backoff_delay(4).as_secs(), 240);
        assert_eq!(backoff_delay(5).as_secs(), 480);
    }

    #[test]
    fn the_delay_is_capped_and_never_overflows() {
        // u32 failure counts are reachable on an account refused for days; a
        // shift by 32+ is UB-adjacent and `1u64 << 64` panics in debug, so the
        // doublings are clamped before the shift rather than after.
        assert_eq!(backoff_delay(6).as_secs(), RECONNECT_DELAY_MAX.as_secs());
        assert_eq!(backoff_delay(50).as_secs(), RECONNECT_DELAY_MAX.as_secs());
        assert_eq!(backoff_delay(u32::MAX).as_secs(), RECONNECT_DELAY_MAX.as_secs());
    }

    #[test]
    fn a_refused_account_settles_far_below_a_healthy_ones_connect_rate() {
        // The property that motivated this: the app must stop pushing hardest
        // at the account being turned away. A healthy account re-IDLEs about 16
        // times an hour; at the cap a refused one must be well under that.
        let cycle = backoff_delay(u32::MAX).as_secs() + 15; // + connect timeout
        let per_hour = 3600 / cycle;
        assert!(per_hour <= 4, "refused account would still make {per_hour}/hour");
    }
}

/// Run a single IDLE session.
async fn run_idle_session(
    app: &AppHandle,
    account: &db::accounts::Account,
    shutdown: &AtomicBool,
) -> Result<(), AppError> {
    let account_id = account.id.as_str();
    let email = account.email.as_str();
    log::info!("IDLE: Connecting for {}", email);

    let mut session = imap::connect_for_account(account).await?;

    // Every `?` below abandons `session` — the value dies, `Drop` closes the
    // socket, and no LOGOUT is attempted. That is unavoidable for most of them
    // (once `session.idle()` consumes it, the session lives inside the handle
    // and cannot be recovered to log out), but it must still be COUNTED: this
    // loop reconnects hardest on the sickest account, so leaving it unaccounted
    // is what made the old `believed_open` read 14 for an account with ~2
    // sockets. The guard rides on the scope so a future error path cannot
    // reopen the hole by forgetting.
    let mut accounting = imap::SessionGuard::new(email);

    tokio::time::timeout(IDLE_OP_TIMEOUT, session.select("INBOX"))
        .await
        .map_err(|_| AppError::Imap("IDLE SELECT timed out".to_string()))?
        .map_err(|e| AppError::Imap(format!("IDLE SELECT failed: {}", e)))?;

    log::info!("IDLE: Entering IDLE mode for {}", email);
    let _ = app.emit(
        "idle-status",
        serde_json::json!({
            "account_id": account_id,
            "status": "connected",
        }),
    );

    loop {
        if shutdown.load(Ordering::Relaxed) {
            // The one path that holds the session and can close it properly.
            // Route it through `disconnect` rather than a bare `logout()` so
            // the clean close is actually recorded as one.
            accounting.disarm();
            let _ = imap::disconnect(session, email).await;
            return Ok(());
        }

        // Create IDLE handle — consumes session, returns it via done()
        let mut idle_handle = session.idle();

        // Send IDLE command (with timeout — control command should be fast)
        tokio::time::timeout(IDLE_OP_TIMEOUT, idle_handle.init())
            .await
            .map_err(|_| AppError::Imap("IDLE init timed out".to_string()))?
            .map_err(|e| AppError::Imap(format!("IDLE init failed: {}", e)))?;

        // Wait for server notification or timeout (4 min keepalive)
        let (wait_future, _interrupt) = idle_handle.wait_with_timeout(IDLE_KEEPALIVE);
        let idle_result = wait_future
            .await
            .map_err(|e| AppError::Imap(format!("IDLE wait failed: {}", e)))?;

        let had_updates = matches!(
            idle_result,
            async_imap::extensions::idle::IdleResponse::NewData(_)
        );

        // Send DONE to end IDLE, get session back (with timeout)
        session = tokio::time::timeout(IDLE_OP_TIMEOUT, idle_handle.done())
            .await
            .map_err(|_| AppError::Imap("IDLE done timed out".to_string()))?
            .map_err(|e| AppError::Imap(format!("IDLE done failed: {}", e)))?;

        if had_updates {
            log::info!("IDLE: New mail detected for {}", email);

            // Sync new mail into the DB right here so the user sees it
            // even if the frontend listener is broken / not running.
            // Adapted from src-tauri/src/bin/helper.rs:214-301.
            if let Some(state) = app.try_state::<AppState>() {
                mini_sync(&state, app, &mut session, account_id, email).await;
            } else {
                log::warn!("IDLE: AppState unavailable, skipping mini_sync for {}", email);
            }

            let _ = app.emit(
                "idle-new-mail",
                serde_json::json!({
                    "account_id": account_id,
                    "folder": "INBOX",
                }),
            );
            // Refresh dock badge and tray tooltip from current DB state
            crate::notify::update_badge(app);
        }
    }
}

/// Quick sync triggered by IDLE NewData: prepare → fetch headers → apply →
/// reconcile, with small lock scopes. New headers land in the DB before we
/// emit the idle-new-mail event so the frontend (or any other listener) sees
/// fresh data the moment it refreshes. Errors are logged and swallowed —
/// the IDLE loop must keep running.
///
/// Adapted from `src-tauri/src/bin/helper.rs::mini_sync`.
async fn mini_sync(
    state: &AppState,
    app: &AppHandle,
    session: &mut imap::ImapSession,
    account_id: &str,
    email: &str,
) {
    let mut prepared = {
        let conn = state.db.safe_lock();
        match sync::prepare_folder_sync(&conn, account_id, "INBOX") {
            Ok(p) => p,
            Err(e) => {
                log::warn!("IDLE mini_sync: prepare failed for {}: {}", email, e);
                return;
            }
        }
    };

    let plan = match sync::select_and_plan(session, "INBOX", &prepared.sync_state).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("IDLE mini_sync: select_and_plan failed for {}: {}", email, e);
            return;
        }
    };

    let was_reset = {
        let conn = state.db.safe_lock();
        match sync::apply_reset_if_needed(&conn, account_id, "INBOX", &plan) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("IDLE mini_sync: reset failed for {}: {}", email, e);
                return;
            }
        }
    };
    if was_reset {
        // Keep in-memory snapshot in sync with the now-empty cache so per-batch
        // new-mail filtering doesn't suppress legitimate notifications.
        prepared.existing_uids.clear();
    }

    let existing_uids = prepared.existing_uids;
    let mut total_new = 0usize;
    let mut final_last_uid = plan.initial_last_uid;
    // Single-batch = real-time IDLE arrival. Multi-batch = catch-up after a
    // long disconnect; suppress per-message popups to avoid notification storms.
    let notify_per_batch = plan.batches.len() <= 1;

    for &(start_uid, end_uid) in &plan.batches {
        let batch_headers = match imap::fetch_headers(session, start_uid, end_uid).await {
            Ok(h) => h,
            Err(e) => {
                log::warn!(
                    "IDLE mini_sync: fetch_headers {}..{} failed for {}: {}",
                    start_uid, end_uid, email, e
                );
                return;
            }
        };
        {
            let conn = state.db.safe_lock();
            if let Err(e) = sync::apply_header_batch(
                &conn,
                account_id,
                "INBOX",
                &batch_headers,
                end_uid,
                plan.uidvalidity,
            ) {
                log::warn!(
                    "IDLE mini_sync: apply_header_batch {}..{} failed for {}: {}",
                    start_uid, end_uid, email, e
                );
                return;
            }
        }

        let new_mail: Vec<crate::notify::NewMailInfo> = batch_headers
            .iter()
            .filter(|h| !was_reset && !existing_uids.contains(&h.uid) && !h.is_read)
            .map(|h| crate::notify::NewMailInfo {
                account_id: account_id.to_string(),
                folder: "INBOX".to_string(),
                uid: h.uid,
                from_name: h.from_name.clone(),
                from_email: h.from_email.clone(),
                subject: h.subject.clone(),
            })
            .collect();

        if !new_mail.is_empty() {
            total_new += new_mail.len();
            if notify_per_batch {
                if crate::notify::account_notify_enabled(app, account_id) {
                    crate::notify::on_new_mail(app, &new_mail);
                } else {
                    // Account is muted: skip popups but keep tray badge accurate.
                    crate::notify::update_badge(app);
                }
            }
        }

        final_last_uid = end_uid;
    }

    {
        let conn = state.db.safe_lock();
        if let Err(e) = sync::finalize_header_sync(
            &conn,
            account_id,
            "INBOX",
            final_last_uid,
            plan.uidvalidity,
            plan.uidnext,
        ) {
            log::warn!("IDLE mini_sync: finalize failed for {}: {}", email, e);
            return;
        }
    }

    let reconciliation_plan = {
        let conn = state.db.safe_lock();
        match sync::prepare_reconciliation(&conn, account_id, "INBOX", sync::SyncMode::Limited) {
            Ok(plan) => plan,
            Err(e) => {
                log::warn!(
                    "IDLE mini_sync: reconciliation prepare failed for {}: {}",
                    email, e
                );
                return;
            }
        }
    };

    let reconciliation = match sync::fetch_reconciliation(session, &reconciliation_plan).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!(
                "IDLE mini_sync: reconciliation fetch failed for {}: {}",
                email, e
            );
            return;
        }
    };

    {
        let conn = state.db.safe_lock();
        if let Err(e) = sync::apply_reconciliation(
            &conn,
            account_id,
            "INBOX",
            &reconciliation_plan,
            reconciliation,
        ) {
            log::warn!(
                "IDLE mini_sync: reconciliation apply failed for {}: {}",
                email, e
            );
            return;
        }
    }

    if total_new > 0 {
        log::info!(
            "IDLE mini_sync: {} new message(s) for {}",
            total_new,
            email
        );
    }
}
