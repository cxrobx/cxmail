use crate::error::AppError;
use crate::keychain;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use url::{Host, Url};

const PROVIDER_KEY: &str = "ai:provider";
const MODEL_KEY: &str = "ai:model";
const BASE_URL_KEY: &str = "ai:base_url";
const LEGACY_OPENAI_KEY: &str = "openai:api_key";
const OPENAI_KEY: &str = "ai:openai:key";
const ANTHROPIC_KEY: &str = "ai:anthropic:key";
const COMPATIBLE_KEY: &str = "ai:compatible:key";

const DEFAULT_OPENAI_MODEL: &str = "gpt-5.4-mini";
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5";
const DEFAULT_COMPATIBLE_MODEL: &str = "llama3.2";
const DEFAULT_OPENAI_URL: &str = "https://api.openai.com/v1";
const DEFAULT_ANTHROPIC_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_COMPATIBLE_URL: &str = "http://127.0.0.1:11434/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Openai,
    Anthropic,
    Compatible,
}

impl ProviderKind {
    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "openai" => Ok(Self::Openai),
            "anthropic" => Ok(Self::Anthropic),
            "compatible" => Ok(Self::Compatible),
            _ => Err(AppError::AiService("Unsupported AI provider".to_string())),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Compatible => "compatible",
        }
    }

    fn default_model(self) -> &'static str {
        match self {
            Self::Openai => DEFAULT_OPENAI_MODEL,
            Self::Anthropic => DEFAULT_ANTHROPIC_MODEL,
            Self::Compatible => DEFAULT_COMPATIBLE_MODEL,
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::Openai => DEFAULT_OPENAI_URL,
            Self::Anthropic => DEFAULT_ANTHROPIC_URL,
            Self::Compatible => DEFAULT_COMPATIBLE_URL,
        }
    }

    fn credential_key(self) -> &'static str {
        match self {
            Self::Openai => OPENAI_KEY,
            Self::Anthropic => ANTHROPIC_KEY,
            Self::Compatible => COMPATIBLE_KEY,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    pub provider: ProviderKind,
    pub model: String,
    pub base_url: String,
    pub api_key_configured: bool,
    pub masked_api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProviderSettings {
    pub provider: ProviderKind,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
}

/// What a call returned, and what it cost.
#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// The model the provider says answered, which can differ from the one
    /// asked for when an alias resolves.
    pub model: String,
}

/// Per-call overrides. `model: None` means the account-wide setting.
#[derive(Debug, Clone, Copy)]
pub struct CompleteOpts<'a> {
    pub model: Option<&'a str>,
    pub reasoning_effort: Option<&'a str>,
    pub max_tokens: u32,
    /// `None` omits the field entirely.
    ///
    /// Reasoning models refuse an explicit temperature — gpt-5.6-luna answers
    /// `400 Unsupported value: 'temperature' does not support 0.0 with this
    /// model` — so sending 0.0 for determinism fails EVERY call. Omitting it
    /// costs the determinism and is the only way those models can be used.
    pub temperature: Option<f32>,
}

#[derive(Clone)]
pub struct InferenceClient {
    provider: ProviderKind,
    model: String,
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl InferenceClient {
    pub fn load() -> Result<Self, AppError> {
        let provider = ProviderKind::parse(
            keychain::get_credential(PROVIDER_KEY)?
                .as_deref()
                .unwrap_or("openai"),
        )?;
        let model = keychain::get_credential(MODEL_KEY)?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| provider.default_model().to_string());
        let base_url = keychain::get_credential(BASE_URL_KEY)?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| provider.default_base_url().to_string());
        validate_base_url(&base_url)?;

        let api_key = load_api_key(provider)?;
        if provider != ProviderKind::Compatible && api_key.is_none() {
            return Err(AppError::AiService(format!(
                "{} API key not configured",
                provider_label(provider)
            )));
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::AiService(format!("HTTP client init failed: {e}")))?;

