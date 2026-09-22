use crate::error::AppError;
use crate::keychain;
use serde::Serialize;
use std::time::Duration;

const LICENSE_KEY: &str = "license:cxmail:key";
const VALIDATED_AT_KEY: &str = "license:cxmail:validated_at";
const DEFAULT_LICENSE_URL: &str = "https://cxventures.io/api/products/cxmail/license/validate";
const OFFLINE_GRACE_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseStatus {
    active: bool,
    masked_key: Option<String>,
    validated_at: Option<String>,
    offline_grace: bool,
    development_build: bool,
}

#[derive(serde::Deserialize)]
struct ValidationResponse {
    valid: bool,
}

#[cfg(debug_assertions)]
fn development_status() -> LicenseStatus {
    LicenseStatus {
        active: true,
        masked_key: Some("DEVELOPMENT".to_string()),
        validated_at: None,
        offline_grace: false,
        development_build: true,
    }
}

/// Owner/team escape hatch, resolved at COMPILE time. Owner builds are produced
/// with a non-empty `CXMAIL_OWNER_LICENSE` env var set for that single build
/// (e.g. `CXMAIL_OWNER_LICENSE=owner npm run tauri build`); customer builds never
/// set it, so `option_env!` bakes in `None` and the gate behaves normally. When
/// present, this satisfies the license without a stored key or any network call —
/// so it short-circuits before the keychain read and remote validation in both
/// `get_license_status` and `refresh_license_status`.
///
/// IMPORTANT: never export `CXMAIL_OWNER_LICENSE` in a shell profile or CI that
/// builds customer artifacts — it would unlock those builds too.
#[cfg(not(debug_assertions))]
fn owner_bypass_status() -> Option<LicenseStatus> {
    match option_env!("CXMAIL_OWNER_LICENSE") {
        Some(key) if !key.trim().is_empty() => Some(LicenseStatus {
            active: true,
            masked_key: Some("OWNER".to_string()),
            validated_at: None,
            offline_grace: false,
            development_build: false,
        }),
        _ => None,
    }
}

#[tauri::command]
pub async fn get_license_status() -> Result<LicenseStatus, AppError> {
    #[cfg(debug_assertions)]
    return Ok(development_status());

    #[cfg(not(debug_assertions))]
    {
        if let Some(status) = owner_bypass_status() {
            return Ok(status);
        }
        local_status()
    }
}

#[tauri::command]
pub async fn refresh_license_status() -> Result<LicenseStatus, AppError> {
    #[cfg(debug_assertions)]
    return Ok(development_status());

    #[cfg(not(debug_assertions))]
    {
        if let Some(status) = owner_bypass_status() {
            return Ok(status);
        }
        let Some(license_key) = keychain::get_credential(LICENSE_KEY)? else {
            return Ok(inactive_status());
        };
        match validate_remote(&license_key).await {
            Ok(true) => {
                let now = chrono::Utc::now().to_rfc3339();
                keychain::store_credential(VALIDATED_AT_KEY, &now)?;
                local_status()
            }
            Ok(false) => {
                keychain::delete_credential(LICENSE_KEY)?;
                keychain::delete_credential(VALIDATED_AT_KEY)?;
                Ok(inactive_status())
            }
            Err(error) if validation_is_fresh()? => {
                log::warn!("License validation unavailable; using offline grace: {error}");
                let mut status = local_status()?;
                status.offline_grace = true;
                Ok(status)
            }
            Err(error) => Err(error),
        }
    }
}

#[tauri::command]
pub async fn activate_license(license_key: String) -> Result<LicenseStatus, AppError> {
    let normalized = license_key.trim().to_uppercase();
    if !normalized.starts_with("CXM-") || normalized.len() > 100 {
        return Err(AppError::General("That CXMail license key is not valid".to_string()));
    }
    if !validate_remote(&normalized).await? {
        return Err(AppError::General("That CXMail license key is not active".to_string()));
    }
    keychain::store_credential(LICENSE_KEY, &normalized)?;
    keychain::store_credential(VALIDATED_AT_KEY, &chrono::Utc::now().to_rfc3339())?;
    #[cfg(debug_assertions)]
    return Ok(development_status());
    #[cfg(not(debug_assertions))]
    local_status()
}

#[cfg(not(debug_assertions))]
fn local_status() -> Result<LicenseStatus, AppError> {
    let key = keychain::get_credential(LICENSE_KEY)?;
    let validated_at = keychain::get_credential(VALIDATED_AT_KEY)?;
    Ok(LicenseStatus {
        active: key.is_some() && validation_is_fresh()?,
        masked_key: key.as_deref().map(mask_key),
        validated_at,
        offline_grace: false,
        development_build: false,
    })
}

#[cfg(not(debug_assertions))]
fn inactive_status() -> LicenseStatus {
    LicenseStatus {
        active: false,
        masked_key: None,
        validated_at: None,
        offline_grace: false,
        development_build: false,
    }
}

#[cfg(not(debug_assertions))]
fn validation_is_fresh() -> Result<bool, AppError> {
    let Some(value) = keychain::get_credential(VALIDATED_AT_KEY)? else {
        return Ok(false);
    };
    let parsed = chrono::DateTime::parse_from_rfc3339(&value)
        .map_err(|_| AppError::General("Stored license receipt is invalid".to_string()))?;
    Ok(chrono::Utc::now().timestamp() - parsed.timestamp() <= OFFLINE_GRACE_SECONDS)
}

async fn validate_remote(license_key: &str) -> Result<bool, AppError> {
    let endpoint = option_env!("CXMAIL_LICENSE_API_URL").unwrap_or(DEFAULT_LICENSE_URL);
    if !endpoint.starts_with("https://")
        && !(cfg!(debug_assertions) && endpoint.starts_with("http://127.0.0.1:"))
    {
        return Err(AppError::General("License server URL is not secure".to_string()));
    }
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::General(format!("License client failed: {e}")))?
        .post(endpoint)
        .json(&serde_json::json!({ "licenseKey": license_key }))
        .send()
        .await
        .map_err(|e| AppError::General(format!("Could not reach the CXMail license server: {e}")))?;
    require_success_status(response.status())?;
    response
        .json::<ValidationResponse>()
        .await
        .map(|body| body.valid)
        .map_err(|e| AppError::General(format!("License server response was invalid: {e}")))
}

fn require_success_status(status: reqwest::StatusCode) -> Result<(), AppError> {
    if !status.is_success() {
        return Err(AppError::General(format!(
            "License server returned a temporary error ({})",
            status
        )));
    }
    Ok(())
}

#[allow(dead_code)]
fn mask_key(value: &str) -> String {
    if value.len() < 12 {
        "••••••••".to_string()
    } else {
        format!("{}••••{}", &value[..8], &value[value.len() - 6..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_server_failures_are_transient_not_revocations() {
        assert!(require_success_status(reqwest::StatusCode::TOO_MANY_REQUESTS).is_err());
        assert!(require_success_status(reqwest::StatusCode::SERVICE_UNAVAILABLE).is_err());
        assert!(require_success_status(reqwest::StatusCode::OK).is_ok());
    }
}
