use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use serde::Serialize;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct SnoozedMessageView {
    pub uid: u32,
    pub account_id: String,
    pub folder_name: String,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
    pub date: String,
    pub snippet: Option<String>,
    pub wake_at: String,
    pub is_read: bool,
    pub is_flagged: bool,
    pub has_attachments: bool,
}

#[tauri::command]
pub async fn snooze_message(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
    wake_at: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::snoozed::insert(&conn, &account_id, &folder, uid, &wake_at)?;
    Ok(())
}

#[tauri::command]
pub async fn unsnooze_message(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::snoozed::delete_by_message(&conn, &account_id, &folder, uid)?;
    Ok(())
}

#[tauri::command]
pub async fn list_snoozed_messages(
    state: State<'_, AppState>,
) -> Result<Vec<SnoozedMessageView>, AppError> {
    let conn = state.db.safe_lock();
    let snoozed = db::snoozed::list_all(&conn)?;

    let mut views = Vec::new();
    for s in snoozed {
        let mut stmt = conn.prepare(
            "SELECT subject, from_name, from_email, date, snippet, is_read, is_flagged, has_attachments
             FROM messages
             WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
        ).map_err(AppError::Database)?;

        let mut rows = stmt.query_map(
            rusqlite::params![s.account_id, s.folder_name, s.uid],
            |row| {
                Ok(SnoozedMessageView {
                    uid: s.uid,
                    account_id: s.account_id.clone(),
                    folder_name: s.folder_name.clone(),
                    subject: row.get(0)?,
                    from_name: row.get(1)?,
                    from_email: row.get(2)?,
                    date: row.get(3)?,
                    snippet: row.get(4)?,
                    wake_at: s.wake_at.clone(),
                    is_read: row.get::<_, i32>(5)? != 0,
                    is_flagged: row.get::<_, i32>(6)? != 0,
                    has_attachments: row.get::<_, i32>(7)? != 0,
                })
            },
        ).map_err(AppError::Database)?;

        if let Some(Ok(view)) = rows.next() {
            views.push(view);
        }
    }

    Ok(views)
}
