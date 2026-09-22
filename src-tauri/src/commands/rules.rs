use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use tauri::State;

#[tauri::command]
pub async fn list_mail_rules(
    state: State<'_, AppState>,
) -> Result<Vec<db::rules::MailRule>, AppError> {
    let conn = state.db.safe_lock();
    db::rules::list(&conn)
}

#[tauri::command]
pub async fn create_mail_rule(
    state: State<'_, AppState>,
    rule: db::rules::MailRule,
) -> Result<i64, AppError> {
    let conn = state.db.safe_lock();
    db::rules::insert(&conn, &rule)
}

#[tauri::command]
pub async fn update_mail_rule(
    state: State<'_, AppState>,
    rule: db::rules::MailRule,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::rules::update(&conn, &rule)
}

#[tauri::command]
pub async fn delete_mail_rule(
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::rules::delete(&conn, id)
}
