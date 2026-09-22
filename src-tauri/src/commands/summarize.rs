use crate::db;
use crate::email;
use crate::error::AppError;
use crate::keychain;
use crate::AppState;
use crate::LockExt;
use tauri::State;

const API_KEY_NAME: &str = "openai:api_key";

#[tauri::command]
pub async fn summarize_message(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uid: u32,
) -> Result<String, AppError> {
    // Check cache first
    {
        let conn = state.db.safe_lock();
        if let Some(summary) = db::messages::get_summary(&conn, &account_id, &folder, uid)? {
            return Ok(summary);
        }
    }

    // Get the message body
    let plain_text = {
        let conn = state.db.safe_lock();
        db::messages::get_body(&conn, &account_id, &folder, uid)?
            .and_then(|b| b.plain_text)
            .ok_or_else(|| AppError::NotFound("Message body not found".to_string()))?
    };

    let client = email::inference::InferenceClient::load()?;
    let summary = email::summarize::summarize_text(&client, &plain_text).await?;

    // Cache the result
    {
        let conn = state.db.safe_lock();
        db::messages::save_summary(
            &conn,
            &account_id,
            &folder,
            uid,
            &summary,
            client.model(),
        )?;
    }

    Ok(summary)
}

#[tauri::command]
pub async fn get_ai_api_key() -> Result<Option<String>, AppError> {
    let key = keychain::get_credential(API_KEY_NAME)?;
    // Return whether a key exists, not the key itself
    Ok(key.map(|k| {
        if k.len() > 8 {
            format!("{}...{}", &k[..4], &k[k.len() - 4..])
        } else {
            "****".to_string()
        }
    }))
}

#[tauri::command]
pub async fn save_ai_api_key(api_key: String) -> Result<(), AppError> {
    if api_key.is_empty() {
        keychain::delete_credential(API_KEY_NAME)?;
    } else {
        keychain::store_credential(API_KEY_NAME, &api_key)?;
    }
    Ok(())
}