        Ok(Self {
            provider,
            model,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            http,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// A completion plus what it cost.
    ///
    /// `complete` throws the `usage` block away, which is how the first triage
    /// run ended up with a cost that had to be ESTIMATED from prompt lengths
    /// when the provider had reported it exactly. Anything that stores a result
    /// should store what it paid for it.
    pub async fn complete_measured(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        opts: CompleteOpts<'_>,
    ) -> Result<Completion, AppError> {
        let model = opts.model.unwrap_or(&self.model);
        match self.provider {
            // Anthropic has no `reasoning_effort`; sending one is a 400, so the
            // option is dropped rather than translated into a guess.
            ProviderKind::Anthropic => {
                let text = self
                    .complete_anthropic(
                        system_prompt,
                        user_prompt,
                        opts.max_tokens,
                        opts.temperature.unwrap_or(0.7),
                    )
                    .await?;
                Ok(Completion { text, input_tokens: 0, output_tokens: 0, model: model.to_string() })
            }
            ProviderKind::Openai | ProviderKind::Compatible => {
                let body = openai_compatible_body_with(
                    self.provider,
                    model,
                    system_prompt,
                    user_prompt,
                    opts.max_tokens,
                    opts.temperature,
                    opts.reasoning_effort,
                );
                let mut request = self
                    .http
                    .post(format!("{}/chat/completions", self.base_url))
                    .json(&body);
                if let Some(api_key) = &self.api_key {
                    request = request.bearer_auth(api_key);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|e| AppError::AiService(format!("Request failed: {e}")))?;
                let status = response.status();
                let value: serde_json::Value = response
                    .json()
                    .await
                    .map_err(|e| AppError::AiService(format!("Failed to parse response: {e}")))?;
                if !status.is_success() {
                    return Err(provider_http_error(self.provider, status, &value));
                }
                let text = value["choices"][0]["message"]["content"]
                    .as_str()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| AppError::AiService("No text response from model".to_string()))?;
                Ok(Completion {
                    text,
                    input_tokens: value["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                    // Reasoning tokens bill as output and are included here by
                    // the provider, which is the number that matters once
                    // `reasoning_effort` is in play.
                    output_tokens: value["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
                    model: value["model"].as_str().unwrap_or(model).to_string(),
                })
            }
        }
    }

    pub async fn complete(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<String, AppError> {
        match self.provider {
            ProviderKind::Anthropic => {
                self.complete_anthropic(system_prompt, user_prompt, max_tokens, temperature)
                    .await
            }
            ProviderKind::Openai | ProviderKind::Compatible => {
                self.complete_openai_compatible(system_prompt, user_prompt, max_tokens, temperature)
                    .await
            }
        }
    }

    async fn complete_openai_compatible(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<String, AppError> {
        let body = openai_compatible_body(
            self.provider,
            &self.model,
            system_prompt,
            user_prompt,
            max_tokens,
            temperature,
        );
        let mut request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }
        let response = request
            .send()
            .await
            .map_err(|e| AppError::AiService(format!("Request failed: {e}")))?;
        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::AiService(format!("Failed to parse response: {e}")))?;
        if !status.is_success() {
            return Err(provider_http_error(self.provider, status, &value));
        }
        value["choices"][0]["message"]["content"]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| AppError::AiService("No text response from model".to_string()))
    }

    async fn complete_anthropic(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<String, AppError> {
        let body = anthropic_body(
            &self.model,
            system_prompt,
            user_prompt,
            max_tokens,
            temperature,
        );
        let response = self
            .http
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", self.api_key.as_deref().unwrap_or_default())
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::AiService(format!("Request failed: {e}")))?;
        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::AiService(format!("Failed to parse response: {e}")))?;
        if !status.is_success() {
            return Err(provider_http_error(self.provider, status, &value));
        }
        value["content"]
            .as_array()
            .and_then(|blocks| {
                blocks.iter().find_map(|block| {
                    (block["type"].as_str() == Some("text"))
                        .then(|| block["text"].as_str())
                        .flatten()
                })
            })
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| AppError::AiService("No text response from model".to_string()))
    }
}

