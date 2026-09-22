use crate::db;
use crate::db::needs_you::MessageRef;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use tauri::State;

#[tauri::command]
pub async fn list_needs_you(
    state: State<'_, AppState>,
) -> Result<Vec<db::needs_you::NeedsYouItem>, AppError> {
    let conn = state.open_read_conn()?;
    db::needs_you::list(
        &conn,
        100,
        // Only `on` lets a stored verdict reach the queue. This is a local
        // table read either way — the queue never makes a network call.
        cxmail_email::email::triage_gate::effective_mode().may_surface(),
    )
}

#[tauri::command]
pub async fn dismiss_needs_you(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::needs_you::dismiss(&conn, &account_id, &folder, uid)
}

/// Dismiss every message behind a collapsed row (repeat alerts, thread
/// siblings). Dismissing only the representative would let the row reappear on
/// the next refresh, represented by its next member.
#[tauri::command]
pub async fn dismiss_needs_you_group(
    state: State<'_, AppState>,
    members: Vec<MessageRef>,
) -> Result<usize, AppError> {
    if members.is_empty() {
        return Err(AppError::General("No messages to dismiss".to_string()));
    }
    let mut conn = state.db.safe_lock();
    db::needs_you::dismiss_group(&mut conn, &members)
}
