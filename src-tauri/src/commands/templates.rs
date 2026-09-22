use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use tauri::State;

#[tauri::command]
pub async fn list_templates(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<db::templates::TemplateRow>, AppError> {
    let conn = state.db.safe_lock();
    db::templates::list(&conn, account_id.as_deref())
}

#[tauri::command]
pub async fn create_template(
    state: State<'_, AppState>,
    account_id: Option<String>,
    name: String,
    subject: String,
    html_body: String,
    plain_body: Option<String>,
) -> Result<i64, AppError> {
    let conn = state.db.safe_lock();
    db::templates::insert(&conn, account_id.as_deref(), &name, &subject, &html_body, plain_body.as_deref())
}

#[tauri::command]
pub async fn update_template(
    state: State<'_, AppState>,
    id: i64,
    name: String,
    subject: String,
    html_body: String,
    plain_body: Option<String>,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::templates::update(&conn, id, &name, &subject, &html_body, plain_body.as_deref())
}

#[tauri::command]
pub async fn delete_template(
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::templates::delete(&conn, id)
}
