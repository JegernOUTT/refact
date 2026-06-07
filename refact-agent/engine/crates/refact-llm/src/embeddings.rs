use std::sync::Arc;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::json;
use tokio::sync::Mutex as AMutex;
use tracing::info;

use refact_core::llm_types::{EmbeddingEndpointStyle, EmbeddingModelRecord};

use crate::adapter::insert_extra_headers;

#[derive(serde::Serialize)]
struct EmbeddingsPayloadOpenAI {
    pub input: Vec<String>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,
}

#[derive(serde::Serialize)]
struct EmbeddingsPayloadOllama {
    pub input: Vec<String>,
    pub model: String,
}

#[derive(serde::Deserialize)]
struct EmbeddingsResponseOpenAI {
    data: Vec<EmbeddingsResultOpenAI>,
}

#[derive(serde::Deserialize)]
struct EmbeddingsResultOpenAI {
    embedding: Vec<f32>,
    index: Option<usize>,
}

#[derive(serde::Deserialize)]
struct EmbeddingsResponseOllama {
    embeddings: Vec<Vec<f32>>,
}

pub async fn get_embedding_openai_style(
    client: Arc<AMutex<reqwest::Client>>,
    text: Vec<String>,
    model_rec: &EmbeddingModelRecord,
) -> Result<Vec<Vec<f32>>, String> {
    get_embedding_for_style(client, text, model_rec, EmbeddingEndpointStyle::Openai).await
}

pub async fn get_embedding_ollama_native_style(
    client: Arc<AMutex<reqwest::Client>>,
    text: Vec<String>,
    model_rec: &EmbeddingModelRecord,
) -> Result<Vec<Vec<f32>>, String> {
    get_embedding_for_style(
        client,
        text,
        model_rec,
        EmbeddingEndpointStyle::OllamaNative,
    )
    .await
}

pub async fn get_embedding_for_style(
    client: Arc<AMutex<reqwest::Client>>,
    text: Vec<String>,
    model_rec: &EmbeddingModelRecord,
    style: EmbeddingEndpointStyle,
) -> Result<Vec<Vec<f32>>, String> {
    if text.is_empty() {
        return Ok(vec![]);
    }
    if model_rec.base.endpoint.is_empty() {
        return Err(format!(
            "embedding_endpoint_style '{}': no embedding endpoint configured",
            style
        ));
    }

    let payload = match style {
        EmbeddingEndpointStyle::Openai => json!(EmbeddingsPayloadOpenAI {
            input: text.clone(),
            model: model_rec.base.name.clone(),
            dimensions: model_rec.dimensions,
        }),
        EmbeddingEndpointStyle::OllamaNative => json!(EmbeddingsPayloadOllama {
            input: text.clone(),
            model: model_rec.base.name.clone(),
        }),
        _ => {
            return Err(format!(
                "embedding_endpoint_style '{}' is recognized but not supported yet",
                style
            ));
        }
    };

    let client_clone = client.lock().await.clone();
    let response = client_clone
        .post(&model_rec.base.endpoint)
        .headers(embedding_headers(model_rec, style)?)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            format!(
                "embedding_endpoint_style '{}': retryable network error: {}",
                style, e
            )
        })?;

    let status = response.status();
    if !status.is_success() {
        if status.as_u16() != 503 {
            info!(
                "embedding request: endpoint={} embedding_endpoint_style={} status={}",
                model_rec.base.endpoint, style, status
            );
        }
        let body = response.text().await.unwrap_or_default();
        let retryable = is_retryable_status(status.as_u16());
        let retry_hint = if retryable { "retryable " } else { "" };
        return Err(format!(
            "embedding_endpoint_style '{}': {}bad status {}: {}",
            style, retry_hint, status, body
        ));
    }

    let json = response.json::<serde_json::Value>().await.map_err(|err| {
        format!(
            "embedding_endpoint_style '{}': malformed provider payload: {}",
            style, err
        )
    })?;

    match style {
        EmbeddingEndpointStyle::Openai => parse_openai_response(json, text.len(), model_rec),
        EmbeddingEndpointStyle::OllamaNative => parse_ollama_response(json, text.len(), model_rec),
        _ => Err(format!(
            "embedding_endpoint_style '{}' is recognized but not supported yet",
            style
        )),
    }
}

fn embedding_headers(
    model_rec: &EmbeddingModelRecord,
    style: EmbeddingEndpointStyle,
) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let bearer = if !model_rec.base.auth_token.is_empty() {
        &model_rec.base.auth_token
    } else {
        &model_rec.base.api_key
    };
    if !bearer.is_empty() {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", bearer)).map_err(|e| {
                format!(
                    "embedding_endpoint_style '{}': invalid authorization header: {}",
                    style, e
                )
            })?,
        );
    }
    insert_extra_headers(&mut headers, &model_rec.base.extra_headers);
    Ok(headers)
}

