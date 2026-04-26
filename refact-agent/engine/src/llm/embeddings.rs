use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tracing::info;

use crate::caps::EmbeddingModelRecord;

#[derive(serde::Serialize)]
struct EmbeddingsPayloadOpenAI {
    pub input: Vec<String>,
    pub model: String,
}

#[derive(serde::Deserialize)]
struct EmbeddingsResultOpenAI {
    pub embedding: Vec<f32>,
    pub index: usize,
}

#[derive(serde::Deserialize)]
struct EmbeddingsResultOpenAINoIndex {
    pub embedding: Vec<f32>,
}

pub async fn get_embedding_openai_style(
    client: Arc<AMutex<reqwest::Client>>,
    text: Vec<String>,
    model_rec: &EmbeddingModelRecord,
) -> Result<Vec<Vec<f32>>, String> {
    // Early return for empty batch - avoid unnecessary network request
    if text.is_empty() {
        return Ok(vec![]);
    }

    if model_rec.base.endpoint.is_empty() {
        return Err("No embedding endpoint configured".to_string());
    }

    #[allow(non_snake_case)]
    let B: usize = text.len();
    let payload = EmbeddingsPayloadOpenAI {
        input: text,
        model: model_rec.base.name.to_string(),
    };

    // Clone the client under the lock, then drop the lock before await
    let client_clone = client.lock().await.clone();

    let mut request = client_clone.post(&model_rec.base.endpoint).json(&payload);

    // Only add bearer auth if api_key is non-empty
    if !model_rec.base.api_key.is_empty() {
        request = request.bearer_auth(&model_rec.base.api_key);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Failed to send embedding request: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        if status.as_u16() != 503 {
            info!(
                "get_embedding_openai_style: endpoint={} status={}",
                model_rec.base.endpoint, status
            );
        }
        return Err(format!(
            "get_embedding_openai_style: bad status: {}",
            status
        ));
    }

    let json = response.json::<serde_json::Value>().await.map_err(|err| {
        format!(
            "get_embedding_openai_style: failed to parse response: {}",
            err
        )
    })?;

    let mut result: Vec<Vec<f32>> = vec![vec![]; B];
    match serde_json::from_value::<Vec<EmbeddingsResultOpenAI>>(json["data"].clone()) {
        Ok(unordered) => {
            for ures in unordered.into_iter() {
                let index = ures.index;
                if index >= B {
                    return Err(format!(
                        "get_embedding_openai_style: index {} out of bounds (batch size {})",
                        index, B
                    ));
                }
                result[index] = ures.embedding;
            }
        }
        Err(_) => {
            match serde_json::from_value::<Vec<EmbeddingsResultOpenAINoIndex>>(json["data"].clone())
            {
                Ok(ordered) => {
                    if ordered.len() != B {
                        return Err(format!(
                            "get_embedding_openai_style: response length mismatch: expected {}, got {}",
                            B, ordered.len()
                        ));
                    }
                    for (i, res) in ordered.into_iter().enumerate() {
                        result[i] = res.embedding;
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        "get_embedding_openai_style: failed to parse response structure: {}",
                        err
                    );
                    return Err(format!(
                        "get_embedding_openai_style: failed to parse response: {}",
                        err
                    ));
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::BaseModelRecord;

    fn make_test_model_rec() -> EmbeddingModelRecord {
        EmbeddingModelRecord {
            base: BaseModelRecord {
                name: "test-embedding".to_string(),
                endpoint: "http://localhost:8080/embeddings".to_string(),
                api_key: "test-key".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_empty_batch_returns_empty_vec() {
        let client = Arc::new(AMutex::new(reqwest::Client::new()));
        let model_rec = make_test_model_rec();

        let result = get_embedding_openai_style(client, vec![], &model_rec).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Vec::<Vec<f32>>::new());
    }

    #[tokio::test]
    async fn test_empty_endpoint_returns_error() {
        let client = Arc::new(AMutex::new(reqwest::Client::new()));
        let mut model_rec = make_test_model_rec();
        model_rec.base.endpoint = String::new();

        let result = get_embedding_openai_style(client, vec!["test".to_string()], &model_rec).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No embedding endpoint"));
    }
}