fn anthropic_body(
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    temperature: f32,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "system": system_prompt,
        "messages": [{ "role": "user", "content": user_prompt }],
        "max_tokens": max_tokens
    });

    // Sonnet 5 rejects non-default sampling parameters and enables adaptive
    // thinking by default. CXMail's email helpers are short, direct generation
    // tasks, so disable thinking and rely on prompt instructions for tone.
    // Opus 4.7+ has the same sampling-parameter restriction.
    if anthropic_uses_fixed_sampling(model) {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
    } else {
        body["temperature"] = serde_json::json!(temperature);
    }
    body
}

fn anthropic_uses_fixed_sampling(model: &str) -> bool {
    model == "claude-sonnet-5"
        || model.starts_with("claude-sonnet-5-")
        || model == "claude-opus-4-7"
        || model.starts_with("claude-opus-4-7-")
        || model == "claude-opus-4-8"
        || model.starts_with("claude-opus-4-8-")
}

fn openai_compatible_body(
    provider: ProviderKind,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    temperature: f32,
) -> serde_json::Value {
    openai_compatible_body_with(
        provider,
        model,
        system_prompt,
        user_prompt,
        max_tokens,
        Some(temperature),
        None,
    )
}

/// Valid `reasoning_effort` values.
///
/// Sent verbatim, so an unrecognized string would be a 400 from the provider
/// rather than a silently ignored setting — which is why the setter validates
/// against this list rather than trusting whatever is stored.
pub const REASONING_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "max"];

#[allow(clippy::too_many_arguments)]
fn openai_compatible_body_with(
    provider: ProviderKind,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt }
        ]
    });
    // Omitted entirely when None — `"temperature": null` is not the same thing
    // and a model that rejects the field rejects the null too.
    if let Some(t) = temperature {
        body["temperature"] = serde_json::json!(t);
    }
    let token_field = if provider == ProviderKind::Compatible {
        "max_tokens"
    } else {
        "max_completion_tokens"
    };
    body[token_field] = serde_json::json!(max_tokens);
    if let Some(effort) = reasoning_effort {
        body["reasoning_effort"] = serde_json::json!(effort);
    }
    body
}

pub fn get_settings() -> Result<ProviderSettings, AppError> {
    let provider = ProviderKind::parse(
        keychain::get_credential(PROVIDER_KEY)?
            .as_deref()
            .unwrap_or("openai"),
    )?;
    let model = keychain::get_credential(MODEL_KEY)?
        .unwrap_or_else(|| provider.default_model().to_string());
    let base_url = keychain::get_credential(BASE_URL_KEY)?
        .unwrap_or_else(|| provider.default_base_url().to_string());
    let api_key = load_api_key(provider)?;
    Ok(ProviderSettings {
        provider,
        model,
        base_url,
        api_key_configured: api_key.is_some(),
        masked_api_key: api_key.as_deref().map(mask_secret),
    })
}

pub fn save_settings(settings: SaveProviderSettings) -> Result<ProviderSettings, AppError> {
    let model = settings.model.trim();
    if model.is_empty() || model.len() > 200 {
        return Err(AppError::AiService(
            "Model name must be between 1 and 200 characters".to_string(),
        ));
    }
    let base_url = settings.base_url.trim().trim_end_matches('/');
    validate_base_url(base_url)?;

    keychain::store_credential(PROVIDER_KEY, settings.provider.as_str())?;
    keychain::store_credential(MODEL_KEY, model)?;
    keychain::store_credential(BASE_URL_KEY, base_url)?;
    if settings.clear_api_key {
        keychain::delete_credential(settings.provider.credential_key())?;
        if settings.provider == ProviderKind::Openai {
            keychain::delete_credential(LEGACY_OPENAI_KEY)?;
        }
    } else if let Some(api_key) = settings.api_key {
        let api_key = api_key.trim();
        if !api_key.is_empty() {
            keychain::store_credential(settings.provider.credential_key(), api_key)?;
        }
    }
    get_settings()
}

