use crate::db;
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use tauri::Manager;
use tauri::State;

// Stored outside localStorage so setup can restore the preference before the
// frontend loads. Missing preferences preserve the existing visible default.
pub(crate) fn read_show_in_menu_bar(app_dir: &std::path::Path) -> Result<bool, AppError> {
    match std::fs::read(app_dir.join("menu-bar.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| AppError::Parse(format!("Invalid menu bar preference: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e.into()),
    }
}

fn write_show_in_menu_bar(app_dir: &std::path::Path, visible: bool) -> Result<(), AppError> {
    let pending = app_dir.join("menu-bar.json.tmp");
    std::fs::write(&pending, if visible { "true" } else { "false" })?;
    std::fs::rename(pending, app_dir.join("menu-bar.json"))?;
    Ok(())
}

#[tauri::command]
pub async fn get_show_in_menu_bar(app: tauri::AppHandle) -> Result<bool, AppError> {
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::General(e.to_string()))?;
    read_show_in_menu_bar(&app_dir)
}

#[tauri::command]
pub async fn set_show_in_menu_bar(app: tauri::AppHandle, visible: bool) -> Result<(), AppError> {
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::General(e.to_string()))?;
    let handle = app.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    // Serialize native updates and file writes on the main thread. Wait for
    // completion so the UI only reports success after both have succeeded.
    app.run_on_main_thread(move || {
        let result = (|| {
            let previous = read_show_in_menu_bar(&app_dir)?;
            let tray = handle
                .tray_by_id("main-tray")
                .ok_or_else(|| AppError::NotFound("Menu bar icon not found".into()))?;
            tray.set_visible(visible)
                .map_err(|e| AppError::General(e.to_string()))?;
            if let Err(e) = write_show_in_menu_bar(&app_dir, visible) {
                if let Err(rollback) = tray.set_visible(previous) {
                    log::warn!("Could not restore menu bar icon after save failure: {rollback}");
                }
                return Err(e);
            }
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| AppError::General(e.to_string()))?;
    rx.await.map_err(|e| AppError::General(e.to_string()))?
}

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
    let id = db::identities::insert(&conn, &identity)?;
    // Re-adding an address the user once removed brings it back.
    db::identities::unhide_send_as(&conn, &identity.account_id, &identity.email)?;
    Ok(id)
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

// ─── Send-as addresses ────────────────────────────────────────────────
//
// `identities` rows are the account's ALIASES; the account's own address is
// always implicitly available and is never a row you have to create. See
// `db::identities` for why there is no discovery path (no IMAP extension, and
// the provider APIs that know are outside CXMail's OAuth grant).

/// One address the account may send from, with its display name and signature.
/// Primary first. Drives the compose From picker and the aliases editor.
#[tauri::command]
pub async fn list_send_as(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<db::identities::SendAsAddress>, AppError> {
    let conn = state.db.safe_lock();
    let account = db::accounts::get_by_id(&conn, &account_id)?
        .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?;
    db::identities::send_as_addresses(
        &conn,
        &account.id,
        &account.email,
        account.display_name.as_deref(),
    )
}

/// Remove a non-primary send-as address — configured or found on Sent mail —
/// and keep it removed (`db::identities::remove_send_as`).
#[tauri::command]
pub async fn remove_send_as(
    state: State<'_, AppState>,
    account_id: String,
    email: String,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    let account = db::accounts::get_by_id(&conn, &account_id)?
        .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?;
    db::identities::remove_send_as(&conn, &account.id, &account.email, &email)
}

/// The address a reply to this message should go out FROM: the send-as the
/// original was addressed to, falling back to the account's own address.
///
/// The match runs in Rust so the compose window and the MCP cannot disagree
/// about it (gotcha #36). Reads the local cache only — never an IMAP fetch, so
/// a reply's default can't wait on the network.
#[tauri::command]
pub async fn reply_from_for_message(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<db::identities::SendAsAddress, AppError> {
    let conn = state.db.safe_lock();
    let account = db::accounts::get_by_id(&conn, &account_id)?
        .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?;
    db::identities::reply_from_for_message(
        &conn,
        &account.id,
        &account.email,
        account.display_name.as_deref(),
        &folder,
        uid,
    )
}

/// Addresses this account's mail was delivered to (per the delivery headers)
/// that it cannot yet send as — candidates for the aliases editor. Never added
/// automatically: incoming headers can be forged, so a person confirms.
#[tauri::command]
pub async fn suggest_send_as(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<SendAsSuggestion>, AppError> {
    let conn = state.db.safe_lock();
    let account = db::accounts::get_by_id(&conn, &account_id)?
        .ok_or_else(|| AppError::NotFound("Account not found".to_string()))?;
    Ok(
        db::identities::suggest_aliases(&conn, &account.id, &account.email, 8)?
            .into_iter()
            .map(|(email, message_count)| SendAsSuggestion {
                email,
                message_count,
            })
            .collect(),
    )
}

#[derive(Debug, serde::Serialize)]
pub struct SendAsSuggestion {
    pub email: String,
    pub message_count: u32,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bar_preference_defaults_on_and_persists_both_values() {
        let dir = std::env::temp_dir().join(format!("cxmail-menu-bar-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_show_in_menu_bar(&dir).unwrap());
        write_show_in_menu_bar(&dir, false).unwrap();
        assert!(!read_show_in_menu_bar(&dir).unwrap());
        write_show_in_menu_bar(&dir, true).unwrap();
        assert!(read_show_in_menu_bar(&dir).unwrap());
        assert!(!dir.join("menu-bar.json.tmp").exists());
        std::fs::write(dir.join("menu-bar.json"), "\"false\"").unwrap();
        assert!(read_show_in_menu_bar(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
