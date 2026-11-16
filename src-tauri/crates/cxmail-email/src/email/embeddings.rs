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
    if !matches!(settings.source.as_str(), "local" | "local-vault" | "provider")
        || settings.model.trim().is_empty()
        || settings.model.len() > 200
        || settings.model.contains(char::is_control)
        || (settings.source == "local-vault" && settings.model != "embeddinggemma-2")
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
    local_vault: bool,
}
impl EmbeddingClient {
    pub fn load() -> Result<Self, AppError> {
        let s = settings()?;
        let client = if s.source != "provider" {
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
            local_vault: s.source == "local-vault",
        })
    }
    pub fn key(&self) -> String {
        // Endpoint paths can contain credentials. Only their fingerprint
        // belongs in SQLite; API keys remain in the credential store.
        format!(
            "{:x}",
            Sha256::digest(format!(
                "{}|{}|v1",
                if self.local_vault { "http://127.0.0.1:11436/api/embed" } else { self.client.embedding_endpoint() },
                self.model
            ))
        )
    }
    /// Conservative model-specific null-match gate. The fiction-only local
    /// relevance probe (scripts/eval-mail-retrieval.py) measures both positives
    /// and hard negatives; cosine scales are not interchangeable across models.
    pub fn minimum_similarity(&self) -> f32 {
        if self.model == "embeddinggemma-2" { 0.65 } else { 0.35 }
    }
    pub fn is_local(&self) -> bool {
        self.local_vault || self.client.embedding_is_local()
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
        if self.local_vault {
            return self.embed_local_vault(&input).await;
        }
        self.client.embed(&self.model, &input).await
    }
    async fn embed_local_vault(&self, input: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        if input.is_empty() || input.len() > super::inference::MAX_EMBED_BATCH || input.iter().any(|s|s.trim().is_empty() || s.chars().count()>2_000) {
            return Err(AppError::AiService("Invalid embedding batch".into()));
        }
        // Both the desktop's 32-passage caller and the companion must honor
        // the same GPU bound. Sixteen long passages exhausted the live model;
        // two were measured to fit. Preserve provider order across sub-batches.
        let mut output: Vec<Vec<f32>> = Vec::new();
        for batch in input.chunks(2) {
            let vectors=self.local_vault_batch(batch).await?;
            if output.first().is_some_and(|first| vectors.iter().any(|v|v.len()!=first.len())) {
                return Err(AppError::AiService("Inconsistent local embedding dimensions".into()));
            }
            output.extend(vectors);
        }
        Ok(output)
    }
    async fn local_vault_batch(&self,input: &[String]) -> Result<Vec<Vec<f32>>,AppError> {
        use futures::StreamExt;
        let response = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none()).build()
            .map_err(|e| AppError::AiService(e.to_string()))?
            .post("http://127.0.0.1:11436/api/embed")
            .json(&serde_json::json!({"model": self.model, "input": input})).send().await
            .map_err(|e| AppError::AiService(e.to_string()))?;
        if !response.status().is_success() { return Err(if response.status().is_client_error() { AppError::Parse("Local embedding input rejected".into()) } else { AppError::AiService(format!("Local embedding service returned {}",response.status())) }); }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(part) = stream.next().await {
            let part = part.map_err(|e| AppError::AiService(e.to_string()))?;
            if bytes.len() + part.len() > 4 * 1024 * 1024 { return Err(AppError::AiService("Embedding response too large".into())); }
            bytes.extend_from_slice(&part);
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| AppError::AiService(e.to_string()))?;
        parse_local_vectors(&value, input.len())
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
    } else if model == "embeddinggemma-2" {
        text.iter().map(|s| if query { format!("task: search result | query: {s}") } else { format!("title: Email | text: {s}") }).collect()
    } else {
        text.to_vec()
    }
}

// The local service implements Ollama's API, not OpenAI's. Keep count,
// dimension, finite and zero-vector validation identical in both adapters.
fn parse_local_vectors(value: &serde_json::Value, count: usize) -> Result<Vec<Vec<f32>>, AppError> {
    let values = value["embeddings"].as_array().ok_or_else(|| AppError::AiService("Missing local vectors".into()))?;
    let data: Vec<_> = values.iter().enumerate().map(|(index, embedding)| serde_json::json!({"index": index, "embedding": embedding})).collect();
    parse_vectors(&serde_json::json!({"data": data}), count)
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
    #[tokio::test]
    #[ignore = "Live local-model GPU regression; fiction only"]
    async fn local_vault_long_batch_is_split_before_gpu_submission() {
        let client=EmbeddingClient { client: InferenceClient::local_embeddings().unwrap(), model:"embeddinggemma-2".into(), local_vault:true };
        let text="Fictional meeting notes: Alex will review the proposal and send the revised budget next week. ".repeat(20).chars().take(1_600).collect::<String>();
        let vectors=client.embed(&vec![text;16]).await.unwrap();
        assert_eq!(vectors.len(),16);
        assert!(vectors.iter().all(|v|v.len()==768 && v.iter().all(|x|x.is_finite())));
    }
    #[test]
    fn local_vault_api_and_task_profile_are_explicit() {
        // Contract observed in vault-mcp/embed_server/server.py do_POST and
        // vault_mcp/embeddings.py PROFILES, not inferred from our client.
        assert_eq!(task_input("embeddinggemma-2", &["hotel receipt".into()], true), vec!["task: search result | query: hotel receipt"]);
        assert_eq!(task_input("embeddinggemma-2", &["Paid for hotel".into()], false), vec!["title: Email | text: Paid for hotel"]);
        assert!(parse_local_vectors(&serde_json::json!({"embeddings":[[1.0,0.0]]}),1).is_ok());
        assert!(parse_local_vectors(&serde_json::json!({"embeddings":[[1.0,0.0]]}),2).is_err());
        assert!(parse_local_vectors(&serde_json::json!({"embeddings":[[0.0,0.0]]}),1).is_err());
        assert!(parse_local_vectors(&serde_json::json!({"embeddings":[[1.0],[1.0,0.0]]}),2).is_err());
    }
    #[test]
    fn invalid_settings_never_fall_through_to_a_remote_provider() {
        assert!(validate(&EmbeddingSettings { source: "local-vault".into(), model: "unrecognized-model".into() }).is_err());
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
