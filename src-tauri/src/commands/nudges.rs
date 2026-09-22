use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use tauri::State;

/// Follow-up and reply nudges for the message list.
///
/// Read-only and deterministic — the inbox renders this on every load, so it
/// must never reach for the network or an inference provider.
#[tauri::command]
pub async fn list_nudges(state: State<'_, AppState>) -> Result<Vec<db::nudges::Nudge>, AppError> {
    let conn = state.open_read_conn()?;
    db::nudges::list(&conn, &db::nudges::NudgeOptions::default())
}

#[tauri::command]
pub async fn dismiss_nudge(
    state: State<'_, AppState>,
    account_id: String,
    thread_key: String,
    kind: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::nudges::dismiss(&conn, &account_id, &thread_key, &kind)
}