fn parse_openai_response(
    json: serde_json::Value,
    expected_count: usize,
    model_rec: &EmbeddingModelRecord,
) -> Result<Vec<Vec<f32>>, String> {
    let response: EmbeddingsResponseOpenAI = serde_json::from_value(json).map_err(|err| {
        format!(
            "embedding_endpoint_style 'openai': malformed provider payload: {}",
            err
        )
    })?;
    if response.data.len() != expected_count {
        return Err(format!(
            "embedding_endpoint_style 'openai': validation error: response length mismatch: expected {}, got {}",
            expected_count,
            response.data.len()
        ));
    }

    let has_any_index = response.data.iter().any(|item| item.index.is_some());
    let has_all_indices = response.data.iter().all(|item| item.index.is_some());
    if has_any_index && !has_all_indices {
        return Err(
            "embedding_endpoint_style 'openai': validation error: missing index in indexed response"
                .to_string(),
        );
    }

    if has_all_indices {
        let mut results: Vec<Option<Vec<f32>>> = vec![None; expected_count];
        for item in response.data {
            let index = item.index.unwrap();
            if index >= expected_count {
                return Err(format!(
                    "embedding_endpoint_style 'openai': validation error: index {} out of range for response length {}",
                    index, expected_count
                ));
            }
            if results[index].is_some() {
                return Err(format!(
                    "embedding_endpoint_style 'openai': validation error: duplicate index {}",
                    index
                ));
            }
            validate_vector("openai", &item.embedding, model_rec)?;
            results[index] = Some(item.embedding);
        }
        results
            .into_iter()
            .enumerate()
            .map(|(index, embedding)| {
                embedding.ok_or_else(|| {
                    format!(
                        "embedding_endpoint_style 'openai': validation error: missing index {}",
                        index
                    )
                })
            })
            .collect()
    } else {
        response
            .data
            .into_iter()
            .map(|item| {
                validate_vector("openai", &item.embedding, model_rec)?;
                Ok(item.embedding)
            })
            .collect()
    }
}

fn parse_ollama_response(
    json: serde_json::Value,
    expected_count: usize,
    model_rec: &EmbeddingModelRecord,
) -> Result<Vec<Vec<f32>>, String> {
    let response: EmbeddingsResponseOllama = serde_json::from_value(json).map_err(|err| {
        format!(
            "embedding_endpoint_style 'ollama_native': malformed provider payload: {}",
            err
        )
    })?;
    if response.embeddings.len() != expected_count {
        return Err(format!(
            "embedding_endpoint_style 'ollama_native': validation error: response length mismatch: expected {}, got {}",
            expected_count,
            response.embeddings.len()
        ));
    }
    response
        .embeddings
        .into_iter()
        .map(|embedding| {
            validate_vector("ollama_native", &embedding, model_rec)?;
            Ok(embedding)
        })
        .collect()
}

fn validate_vector(
    style: &str,
    embedding: &[f32],
    model_rec: &EmbeddingModelRecord,
) -> Result<(), String> {
    if embedding.is_empty() {
        return Err(format!(
            "embedding_endpoint_style '{}': validation error: empty embedding vector",
            style
        ));
    }
    let expected_dimension = model_rec
        .dimensions
        .unwrap_or_else(|| model_rec.embedding_size.max(0) as usize);
    if expected_dimension > 0 && embedding.len() != expected_dimension {
        return Err(format!(
            "embedding_endpoint_style '{}': validation error: dimension mismatch: expected {}, got {}",
            style,
            expected_dimension,
            embedding.len()
        ));
    }
    Ok(())
}

pub fn is_retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || (500..=599).contains(&status)
}

