use crate::db;
use crate::db::tracking::{TrackingConfig, TrackingPixel};
use crate::error::AppError;
use crate::AppState;
use crate::LockExt;
use tauri::State;

#[tauri::command]
pub async fn get_tracking_config(
    state: State<'_, AppState>,
) -> Result<TrackingConfig, AppError> {
    let conn = state.db.safe_lock();
    db::tracking::get_config(&conn)
}

#[tauri::command]
pub async fn set_tracking_config(
    state: State<'_, AppState>,
    config: TrackingConfig,
) -> Result<(), AppError> {
    let conn = state.db.safe_lock();
    db::tracking::set_config(&conn, &config)
}

#[tauri::command]
pub async fn list_tracking_pixels(
    state: State<'_, AppState>,
    account_id: Option<String>,
    page: Option<i64>,
) -> Result<Vec<TrackingPixel>, AppError> {
    let conn = state.db.safe_lock();
    let limit = 50i64;
    let offset = page.unwrap_or(0) * limit;
    match account_id {
        Some(id) => db::tracking::list_by_account(&conn, &id, limit, offset),
        None => db::tracking::list_all(&conn, limit, offset),
    }
}

#[tauri::command]
pub async fn get_tracking_pixel(
    state: State<'_, AppState>,
    pixel_code: String,
) -> Result<Option<TrackingPixel>, AppError> {
    let conn = state.db.safe_lock();
    db::tracking::get_by_code(&conn, &pixel_code)
}

#[tauri::command]
pub async fn get_tracking_pixel_for_message(
    state: State<'_, AppState>,
    message_id: String,
) -> Result<Option<TrackingPixel>, AppError> {
    let conn = state.db.safe_lock();
    db::tracking::get_by_message_id(&conn, &message_id)
}

#[tauri::command]
pub async fn sync_tracking_events(
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    // Per-account flags gate pixel INJECTION, not sync — opens keep flowing for
    // already-sent pixels. Sync quietly no-ops unless the service is configured
    // and there is at least one pixel to receive events for.
    let (config, has_pixels) = {
        let conn = state.db.safe_lock();
        let config = db::tracking::get_config(&conn)?;
        let has_pixels: bool = conn
            .query_row("SELECT EXISTS(SELECT 1 FROM tracking_pixels)", [], |r| {
                r.get(0)
            })
            .unwrap_or(false);
        (config, has_pixels)
    };

    let api_key = {
        let conn = state.db.safe_lock();
        db::tracking::get_api_key(&conn)?
    };
    let (Some(service_url), Some(api_key)) = (config.service_url, api_key) else {
        return Ok(());
    };
    if !has_pixels {
        return Ok(());
    }

    let since = config.last_synced.unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
    let url = format!("{}/api/events?since={}", service_url.trim_end_matches('/'), since);

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|e| AppError::General(format!("Failed to sync tracking events: {}", e)))?;

    if !resp.status().is_success() {
        return Err(AppError::General(format!("Tracker returned status: {}", resp.status())));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::General(format!("Failed to parse tracker response: {}", e)))?;

    let conn = state.db.safe_lock();

    if let Some(events) = body["events"].as_array() {
        for event in events {
            let pixel_code = event["pixel_code"].as_str().unwrap_or_default();
            let open_count = event["open_count"].as_i64().unwrap_or(0) as i32;
            let first_open_at = event["first_open_at"].as_str();
            let last_open_at = event["last_open_at"].as_str();

            let _ = db::tracking::update_opens(
                &conn,
                pixel_code,
                open_count,
                first_open_at,
                last_open_at,
            );
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    let _ = db::tracking::update_last_synced(&conn, &now);

    Ok(())
}

#[tauri::command]
pub async fn test_tracker_connection(
    state: State<'_, AppState>,
) -> Result<bool, AppError> {
    let config = {
        let conn = state.db.safe_lock();
        db::tracking::get_config(&conn)?
    };

    let service_url = match &config.service_url {
        Some(url) => url.clone(),
        None => return Ok(false),
    };

    let url = format!("{}/api/health", service_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    match client.get(&url).send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(_) => Ok(false),
    }
}
