use std::sync::Arc;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use tokio::sync::Mutex as AMutex;

use refact_core::llm_types::EmbeddingEndpointStyle;
use refact_core::vecdb_types::EmbeddingModelConfig;

#[derive(serde::Serialize)]
struct OpenAiPayload {
    input: Vec<String>,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(serde::Serialize)]
struct OllamaPayload {
    input: Vec<String>,
    model: String,
}

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
        EmbeddingEndpointStyle::Openai | EmbeddingEndpointStyle::OllamaNative => {
            get_embedding_for_style(client, text, embedding_model, style).await
        }
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
                if attempt_n >= max_retries || !is_retryable_embedding_error(&e) {
                    return Err(e);
                }
                if text.len() > 1 {
                    tracing::info!(
                        "embedding retry #{} for {} texts: {}",
                        attempt_n,
                        text.len(),
                        e
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_ON_BIG_BATCH))
                        .await;
                } else {
                    tracing::info!("embedding retry #{} for 1 text: {}", attempt_n, e);
                    tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_ON_BATCH_ONE))
                        .await;
                }
            }
        }
    }
}

async fn get_embedding_for_style(
    client: Arc<AMutex<reqwest::Client>>,
    text: Vec<String>,
    model: &EmbeddingModelConfig,
    style: EmbeddingEndpointStyle,
) -> Result<Vec<Vec<f32>>, String> {
    if text.is_empty() {
        return Ok(vec![]);
    }
    if model.endpoint.is_empty() {
        return Err(format!(
            "embedding_endpoint_style '{}': no embedding endpoint configured",
            style
        ));
    }

    let expected_count = text.len();
    let payload = match style {
        EmbeddingEndpointStyle::Openai => serde_json::to_value(OpenAiPayload {
            input: text,
            model: model.model_name.clone(),
            dimensions: model.dimensions,
        })
        .map_err(|e| {
            format!(
                "embedding_endpoint_style 'openai': failed to encode payload: {}",
                e
            )
        })?,
        EmbeddingEndpointStyle::OllamaNative => serde_json::to_value(OllamaPayload {
            input: text,
            model: model.model_name.clone(),
        })
        .map_err(|e| {
            format!(
                "embedding_endpoint_style 'ollama_native': failed to encode payload: {}",
                e
            )
        })?,
        _ => {
            return Err(format!(
                "embedding_endpoint_style '{}' is recognized but not supported yet",
                style
            ));
        }
    };

    let client_clone = client.lock().await.clone();
    let response = client_clone
        .post(&model.endpoint)
        .headers(embedding_headers(model, style)?)
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
        let body = response.text().await.unwrap_or_default();
        let retry_hint = if is_retryable_status(status.as_u16()) {
            "retryable "
        } else {
            ""
        };
        return Err(format!(
            "embedding_endpoint_style '{}': {}bad status {}: {}",
            style, retry_hint, status, body
        ));
    }

    let response_json: serde_json::Value = response.json().await.map_err(|e| {
        format!(
            "embedding_endpoint_style '{}': malformed provider payload: {}",
            style, e
        )
    })?;

    match style {
        EmbeddingEndpointStyle::Openai => {
            parse_openai_response(response_json, expected_count, model)
        }
        EmbeddingEndpointStyle::OllamaNative => {
            parse_ollama_response(response_json, expected_count, model)
        }
        _ => Err(format!(
            "embedding_endpoint_style '{}' is recognized but not supported yet",
            style
        )),
    }
}

fn embedding_headers(
    model: &EmbeddingModelConfig,
    style: EmbeddingEndpointStyle,
) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let bearer = if !model.auth_token.is_empty() {
        &model.auth_token
    } else {
        &model.api_key
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
    insert_extra_headers(&mut headers, &model.extra_headers);
    Ok(headers)
}

