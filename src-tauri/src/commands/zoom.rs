//! Zoom Server-to-Server credential management, plus the health counts that make
//! a stuck link visible without reading a log.
//!
//! Shape follows `commands::inference`: a status getter, a save that validates
//! before it persists, and a clear.

use crate::db;
use crate::email::zoom;
use crate::error::AppError;
use crate::keychain;
use crate::AppState;
use crate::LockExt;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomStatus {
    pub configured: bool,
    /// A masked account id, so the panel can show *which* Zoom app is wired up.
    /// The client secret is never returned in any form.
    pub masked_account_id: Option<String>,
    /// Links whose Google event is gone. Non-zero means a real Zoom meeting may
    /// be running with nothing pointing at it.
    pub orphan_count: i64,
    /// Links that could not be verified, plus creates that never recorded an id.
    /// Neither is ever auto-deleted, so they need somewhere to be seen.
    pub unverified_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveZoomCredentials {
    pub account_id: String,
    pub client_id: String,
    pub client_secret: String,
}

/// Show only enough of the account id to recognise it.
fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 6 {
        return "••••••".to_string();
    }
    format!(
        "{}••••{}",
        chars[..3].iter().collect::<String>(),
        chars[chars.len() - 3..].iter().collect::<String>()
    )
}

fn status(state: &AppState) -> Result<ZoomStatus, AppError> {
    let credentials = zoom::load_credentials()?;
    let (orphan_count, unverified_count) = {
        let conn = state.db.safe_lock();
        db::zoom::health_counts(&conn)?
    };
    Ok(ZoomStatus {
        configured: credentials.is_some(),
        masked_account_id: credentials.map(|c| mask(&c.account_id)),
        orphan_count,
        unverified_count,
    })
}

#[tauri::command]
pub async fn zoom_connection_status(state: State<'_, AppState>) -> Result<ZoomStatus, AppError> {
    status(state.inner())
}

/// Save the credential triple, **validating it against Zoom first**.
///
/// Validate-then-persist rather than persist-then-test: storing a triple that
/// cannot mint a token leaves the app looking configured while every calendar
/// create fails, and the failure surfaces far from the mistake.
///
/// A partial keychain write is unwound completely. Half-written credentials are
/// worse than none — a new client id paired with the previous secret produces an
/// authentication error that reads like a revoked app.
#[tauri::command]
pub async fn save_zoom_credentials(
    state: State<'_, AppState>,
    credentials: SaveZoomCredentials,
) -> Result<ZoomStatus, AppError> {
    let account_id = credentials.account_id.trim();
    let client_id = credentials.client_id.trim();
    let client_secret = credentials.client_secret.trim();
    if account_id.is_empty() || client_id.is_empty() || client_secret.is_empty() {
        return Err(AppError::ZoomApi(
            0,
            "Account ID, Client ID and Client Secret are all required.".to_string(),
        ));
    }

    let candidate = zoom::ZoomCredentials {
        account_id: account_id.to_string(),
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
    };
    // Prove the triple works before anything is written.
    let (access, expires_at, scope) = zoom::fetch_token(&candidate).await?;

    let writes = [
        (zoom::ACCOUNT_ID_KEY, account_id),
        (zoom::CLIENT_ID_KEY, client_id),
        (zoom::CLIENT_SECRET_KEY, client_secret),
        (zoom::ACCESS_KEY, access.as_str()),
    ];
    let expires = expires_at.to_string();
    let mut failure: Option<AppError> = None;
    for (key, value) in writes {
        if let Err(error) = keychain::store_credential(key, value) {
            failure = Some(error);
            break;
        }
    }
    if failure.is_none() {
        if let Err(error) = keychain::store_credential(zoom::EXPIRES_KEY, &expires) {
            failure = Some(error);
        }
    }
    if let Some(error) = failure {
        for key in zoom::ALL_CREDENTIAL_KEYS {
            if let Err(cleanup) = keychain::delete_credential(key) {
                log::warn!("save_zoom_credentials: could not unwind {key}: {cleanup}");
            }
        }
        return Err(error);
    }

    log::info!("Zoom credentials saved; granted scopes: {scope}");
    status(state.inner())
}

#[tauri::command]
pub async fn clear_zoom_credentials(state: State<'_, AppState>) -> Result<ZoomStatus, AppError> {
    for key in zoom::ALL_CREDENTIAL_KEYS {
        if let Err(error) = keychain::delete_credential(key) {
            log::warn!("clear_zoom_credentials: could not delete {key}: {error}");
        }
    }
    status(state.inner())
}

/// Fetch a fresh token and report the scopes Zoom actually granted.
///
/// The granted scope string is the whole value here: a Server-to-Server app whose
/// scopes were never added to the app's Scopes tab authenticates perfectly and
/// then fails every meeting call, which is indistinguishable from a bad secret
/// until you read this.
#[tauri::command]
pub async fn test_zoom_connection() -> Result<String, AppError> {
    let credentials = zoom::load_credentials()?.ok_or_else(|| {
        AppError::ZoomApi(0, "Zoom is not configured yet.".to_string())
    })?;
    let (_access, _expires, scope) = zoom::fetch_token(&credentials).await?;
    if scope.trim().is_empty() {
        return Ok(
            "Connected, but Zoom reported no scopes. Add meeting:write:meeting, \
             meeting:update:meeting and meeting:delete:meeting on the app's Scopes tab."
                .to_string(),
        );
    }
    Ok(format!("Connected. Granted scopes: {scope}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_never_reveals_a_whole_account_id() {
        assert_eq!(mask("abcdefghijkl"), "abc••••jkl");
        assert_eq!(mask("short"), "••••••");
        assert_eq!(mask(""), "••••••");
    }

    /// Every key this integration owns must be in the unwind list, or a partial
    /// save leaves a stale value behind that reads as a revoked Zoom app.
    #[test]
    fn the_unwind_list_covers_every_key_the_integration_writes() {
        for key in [
            zoom::ACCOUNT_ID_KEY,
            zoom::CLIENT_ID_KEY,
            zoom::CLIENT_SECRET_KEY,
            zoom::ACCESS_KEY,
            zoom::EXPIRES_KEY,
        ] {
            assert!(
                zoom::ALL_CREDENTIAL_KEYS.contains(&key),
                "{key} is written but never unwound"
            );
        }
    }
}
