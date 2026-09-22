use crate::email::inference::{self, ProviderSettings, SaveProviderSettings};
use crate::error::AppError;

#[tauri::command]
pub async fn get_ai_provider_settings() -> Result<ProviderSettings, AppError> {
    inference::get_settings()
}

#[tauri::command]
pub async fn save_ai_provider_settings(
    settings: SaveProviderSettings,
) -> Result<ProviderSettings, AppError> {
    inference::save_settings(settings)
}

#[tauri::command]
pub async fn test_ai_provider() -> Result<String, AppError> {
    let client = inference::InferenceClient::load()?;
    client
        .complete(
            "Reply with exactly: Connected",
            "Test the configured CXMail inference provider.",
            20,
            0.0,
        )
        .await
}

// ── External writer (agent draft writing via Antigravity `agy`) ──
//
// The model used when an MCP draft call passes `instruction` and omits
// `writer_model`. Precedence: per-call `writer_model` > this setting > the
// built-in default. Stored under `ai:writer:model` in the same credential
// store as the inference `ai:*` keys, which is what makes it visible to the
// loose MCP binary (credentials.dat first, Keychain fallback).

use crate::email::external_writer;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterSettings {
    /// The stored override, if any (None = using the built-in default).
    pub model: Option<String>,
    /// What a call omitting `writer_model` will actually use.
    pub effective_model: String,
    pub builtin_default: String,
}

fn writer_settings_now() -> WriterSettings {
    let model = external_writer::configured_model();
    WriterSettings {
        effective_model: external_writer::effective_default_from(model.clone()),
        model,
        builtin_default: external_writer::DEFAULT_WRITER_MODEL.to_string(),
    }
}

#[tauri::command]
pub async fn get_writer_settings() -> Result<WriterSettings, AppError> {
    Ok(writer_settings_now())
}

#[tauri::command]
pub async fn save_writer_model(model: Option<String>) -> Result<WriterSettings, AppError> {
    external_writer::save_configured_model(model.as_deref().unwrap_or(""))?;
    Ok(writer_settings_now())
}

#[derive(serde::Serialize)]
pub struct WriterModel {
    pub id: String,
    pub label: String,
}

#[tauri::command]
pub async fn list_writer_models() -> Result<Vec<WriterModel>, AppError> {
    Ok(external_writer::list_models()
        .await?
        .into_iter()
        .map(|(id, label)| WriterModel { id, label })
        .collect())
}
