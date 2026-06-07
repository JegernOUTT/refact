use std::sync::Arc;

use tokio::sync::Mutex as AMutex;
use tracing::error;

use refact_core::llm_types::EmbeddingModelRecord;
use refact_core::llm_types::EmbeddingEndpointStyle;

use super::embeddings::get_embedding_openai_style;

pub async fn get_embedding(
    client: Arc<AMutex<reqwest::Client>>,
    embedding_model: &EmbeddingModelRecord,
    text: Vec<String>,
) -> Result<Vec<Vec<f32>>, String> {
    if embedding_model.base.embedding_endpoint_style.is_empty()
        && embedding_model
            .base
            .endpoint_style
            .eq_ignore_ascii_case("hf")
    {
        return Err("HuggingFace endpoint style is no longer supported. Please use 'openai' embedding_endpoint_style with an OpenAI-compatible embedding endpoint.".to_string());
    }

    match embedding_model.base.effective_embedding_endpoint_style()? {
        EmbeddingEndpointStyle::Openai => get_embedding_openai_style(client, text, embedding_model).await,
        EmbeddingEndpointStyle::OllamaNative => Err(
            "embedding_endpoint_style 'ollama_native' is not supported by this embedding transport yet".to_string(),
        ),
        style => {
            error!("Unsupported embedding_endpoint_style: {}", style);
            Err(format!(
                "embedding_endpoint_style '{}' is recognized but not supported yet",
                style
            ))
        }
    }
}

const SLEEP_ON_BIG_BATCH: u64 = 9000;
const SLEEP_ON_BATCH_ONE: u64 = 100;

pub async fn get_embedding_with_retries(
    client: Arc<AMutex<reqwest::Client>>,
    embedding_model: &EmbeddingModelRecord,
    text: Vec<String>,
    max_retries: usize,
) -> Result<Vec<Vec<f32>>, String> {
    let mut attempt_n = 0;
    loop {
        attempt_n += 1;
        match get_embedding(client.clone(), embedding_model, text.clone()).await {
            Ok(embedding) => return Ok(embedding),
            Err(e) => {
                if attempt_n >= max_retries {
                    return Err(e);
                }
                if text.len() > 1 {
                    if e.contains("503") {
                        tracing::info!("normal sleep on 503");
                    } else {
                        tracing::warn!("will retry later, embedding model doesn't work: {}", e);
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_ON_BIG_BATCH))
                        .await;
                } else {
                    tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_ON_BATCH_ONE))
                        .await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::llm_types::BaseModelRecord;

    #[tokio::test]
    async fn hf_endpoint_style_returns_documented_error() {
        let client = Arc::new(AMutex::new(reqwest::Client::new()));
        let embedding_model = EmbeddingModelRecord {
            base: BaseModelRecord {
                endpoint_style: "hf".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = get_embedding(client, &embedding_model, vec!["test".to_string()]).await;

        assert_eq!(
            result.unwrap_err(),
            "HuggingFace endpoint style is no longer supported. Please use 'openai' embedding_endpoint_style with an OpenAI-compatible embedding endpoint."
        );
    }
}
