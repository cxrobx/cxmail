use crate::error::AppError;
use crate::email::inference::InferenceClient;

const MAX_INPUT_CHARS: usize = 4000;

pub async fn summarize_text(client: &InferenceClient, text: &str) -> Result<String, AppError> {
    let truncated = if text.len() > MAX_INPUT_CHARS {
        &text[..MAX_INPUT_CHARS]
    } else {
        text
    };

    client
        .complete(
            "Summarize this email in 1-2 concise sentences. Focus on the key action items or information. Do not use phrases like \"The email\" — just state the content directly.",
            truncated,
            150,
            0.3,
        )
        .await
}