pub fn is_retryable_embedding_error(error: &str) -> bool {
    error.contains("retryable network error")
        || error.contains("retryable bad status")
        || error.contains("bad status 408")
        || error.contains("bad status 429")
        || (500..=599).any(|status| error.contains(&format!("bad status {}", status)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::llm_types::BaseModelRecord;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn make_test_model_rec(endpoint: String) -> EmbeddingModelRecord {
        EmbeddingModelRecord {
            base: BaseModelRecord {
                name: "test-embedding".to_string(),
                endpoint,
                api_key: "test-key".to_string(),
                ..Default::default()
            },
            embedding_size: 3,
            dimensions: Some(3),
            ..Default::default()
        }
    }

    async fn serve_once(
        status: u16,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0_u8; 8192];
            let n = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..n]).to_string();
            let response = format!(
                "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });
        (format!("http://{}", addr), handle)
    }

    async fn run_openai(body: &'static str) -> Result<Vec<Vec<f32>>, String> {
        let (endpoint, _) = serve_once(200, body).await;
        let client = Arc::new(AMutex::new(reqwest::Client::new()));
        get_embedding_openai_style(
            client,
            vec!["a".to_string(), "b".to_string()],
            &make_test_model_rec(endpoint),
        )
        .await
    }

    #[tokio::test]
    async fn empty_batch_returns_empty_vec() {
        let client = Arc::new(AMutex::new(reqwest::Client::new()));
        let model_rec = make_test_model_rec("http://localhost:8080/embeddings".to_string());

        let result = get_embedding_openai_style(client, vec![], &model_rec).await;

        assert_eq!(result.unwrap(), Vec::<Vec<f32>>::new());
    }

    #[tokio::test]
    async fn empty_endpoint_returns_error_with_style() {
        let client = Arc::new(AMutex::new(reqwest::Client::new()));
        let model_rec = make_test_model_rec(String::new());

        let result = get_embedding_openai_style(client, vec!["test".to_string()], &model_rec).await;

        assert!(result
            .unwrap_err()
            .contains("embedding_endpoint_style 'openai'"));
    }

    #[tokio::test]
    async fn openai_payload_auth_and_headers() {
        let (endpoint, request_handle) =
            serve_once(200, r#"{"data":[{"index":0,"embedding":[1,2,3]}]}"#).await;
        let client = Arc::new(AMutex::new(reqwest::Client::new()));
        let mut model_rec = make_test_model_rec(endpoint);
        model_rec.base.auth_token = "oauth-token".to_string();
        model_rec
            .base
            .extra_headers
            .insert("X-Test".to_string(), "yes".to_string());
        model_rec
            .base
            .extra_headers
            .insert("Authorization".to_string(), "Bearer hacked".to_string());

        let result = get_embedding_openai_style(client, vec!["hello".to_string()], &model_rec)
            .await
            .unwrap();
        let request = request_handle.await.unwrap();

        assert_eq!(result, vec![vec![1.0, 2.0, 3.0]]);
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer oauth-token"));
        assert!(request.to_ascii_lowercase().contains("x-test: yes"));
        assert!(request.contains("\"input\":[\"hello\"]"));
        assert!(request.contains("\"model\":\"test-embedding\""));
        assert!(request.contains("\"dimensions\":3"));
        assert!(!request.contains("Bearer hacked"));
    }

    #[tokio::test]
    async fn openai_indexed_ordering() {
        let result = run_openai(
            r#"{"data":[{"index":1,"embedding":[4,5,6]},{"index":0,"embedding":[1,2,3]}]}"#,
        )
        .await
        .unwrap();

        assert_eq!(result, vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]);
    }

    #[tokio::test]
    async fn openai_no_index_fallback_preserves_order() {
        let result = run_openai(r#"{"data":[{"embedding":[1,2,3]},{"embedding":[4,5,6]}]}"#)
            .await
            .unwrap();

        assert_eq!(result, vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]);
    }

    #[tokio::test]
    async fn ollama_native_payload_and_parsing() {
        let (endpoint, request_handle) =
            serve_once(200, r#"{"embeddings":[[1,2,3],[4,5,6]]}"#).await;
        let client = Arc::new(AMutex::new(reqwest::Client::new()));
        let model_rec = make_test_model_rec(endpoint);

        let result = get_embedding_ollama_native_style(
            client,
            vec!["a".to_string(), "b".to_string()],
            &model_rec,
        )
        .await
        .unwrap();
        let request = request_handle.await.unwrap();

        assert_eq!(result, vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]);
        assert!(request.contains("\"input\":[\"a\",\"b\"]"));
        assert!(request.contains("\"model\":\"test-embedding\""));
        assert!(!request.contains("dimensions"));
    }

    #[tokio::test]
    async fn validation_rejects_wrong_dimensions_and_empty_vectors() {
        assert!(run_openai(
            r#"{"data":[{"index":0,"embedding":[1,2]},{"index":1,"embedding":[3,4,5]}]}"#
        )
        .await
        .unwrap_err()
        .contains("dimension mismatch"));
        assert!(run_openai(
            r#"{"data":[{"index":0,"embedding":[]},{"index":1,"embedding":[3,4,5]}]}"#
        )
        .await
        .unwrap_err()
        .contains("empty embedding vector"));
    }

    #[tokio::test]
    async fn validation_rejects_missing_duplicate_and_out_of_range_indices() {
        assert!(
            run_openai(r#"{"data":[{"index":1,"embedding":[1,2,3]},{"embedding":[4,5,6]}]}"#)
                .await
                .unwrap_err()
                .contains("missing index")
        );
        assert!(run_openai(
            r#"{"data":[{"index":0,"embedding":[1,2,3]},{"index":0,"embedding":[4,5,6]}]}"#
        )
        .await
        .unwrap_err()
        .contains("duplicate index"));
        assert!(run_openai(
            r#"{"data":[{"index":0,"embedding":[1,2,3]},{"index":2,"embedding":[4,5,6]}]}"#
        )
        .await
        .unwrap_err()
        .contains("out of range"));
    }

    #[tokio::test]
    async fn status_retry_classification_is_explicit() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_embedding_error(
            "embedding_endpoint_style 'openai': validation error: empty embedding vector"
        ));
    }
}
