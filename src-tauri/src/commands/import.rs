use crate::email::{import, spam};
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use std::path::PathBuf;
use tauri::State;

// ─── Thunderbird Import ───────────────────────────────────────────────

#[tauri::command]
pub async fn import_mbox(
    state: State<'_, AppState>,
    account_id: String,
    folder_name: String,
    mbox_path: String,
) -> Result<u32, AppError> {
    let conn = state.db.safe_lock();
    import::import_mbox(&conn, &account_id, &folder_name, &PathBuf::from(mbox_path))
}

// ─── Spam Filter ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn spam_classify(
    _state: State<'_, AppState>,
    text: String,
) -> Result<f64, AppError> {
    let app_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.cxmail.app");
    let filter_path = app_dir.join("spam_filter.json");
    let filter = spam::SpamFilter::load(&filter_path);
    Ok(filter.classify(&text))
}

#[tauri::command]
pub async fn spam_train(
    _state: State<'_, AppState>,
    text: String,
    is_spam: bool,
) -> Result<(), AppError> {
    let app_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.cxmail.app");
    let filter_path = app_dir.join("spam_filter.json");
    let mut filter = spam::SpamFilter::load(&filter_path);

    if is_spam {
        filter.train_spam(&text);
    } else {
        filter.train_ham(&text);
    }

    filter.save(&filter_path)
        .map_err(|e| AppError::General(format!("Failed to save spam filter: {}", e)))?;
    Ok(())
}