fn insert_extra_headers(
    headers: &mut HeaderMap,
    extra_headers: &std::collections::HashMap<String, String>,
) {
    for (key, value) in extra_headers {
        let key_lower = key.to_lowercase();
        if key_lower.starts_with("x-refact-internal-")
            || matches!(
                key_lower.as_str(),
                "authorization"
                    | "content-type"
                    | "content-length"
                    | "host"
                    | "connection"
                    | "transfer-encoding"
                    | "x-api-key"
                    | "api-key"
                    | "x-goog-api-key"
            )
        {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(name, value);
        }
    }
}

fn parse_openai_response(
    response_json: serde_json::Value,
    expected_count: usize,
    model: &EmbeddingModelConfig,
) -> Result<Vec<Vec<f32>>, String> {
    let data = response_json
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or("embedding_endpoint_style 'openai': malformed provider payload: missing 'data'")?;

    if data.len() != expected_count {
        return Err(format!(
            "embedding_endpoint_style 'openai': validation error: response length mismatch: expected {}, got {}",
            expected_count,
            data.len()
        ));
    }
    let has_any_index = data.iter().any(|item| item.get("index").is_some());
    let has_all_indices = data.iter().all(|item| item.get("index").is_some());
    if has_any_index && !has_all_indices {
        return Err(
            "embedding_endpoint_style 'openai': validation error: missing index in indexed response"
                .to_string(),
        );
    }

    if has_all_indices {
        let mut results = vec![None; expected_count];
        for item in data {
            let index = item
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("embedding_endpoint_style 'openai': validation error: missing index in indexed response")?
                as usize;
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
            let embedding = parse_embedding_item("openai", item, model)?;
            results[index] = Some(embedding);
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
        data.iter()
            .map(|item| parse_embedding_item("openai", item, model))
            .collect()
    }
}

fn parse_ollama_response(
    response_json: serde_json::Value,
    expected_count: usize,
    model: &EmbeddingModelConfig,
) -> Result<Vec<Vec<f32>>, String> {
    let embeddings = response_json
        .get("embeddings")
        .and_then(|d| d.as_array())
        .ok_or("embedding_endpoint_style 'ollama_native': malformed provider payload: missing 'embeddings'")?;
    if embeddings.len() != expected_count {
        return Err(format!(
            "embedding_endpoint_style 'ollama_native': validation error: response length mismatch: expected {}, got {}",
            expected_count,
            embeddings.len()
        ));
    }
    embeddings
        .iter()
        .map(|embedding| parse_embedding_array("ollama_native", embedding, model))
        .collect()
}

fn parse_embedding_item(
    style: &str,
    item: &serde_json::Value,
    model: &EmbeddingModelConfig,
) -> Result<Vec<f32>, String> {
    let embedding = item.get("embedding").ok_or_else(|| {
        format!(
            "embedding_endpoint_style '{}': malformed provider payload: missing 'embedding'",
            style
        )
    })?;
    parse_embedding_array(style, embedding, model)
}

fn parse_embedding_array(
    style: &str,
    value: &serde_json::Value,
    model: &EmbeddingModelConfig,
) -> Result<Vec<f32>, String> {
    let array = value.as_array().ok_or_else(|| {
        format!(
            "embedding_endpoint_style '{}': malformed provider payload: embedding is not an array",
            style
        )
    })?;
    let mut embedding = Vec::with_capacity(array.len());
    for number in array {
        let value = number.as_f64().ok_or_else(|| {
            format!(
                "embedding_endpoint_style '{}': malformed provider payload: embedding value is not numeric",
                style
            )
        })?;
        embedding.push(value as f32);
    }
    validate_vector(style, &embedding, model)?;
    Ok(embedding)
}

fn validate_vector(
    style: &str,
    embedding: &[f32],
    model: &EmbeddingModelConfig,
) -> Result<(), String> {
    if embedding.is_empty() {
        return Err(format!(
            "embedding_endpoint_style '{}': validation error: empty embedding vector",
            style
        ));
    }
    let expected_dimension = model
        .dimensions
        .unwrap_or_else(|| model.embedding_size.max(0) as usize);
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

fn is_retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || (500..=599).contains(&status)
}

fn is_retryable_embedding_error(error: &str) -> bool {
    error.contains("retryable network error")
        || error.contains("retryable bad status")
        || error.contains("bad status 408")
        || error.contains("bad status 429")
        || (500..=599).any(|status| error.contains(&format!("bad status {}", status)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn model(endpoint: String) -> EmbeddingModelConfig {
        EmbeddingModelConfig {
            endpoint,
            endpoint_style: "openai".to_string(),
            embedding_endpoint_style: String::new(),
            api_key: "api-key".to_string(),
            auth_token: String::new(),
            extra_headers: HashMap::new(),
            model_name: "embed".to_string(),
            embedding_size: 3,
            dimensions: None,
            query_prefix: String::new(),
            document_prefix: String::new(),
            rejection_threshold: 0.0,
            embedding_batch: 64,
            n_ctx: 0,
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

    #[tokio::test]
    async fn openai_indexed_ordering_and_validation_before_ingestion() {
        let (endpoint, _) = serve_once(
            200,
            r#"{"data":[{"index":1,"embedding":[4,5,6]},{"index":0,"embedding":[1,2,3]}]}"#,
        )
        .await;
        let result = get_embedding(
            Arc::new(AMutex::new(reqwest::Client::new())),
            &model(endpoint),
            vec!["a".to_string(), "b".to_string()],
        )
        .await
        .unwrap();

        assert_eq!(result, vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]);
    }

    #[tokio::test]
    async fn ollama_native_payload_and_parsing() {
        let (endpoint, request_handle) = serve_once(200, r#"{"embeddings":[[1,2,3]]}"#).await;
        let mut config = model(endpoint);
        config.embedding_endpoint_style = "ollama_native".to_string();

        let result = get_embedding(
            Arc::new(AMutex::new(reqwest::Client::new())),
            &config,
            vec!["hello".to_string()],
        )
        .await
        .unwrap();
        let request = request_handle.await.unwrap();

        assert_eq!(result, vec![vec![1.0, 2.0, 3.0]]);
        assert!(request.contains("\"input\":[\"hello\"]"));
        assert!(request.contains("\"model\":\"embed\""));
    }

    #[tokio::test]
    async fn rejects_empty_vectors_wrong_dimensions_and_bad_indices() {
        for (body, needle) in [
            (
                r#"{"data":[{"index":0,"embedding":[]}]}"#,
                "empty embedding vector",
            ),
            (
                r#"{"data":[{"index":0,"embedding":[1,2]}]}"#,
                "dimension mismatch",
            ),
            (
                r#"{"data":[{"index":0,"embedding":[1,2,3]},{"index":0,"embedding":[4,5,6]}]}"#,
                "duplicate index",
            ),
        ] {
            let (endpoint, _) = serve_once(200, body).await;
            let mut config = model(endpoint);
            let texts = if needle == "duplicate index" {
                vec!["a".to_string(), "b".to_string()]
            } else {
                vec!["a".to_string()]
            };
            let err = get_embedding(
                Arc::new(AMutex::new(reqwest::Client::new())),
                &config,
                texts,
            )
            .await
            .unwrap_err();
            assert!(err.contains(needle));
            config.embedding_size = 0;
        }
    }

    #[tokio::test]
    async fn unsupported_style_error_mentions_embedding_endpoint_style() {
        let config = EmbeddingModelConfig {
            embedding_endpoint_style: "voyage".to_string(),
            ..model("http://localhost".to_string())
        };

        let err = get_embedding(
            Arc::new(AMutex::new(reqwest::Client::new())),
            &config,
            vec!["a".to_string()],
        )
        .await
        .unwrap_err();

        assert_eq!(
            err,
            "embedding_endpoint_style 'voyage' is recognized but not supported yet"
        );
    }
}
