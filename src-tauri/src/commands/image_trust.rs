use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use tauri::State;

#[tauri::command]
pub async fn trust_image_sender(
    state: State<'_, AppState>,
    sender_email: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::trusted_senders::add(&conn, &sender_email)?;
    Ok(())
}

#[tauri::command]
pub async fn untrust_image_sender(
    state: State<'_, AppState>,
    sender_email: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::trusted_senders::remove(&conn, &sender_email)?;
    Ok(())
}

#[tauri::command]
pub async fn is_image_sender_trusted(
    state: State<'_, AppState>,
    sender_email: String,
) -> Result<bool, AppError> {
    let conn = state.db.safe_lock();
    db::trusted_senders::is_trusted(&conn, &sender_email)
}

#[tauri::command]
pub async fn list_trusted_image_senders(
    state: State<'_, AppState>,
) -> Result<Vec<String>, AppError> {
    let conn = state.db.safe_lock();
    db::trusted_senders::list(&conn)
}
