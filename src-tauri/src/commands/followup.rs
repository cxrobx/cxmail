use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use serde::Serialize;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct FollowupReminderView {
    pub id: i64,
    pub account_id: String,
    pub sent_message_id: String,
    pub to_email: String,
    pub subject: Option<String>,
    pub remind_at: String,
    pub status: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn create_followup_reminder(
    state: State<'_, AppState>,
    account_id: String,
    sent_message_id: String,
    sender_email: String,
    to_email: String,
    subject: Option<String>,
    remind_at: String,
) -> Result<i64, AppError> {
    let conn = state.db.safe_lock();
    db::followup::insert(
        &conn,
        &account_id,
        &sent_message_id,
        &sender_email,
        &to_email,
        subject.as_deref(),
        &remind_at,
    )
}

#[tauri::command]
pub async fn cancel_followup_reminder(
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::followup::update_status(&conn, id, "cancelled")
}

#[tauri::command]
pub async fn dismiss_followup_reminder(
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::followup::update_status(&conn, id, "dismissed")
}

#[tauri::command]
pub async fn list_followup_reminders(
    state: State<'_, AppState>,
) -> Result<Vec<FollowupReminderView>, AppError> {
    let conn = state.db.safe_lock();
    let rows = db::followup::list_pending(&conn)?;
    Ok(rows
        .into_iter()
        .map(|r| FollowupReminderView {
            id: r.id,
            account_id: r.account_id,
            sent_message_id: r.sent_message_id,
            to_email: r.to_email,
            subject: r.subject,
            remind_at: r.remind_at,
            status: r.status,
            created_at: r.created_at,
        })
        .collect())
}