fn load_api_key(provider: ProviderKind) -> Result<Option<String>, AppError> {
    let current = keychain::get_credential(provider.credential_key())?;
    if current.is_some() || provider != ProviderKind::Openai {
        return Ok(current.filter(|value| !value.trim().is_empty()));
    }
    keychain::get_credential(LEGACY_OPENAI_KEY)
        .map(|value| value.filter(|key| !key.trim().is_empty()))
}

fn validate_base_url(value: &str) -> Result<(), AppError> {
    let parsed = Url::parse(value)
        .map_err(|_| AppError::AiService("AI provider URL is invalid".to_string()))?;
    let is_https = parsed.scheme() == "https";
    let is_loopback_http = parsed.scheme() == "http"
        && parsed.host().is_some_and(|host| match host {
            Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
            Host::Ipv4(ip) => ip.is_loopback(),
            Host::Ipv6(ip) => ip.is_loopback(),
        });
    if !is_https && !is_loopback_http {
        return Err(AppError::AiService(
            "AI provider URL must use HTTPS; HTTP is allowed only for a local provider".to_string(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(AppError::AiService(
            "AI provider URL cannot contain a query or fragment".to_string(),
        ));
    }
    Ok(())
}

fn mask_secret(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "••••••••".to_string();
    }
    format!(
        "{}••••{}",
        chars.iter().take(4).collect::<String>(),
        chars.iter().skip(chars.len() - 4).collect::<String>()
    )
}

fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Openai => "OpenAI",
        ProviderKind::Anthropic => "Anthropic",
        ProviderKind::Compatible => "OpenAI-compatible provider",
    }
}

fn provider_http_error(
    provider: ProviderKind,
    status: reqwest::StatusCode,
    body: &serde_json::Value,
) -> AppError {
    let detail = body["error"]["message"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .unwrap_or("The provider rejected the request");
    AppError::AiService(format!(
        "{} returned {status}: {detail}",
        provider_label(provider)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_https_for_remote_hosts() {
        assert!(validate_base_url("https://example.com/v1").is_ok());
        assert!(validate_base_url("http://example.com/v1").is_err());
    }

    #[test]
    fn allows_http_for_loopback_providers() {
        assert!(validate_base_url("http://localhost:11434/v1").is_ok());
        assert!(validate_base_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(validate_base_url("http://[::1]:11434/v1").is_ok());
    }

    #[test]
    fn masks_credentials_without_exposing_them() {
        assert_eq!(mask_secret("sk-1234567890"), "sk-1••••7890");
        assert_eq!(mask_secret("short"), "••••••••");
    }

    #[test]
    fn uses_legacy_token_field_for_local_compatible_servers() {
        let compatible = openai_compatible_body(
            ProviderKind::Compatible,
            "llama3.2",
            "system",
            "user",
            100,
            0.2,
        );
        let openai = openai_compatible_body(
            ProviderKind::Openai,
            "gpt-5.4-mini",
            "system",
            "user",
            100,
            0.2,
        );
        assert_eq!(compatible["max_tokens"], 100);
        assert!(compatible.get("max_completion_tokens").is_none());
        assert_eq!(openai["max_completion_tokens"], 100);
    }

    #[test]
    fn sonnet_five_omits_rejected_sampling_parameters() {
        let body = anthropic_body("claude-sonnet-5", "system", "user", 500, 0.7);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["thinking"]["type"], "disabled");
        assert_eq!(body["max_tokens"], 500);
    }

    #[test]
    fn earlier_sonnet_models_keep_temperature_control() {
        let body = anthropic_body("claude-sonnet-4-6", "system", "user", 500, 0.7);
        let temperature = body["temperature"].as_f64().unwrap();
        assert!((temperature - 0.7).abs() < 0.000_001);
        assert!(body.get("thinking").is_none());
    }
}
