use crate::email::inference::{InferenceClient, ProviderKind};
use crate::{error::AppError, keychain};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SETTINGS_KEY: &str = "ai:embedding:settings";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingSettings {
    pub source: String,
    pub model: String,
}
impl Default for EmbeddingSettings {
    fn default() -> Self {
        Self {
            source: "local".into(),
            model: "nomic-embed-text".into(),
        }
    }
}
pub fn settings() -> Result<EmbeddingSettings, AppError> {
    let settings = match keychain::get_credential(SETTINGS_KEY)? {
        Some(s) => serde_json::from_str(&s)
            .map_err(|_| AppError::AiService("Invalid embedding settings".into())),
        None => Ok(EmbeddingSettings::default()),
    }?;
    validate(&settings)?;
    Ok(settings)
}
pub fn save(settings: &EmbeddingSettings) -> Result<(), AppError> {
    validate(settings)?;
    let normalized = EmbeddingSettings {
        source: settings.source.clone(),
        model: settings.model.trim().into(),
    };
    keychain::store_credential(
        SETTINGS_KEY,
        &serde_json::to_string(&normalized).map_err(|e| AppError::General(e.to_string()))?,
    )
}
fn validate(settings: &EmbeddingSettings) -> Result<(), AppError> {
    if !matches!(settings.source.as_str(), "local" | "provider")
        || settings.model.trim().is_empty()
        || settings.model.len() > 200
        || settings.model.contains(char::is_control)
    {
        return Err(AppError::General(
            "Choose an embedding source and model".into(),
        ));
    }
    Ok(())
}

pub struct EmbeddingClient {
    client: InferenceClient,
    model: String,
}
impl EmbeddingClient {
    pub fn load() -> Result<Self, AppError> {
        let s = settings()?;
        let client = if s.source == "local" {
            InferenceClient::local_embeddings()?
        } else {
            InferenceClient::load()?
        };
        if client.provider_kind() == ProviderKind::Anthropic {
            return Err(AppError::AiService("The configured provider has no embeddings endpoint. Choose local embeddings or an OpenAI-compatible provider.".into()));
        }
        Ok(Self {
            client,
            model: s.model,
        })
    }
    pub fn key(&self) -> String {
        // Endpoint paths can contain credentials. Only their fingerprint
        // belongs in SQLite; API keys remain in the credential store.
        format!(
            "{:x}",
            Sha256::digest(format!(
                "{}|{}|v1",
                self.client.embedding_endpoint(),
                self.model
            ))
        )
    }
    pub fn is_local(&self) -> bool {
        self.client.embedding_is_local()
    }
    pub async fn embed(&self, text: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        self.embed_task(text, false).await
    }
    pub async fn embed_query(&self, text: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        self.embed_task(text, true).await
    }
    async fn embed_task(&self, text: &[String], query: bool) -> Result<Vec<Vec<f32>>, AppError> {
        // Nomic's model card requires distinct task prefixes for retrieval.
        let input = task_input(&self.model, text, query);
        self.client.embed(&self.model, &input).await
    }
}

fn task_input(model: &str, text: &[String], query: bool) -> Vec<String> {
    if model.contains("nomic-embed-text") {
        let prefix = if query {
            "search_query: "
        } else {
            "search_document: "
        };
        text.iter().map(|s| format!("{prefix}{s}")).collect()
    } else {
        text.to_vec()
    }
}

/// Some compatible providers reorder `data`; index, count, shape and finite
/// checks are essential before associating a vector with a mail message.
pub fn parse_vectors(value: &serde_json::Value, count: usize) -> Result<Vec<Vec<f32>>, AppError> {
    let error =
        || AppError::AiService("Embedding provider returned invalid or incomplete vectors".into());
    let data = value["data"].as_array().ok_or_else(error)?;
    if data.len() != count {
        return Err(error());
    }
    let mut output: Vec<Option<Vec<f32>>> = vec![None; count];
    let mut dim = None;
    for item in data {
        let idx = item["index"].as_u64().ok_or_else(error)? as usize;
        if idx >= count || output[idx].is_some() {
            return Err(error());
        }
        let vector: Vec<f32> = item["embedding"]
            .as_array()
            .ok_or_else(error)?
            .iter()
            .map(|x| x.as_f64().map(|v| v as f32).ok_or_else(error))
            .collect::<Result<_, _>>()?;
        cxmail_db::db::knowledge::validate_vector(&vector)?;
        if dim.is_some_and(|d| d != vector.len()) {
            return Err(error());
        }
        dim = Some(vector.len());
        output[idx] = Some(vector);
    }
    output.into_iter().map(|v| v.ok_or_else(error)).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_settings_never_fall_through_to_a_remote_provider() {
        assert!(validate(&EmbeddingSettings {
            source: "typo".into(),
            model: "nomic-embed-text".into()
        })
        .is_err());
        assert!(validate(&EmbeddingSettings {
            source: "local".into(),
            model: "  ".into()
        })
        .is_err());
        assert!(validate(&EmbeddingSettings {
            source: "provider".into(),
            model: "bad\nmodel".into()
        })
        .is_err());
    }
    #[test]
    fn retrieval_prefixes_are_model_specific() {
        assert_eq!(
            task_input("nomic-embed-text", &["policy".into()], true),
            vec!["search_query: policy"]
        );
        assert_eq!(
            task_input("nomic-embed-text", &["policy".into()], false),
            vec!["search_document: policy"]
        );
        assert_eq!(
            task_input("text-embedding-3-small", &["policy".into()], true),
            vec!["policy"]
        );
    }
    #[test]
    fn responses_are_reordered_by_index_and_invalid_vectors_rejected() {
        let x = serde_json::json!({"data":[{"index":1,"embedding":[0.0,1.0]},{"index":0,"embedding":[1.0,0.0]}]});
        assert_eq!(parse_vectors(&x, 2).unwrap()[0], vec![1.0, 0.0]);
        assert!(parse_vectors(&x, 1).is_err());
        assert!(parse_vectors(
            &serde_json::json!({"data":[{"index":0,"embedding":[0.0]}]}),
            1
        )
        .is_err());
        assert!(parse_vectors(&serde_json::json!({"data":[{"index":0,"embedding":[1.0]},{"index":0,"embedding":[1.0]}]}),2).is_err());
    }
}
