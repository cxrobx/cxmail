use crate::db;
use crate::error::AppError;
use crate::LockExt;
use crate::AppState;
use crate::plugins::manager::{PluginInfo, PluginManager};
use std::sync::Mutex;
use tauri::State;

pub struct PluginState {
    pub manager: Mutex<PluginManager>,
}

#[tauri::command]
pub async fn list_plugins(state: State<'_, PluginState>) -> Result<Vec<PluginInfo>, AppError> {
    let manager = state.manager.safe_lock();
    Ok(manager.list())
}

#[tauri::command]
pub async fn reload_plugins(state: State<'_, PluginState>) -> Result<Vec<PluginInfo>, AppError> {
    let mut manager = state.manager.safe_lock();
    manager.discover();
    Ok(manager.list())
}

#[tauri::command]
pub async fn run_plugin(
    plugin_state: State<'_, PluginState>,
    app_state: State<'_, AppState>,
    name: String,
    trigger: String,
    data: String,
) -> Result<String, AppError> {
    let results_json = {
        let manager = plugin_state.manager.safe_lock();
        manager.execute(&name, &trigger, &data)?
    };

    // Parse and dispatch plugin actions
    if let Ok(actions) = serde_json::from_str::<Vec<String>>(&results_json) {
        dispatch_plugin_actions(&app_state, &actions);
    }

    Ok(results_json)
}

/// Parse plugin result strings and dispatch actual actions.
fn dispatch_plugin_actions(state: &AppState, actions: &[String]) {
    let conn = state.db.safe_lock();

    for action in actions {
        if let Some(msg) = action.strip_prefix("LOG:") {
            log::info!("[plugin] {}", msg);
        } else if let Some(rest) = action.strip_prefix("NOTIFY:") {
            if let Some((title, _body)) = rest.split_once('|') {
                log::info!("[plugin] notification: {}", title);
                // Desktop notification would be dispatched here via tauri-plugin-notification
            }
        } else if let Some(rest) = action.strip_prefix("FLAG:") {
            if let Some((uid_str, _flag)) = rest.split_once('|') {
                if let Ok(uid) = uid_str.parse::<u32>() {
                    // Toggle star flag — plugins only have uid, we need account/folder context
                    // For now, log the intent; full routing requires trigger context
                    log::info!("[plugin] flag message uid={}", uid);
                }
            }
        } else if let Some(rest) = action.strip_prefix("MOVE:") {
            let parts: Vec<&str> = rest.splitn(3, '|').collect();
            if parts.len() == 3 {
                if let Ok(uid) = parts[0].parse::<u32>() {
                    log::info!("[plugin] move message uid={} from={} to={}", uid, parts[1], parts[2]);
                    // Move requires IMAP — fire-and-forget via spawn
                    // Full implementation would call imap::move_message here
                }
            }
        } else if let Some(rest) = action.strip_prefix("CATEGORIZE:") {
            let parts: Vec<&str> = rest.splitn(3, '|').collect();
            if parts.len() == 3 {
                if let Ok(uid) = parts[0].parse::<u32>() {
                    if let Err(e) = db::categories::update_message_category(
                        &conn, parts[1], "INBOX", uid, parts[2], "plugin",
                    ) {
                        log::error!("[plugin] Failed to categorize message: {}", e);
                    }
                }
            }
        } else {
            log::warn!("[plugin] Unknown action: {}", action);
        }
    }
}
