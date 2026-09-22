use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use tauri::State;

// ─── Mail Rules ───────────────────────────────────────────────────────
//
// REMOVED 2026-08-03. An older, narrower rule command set lived here
// (`list_rules` / `create_rule` / `delete_rule`) alongside the one in
// `commands/rules.rs`. Both were registered in lib.rs; nothing called this
// one — the frontend (`src/lib/tauri.ts`) and the MCP both use
// `list_mail_rules` / `create_mail_rule` / `update_mail_rule` /
// `delete_mail_rule`. Do not reintroduce a second set.

// ─── Identities ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_identities(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<db::identities::Identity>, AppError> {
    let conn = state.db.safe_lock();
    db::identities::list_by_account(&conn, &account_id)
}

#[tauri::command]
pub async fn create_identity(
    state: State<'_, AppState>,
    identity: db::identities::Identity,
) -> Result<i64, AppError> {
    let conn = state.db.safe_lock();
    db::identities::insert(&conn, &identity)
}

#[tauri::command]
pub async fn update_identity(
    state: State<'_, AppState>,
    identity: db::identities::Identity,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::identities::update(&conn, &identity)
}

#[tauri::command]
pub async fn delete_identity(state: State<'_, AppState>, id: i64) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::identities::delete(&conn, id)
}

// ─── Background Sync ─────────────────────────────────────────────────

#[tauri::command]
pub async fn install_background_sync() -> Result<(), AppError> {
    crate::launchd::install_launch_agent()
}

#[tauri::command]
pub async fn uninstall_background_sync() -> Result<(), AppError> {
    crate::launchd::uninstall_launch_agent()
}

#[tauri::command]
pub async fn is_background_sync_installed() -> Result<bool, AppError> {
    Ok(crate::launchd::is_installed())
}
