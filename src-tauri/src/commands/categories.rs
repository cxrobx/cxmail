use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use std::collections::HashMap;
use tauri::State;

#[tauri::command]
pub async fn get_category_counts(
    state: State<'_, AppState>,
    account_id: Option<String>,
    account_ids: Option<Vec<String>>,
) -> Result<HashMap<String, u32>, AppError> {
    let conn = state.open_read_conn()?;
    match (account_id, account_ids) {
        (Some(id), _) => db::categories::count_unread_by_category(&conn, &id, "INBOX"),
        (_, Some(ids)) if !ids.is_empty() => db::categories::count_unread_by_category_for_accounts(&conn, &ids),
        _ => db::categories::count_unread_by_category_unified(&conn),
    }
}

#[tauri::command]
pub async fn set_message_category(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
    category: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();

    // Update this message
    db::categories::update_message_category(&conn, &account_id, &folder, uid, &category, "user")?;

    // Learn the sender preference (by domain) for future messages
    let from_email: Option<String> = conn
        .query_row(
            "SELECT from_email FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
            rusqlite::params![account_id, folder, uid],
            |row| row.get(0),
        )
        .ok();

    if let Some(email) = from_email {
        if let Some(domain) = email.split('@').nth(1) {
            db::categories::set_sender_category(&conn, domain, &category)?;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn set_messages_category(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uids: Vec<u32>,
    category: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();

    // Update all messages and collect unique sender domains for learning
    let mut learned_domains = std::collections::HashSet::new();
    for &uid in &uids {
        db::categories::update_message_category(&conn, &account_id, &folder, uid, &category, "user")?;

        let from_email: Option<String> = conn
            .query_row(
                "SELECT from_email FROM messages WHERE account_id = ?1 AND folder_name = ?2 AND uid = ?3",
                rusqlite::params![account_id, folder, uid],
                |row| row.get(0),
            )
            .ok();

        if let Some(email) = from_email {
            if let Some(domain) = email.split('@').nth(1) {
                learned_domains.insert(domain.to_string());
            }
        }
    }

    // Learn sender preferences for unique domains
    for domain in learned_domains {
        db::categories::set_sender_category(&conn, &domain, &category)?;
    }

    Ok(())
}
