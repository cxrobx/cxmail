use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use serde::Deserialize;
use tauri::State;

use super::messages::MessagePage;

#[derive(Deserialize)]
pub struct RuleInput {
    pub field: String,
    pub operator: String,
    pub value: String,
}

#[tauri::command]
pub async fn list_inbox_groups(
    state: State<'_, AppState>,
) -> Result<Vec<db::inbox_groups::InboxGroupWithDetails>, AppError> {
    let conn = state.open_read_conn()?;
    db::inbox_groups::list_groups(&conn)
}

#[tauri::command]
pub async fn create_inbox_group(
    state: State<'_, AppState>,
    name: String,
    color: String,
    icon: String,
    rules: Vec<RuleInput>,
    account_ids: Vec<String>,
) -> Result<i64, AppError> {
    let conn = state.db.safe_lock();
    let id = db::inbox_groups::create_group(&conn, &name, &color, &icon)?;
    let rule_tuples: Vec<(String, String, String)> = rules
        .into_iter()
        .map(|r| (r.field, r.operator, r.value))
        .collect();
    db::inbox_groups::set_rules(&conn, id, &rule_tuples)?;
    db::inbox_groups::set_account_ids(&conn, id, &account_ids)?;
    Ok(id)
}

#[tauri::command]
pub async fn update_inbox_group(
    state: State<'_, AppState>,
    id: i64,
    name: String,
    color: String,
    icon: String,
    rules: Vec<RuleInput>,
    account_ids: Vec<String>,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::inbox_groups::update_group(&conn, id, &name, &color, &icon)?;
    let rule_tuples: Vec<(String, String, String)> = rules
        .into_iter()
        .map(|r| (r.field, r.operator, r.value))
        .collect();
    db::inbox_groups::set_rules(&conn, id, &rule_tuples)?;
    db::inbox_groups::set_account_ids(&conn, id, &account_ids)?;
    Ok(())
}

#[tauri::command]
pub async fn delete_inbox_group(
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::inbox_groups::delete_group(&conn, id)
}

#[tauri::command]
pub async fn fetch_inbox_group_messages(
    state: State<'_, AppState>,
    group_id: i64,
    page: u32,
    page_size: u32,
    unread_only: Option<bool>,
) -> Result<MessagePage, AppError> {
    let conn = state.open_read_conn()?;
    let (messages, total) = db::inbox_groups::list_messages_for_group(
        &conn,
        group_id,
        page,
        page_size,
        unread_only.unwrap_or(false),
    )?;
    let has_more = (page + 1) * page_size < total;
    Ok(MessagePage {
        messages,
        total,
        page,
        page_size,
        has_more,
    })
}
