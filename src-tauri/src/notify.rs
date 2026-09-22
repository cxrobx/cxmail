use crate::db;
use crate::AppState;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

/// Minimum seconds between notification bursts.
const DEBOUNCE_SECS: u64 = 5;

/// Timestamp of last notification (epoch seconds).
static LAST_NOTIFIED: AtomicU64 = AtomicU64::new(0);

/// Information about a newly arrived message.
pub struct NewMailInfo {
    pub account_id: String,
    /// The folder the message landed in. Carried because the notification's
    /// "Open in Claude" action needs the full `(account_id, folder, uid)`
    /// triple — `account_id` + `uid` alone cannot address a message.
    pub folder: String,
    pub uid: u32,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
    pub subject: Option<String>,
}

/// Called after sync detects new unread mail. Handles notifications + badge.
pub fn on_new_mail(app: &AppHandle, messages: &[NewMailInfo]) {
    if messages.is_empty() {
        return;
    }

    // Always update badge/tooltip regardless of notification debounce
    update_badge(app);

    // Skip notification if window is active (user sees it in-app)
    if is_window_active(app) {
        return;
    }

    // Debounce: skip if we notified recently
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let last = LAST_NOTIFIED.load(Ordering::Relaxed);
    if now.saturating_sub(last) < DEBOUNCE_SECS {
        return;
    }
    LAST_NOTIFIED.store(now, Ordering::Relaxed);

    // Build and send native macOS notifications with click-to-open support
    if messages.len() <= 3 {
        for msg in messages {
            let title = msg
                .from_name
                .as_deref()
                .or(msg.from_email.as_deref())
                .unwrap_or("New Message");
            let body = msg.subject.as_deref().unwrap_or("(no subject)");
            crate::notify_macos::show_notification(
                &msg.account_id,
                &msg.folder,
                msg.uid,
                title,
                body,
            );
        }
    } else {
        crate::notify_macos::show_summary_notification(messages.len());
    }
}

/// Returns true if the given account has notifications enabled.
/// Fails open (returns true) on any DB error so a transient lookup failure
/// never silently swallows legitimate notifications.
///
/// Uses `open_read_conn()` — same pattern as `get_total_unread` — so this is
/// safe to call from inside sync code holding the write mutex.
pub fn account_notify_enabled(app: &AppHandle, account_id: &str) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return true;
    };
    let conn = match state.open_read_conn() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("account_notify_enabled: failed to open read conn: {e}");
            return true;
        }
    };
    match db::accounts::get_by_id(&conn, account_id) {
        Ok(Some(acct)) => acct.notify_enabled,
        Ok(None) => true,
        Err(e) => {
            log::warn!("account_notify_enabled: lookup failed for {account_id}: {e}");
            true
        }
    }
}

/// Update tray tooltip with current unread count.
pub fn update_badge(app: &AppHandle) {
    let count = get_total_unread(app);

    // Update tray tooltip
    if let Some(tray) = app.tray_by_id("main-tray") {
        let tooltip = if count > 0 {
            format!("CXMail — {} unread", count)
        } else {
            "CXMail".to_string()
        };
        let _ = tray.set_tooltip(Some(&tooltip));
    }
}

/// Check if main window is visible and focused.
fn is_window_active(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
        .unwrap_or(false)
}

/// Query total unread inbox messages from DB.
///
/// Uses a fresh read-only connection so this call can never deadlock on the
/// write mutex — critical because it's called from `update_badge` which runs
/// inside `on_new_mail`, which is in turn called from `notify_new_mail` while
/// sync is processing headers.
fn get_total_unread(app: &AppHandle) -> u32 {
    let Some(state) = app.try_state::<AppState>() else {
        return 0;
    };
    match state.open_read_conn() {
        Ok(conn) => db::messages::count_total_inbox_unread(&conn).unwrap_or(0),
        Err(e) => {
            log::warn!("get_total_unread: failed to open read conn: {e}");
            0
        }
    }
}
