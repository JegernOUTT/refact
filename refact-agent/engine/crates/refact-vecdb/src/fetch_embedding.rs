use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use refact_core::llm_types::EmbeddingEndpointStyle;
use refact_core::vecdb_types::EmbeddingModelConfig;

pub async fn get_embedding(
    client: Arc<AMutex<reqwest::Client>>,
    embedding_model: &EmbeddingModelConfig,
    text: Vec<String>,
) -> Result<Vec<Vec<f32>>, String> {
    if embedding_model.embedding_endpoint_style.is_empty()
        && embedding_model.endpoint_style.eq_ignore_ascii_case("hf")
    {
        return Err("HuggingFace endpoint style is no longer supported. Please use 'openai' embedding_endpoint_style with an OpenAI-compatible embedding endpoint.".to_string());
    }

    let style = if embedding_model.embedding_endpoint_style.is_empty() {
        EmbeddingEndpointStyle::from_config(
            &embedding_model.endpoint_style,
            "embedding_endpoint_style",
        )?
    } else {
        EmbeddingEndpointStyle::from_config(
            &embedding_model.embedding_endpoint_style,
            "embedding_endpoint_style",
        )?
    };

    match style {
        EmbeddingEndpointStyle::Openai => get_embedding_openai_style(client, text, embedding_model).await,
        EmbeddingEndpointStyle::OllamaNative => Err(
            "embedding_endpoint_style 'ollama_native' is not supported by this embedding transport yet".to_string(),
        ),
        style => Err(format!(
            "embedding_endpoint_style '{}' is recognized but not supported yet",
            style
        )),
    }
}

const SLEEP_ON_BIG_BATCH: u64 = 9000;
const SLEEP_ON_BATCH_ONE: u64 = 100;

pub async fn get_embedding_with_retries(
    client: Arc<AMutex<reqwest::Client>>,
    embedding_model: &EmbeddingModelConfig,
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
                        tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_ON_BIG_BATCH))
                            .await;
                    } else {
                        tracing::info!(
                            "embedding retry #{} for {} texts: {}",
                            attempt_n,
                            text.len(),
                            e
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_ON_BIG_BATCH))
                            .await;
                    }
                } else {
                    tracing::info!("embedding retry #{} for 1 text: {}", attempt_n, e);
                    tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_ON_BATCH_ONE))
                        .await;
                }
            }
        }
    }
}

async fn get_embedding_openai_style(
    client: Arc<AMutex<reqwest::Client>>,
    text: Vec<String>,
    model: &EmbeddingModelConfig,
) -> Result<Vec<Vec<f32>>, String> {
    if text.is_empty() {
        return Ok(vec![]);
    }
    if model.endpoint.is_empty() {
        return Err("No embedding endpoint configured".to_string());
    }

    #[derive(serde::Serialize)]
    struct EmbeddingsPayload {
        input: Vec<String>,
        model: String,
    }

    let payload = EmbeddingsPayload {
        input: text.clone(),
        model: model.model_name.clone(),
    };

    let client_clone = client.lock().await.clone();
    let mut request = client_clone.post(&model.endpoint).json(&payload);
    if !model.api_key.is_empty() {
        request = request.bearer_auth(&model.api_key);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Failed to send embedding request: {}", e))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Embedding request failed with status {}: {}",
            status, body
        ));
    }

    let response_json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse embedding response: {}", e))?;
    let data = response_json
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or("Missing 'data' in embedding response")?;

    let mut results: Vec<Vec<f32>> = Vec::new();
    for item in data {
        let embedding = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or("Missing 'embedding' in response item")?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();
        results.push(embedding);
    }

    if results.len() != text.len() {
        return Err(format!(
            "Embedding response length mismatch: expected {}, got {}",
            text.len(),
            results.len()
        ));
    }

    Ok(results)
}
