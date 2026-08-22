use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use refact_core::llm_types::EmbeddingEndpointStyle;
use refact_core::vecdb_types::EmbeddingModelConfig;
use crate::vdb_structs::EmbeddingCredentialResolver;

enum EmbeddingError {
    Retryable(String),
    Terminal(String),
}

impl EmbeddingError {
    fn into_message(self) -> String {
        match self {
            Self::Retryable(message) | Self::Terminal(message) => message,
        }
    }
}

impl From<String> for EmbeddingError {
    fn from(message: String) -> Self {
        Self::Retryable(message)
    }
}

impl From<&str> for EmbeddingError {
    fn from(message: &str) -> Self {
        Self::Retryable(message.to_string())
    }
}

struct EmbeddingCredentialState {
    active: String,
    dynamic: bool,
    refresh_used: bool,
    secrets: Vec<String>,
}

impl EmbeddingCredentialState {
    async fn resolve(
        model: &EmbeddingModelConfig,
        resolver: Option<&EmbeddingCredentialResolver>,
    ) -> Result<Self, EmbeddingError> {
        let mut state = Self {
            active: model.api_key.clone(),
            dynamic: resolver.is_some(),
            refresh_used: false,
            secrets: Vec::new(),
        };
        state.add_secret(&model.api_key);
        if let Some(resolver) = resolver {
            let credential = resolver(None).await.map_err(EmbeddingError::Terminal)?;
            if credential.is_empty() {
                return Err(EmbeddingError::Terminal(
                    "embedding command credential is empty".to_string(),
                ));
            }
            state.active = credential.clone();
            state.add_secret(&credential);
        }
        Ok(state)
    }

    fn add_secret(&mut self, value: &str) {
        if !value.is_empty() && !self.secrets.iter().any(|secret| secret == value) {
            self.secrets.push(value.to_string());
        }
    }
}

pub async fn get_embedding(
    client: Arc<AMutex<reqwest::Client>>,
    embedding_model: &EmbeddingModelConfig,
    credential_resolver: Option<&EmbeddingCredentialResolver>,
    text: Vec<String>,
) -> Result<Vec<Vec<f32>>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut credential_state =
        EmbeddingCredentialState::resolve(embedding_model, credential_resolver)
            .await
            .map_err(EmbeddingError::into_message)?;
    get_embedding_internal(
        client,
        embedding_model,
        credential_resolver,
        &mut credential_state,
        text,
    )
    .await
    .map_err(EmbeddingError::into_message)
}

async fn get_embedding_internal(
    client: Arc<AMutex<reqwest::Client>>,
    embedding_model: &EmbeddingModelConfig,
    credential_resolver: Option<&EmbeddingCredentialResolver>,
    credential_state: &mut EmbeddingCredentialState,
    text: Vec<String>,
) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    if embedding_model.embedding_endpoint_style.is_empty()
        && embedding_model.endpoint_style.eq_ignore_ascii_case("hf")
    {
        return Err("HuggingFace endpoint style is no longer supported. Please use 'openai' embedding_endpoint_style with an OpenAI-compatible embedding endpoint.".into());
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
        EmbeddingEndpointStyle::Openai => {
            get_embedding_openai_style(
                client,
                text,
                embedding_model,
                credential_resolver,
                credential_state,
            )
            .await
        }
        EmbeddingEndpointStyle::OllamaNative => Err(
            "embedding_endpoint_style 'ollama_native' is not supported by this embedding transport yet".to_string(),
        ).map_err(EmbeddingError::from),
        style => Err(EmbeddingError::Retryable(format!(
            "embedding_endpoint_style '{}' is recognized but not supported yet",
            style
        ))),
    }
}

const SLEEP_ON_BIG_BATCH: u64 = 9000;
const SLEEP_ON_BATCH_ONE: u64 = 100;

pub async fn get_embedding_with_retries(
    client: Arc<AMutex<reqwest::Client>>,
    embedding_model: &EmbeddingModelConfig,
    credential_resolver: Option<&EmbeddingCredentialResolver>,
    text: Vec<String>,
    max_retries: usize,
) -> Result<Vec<Vec<f32>>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut credential_state =
        EmbeddingCredentialState::resolve(embedding_model, credential_resolver)
            .await
            .map_err(EmbeddingError::into_message)?;
    let mut attempt_n = 0;
    loop {
        attempt_n += 1;
        match get_embedding_internal(
            client.clone(),
            embedding_model,
            credential_resolver,
            &mut credential_state,
            text.clone(),
        )
        .await
        {
            Ok(embedding) => return Ok(embedding),
            Err(EmbeddingError::Terminal(message)) => return Err(message),
            Err(EmbeddingError::Retryable(message)) => {
                if attempt_n >= max_retries {
                    return Err(message);
                }
                if text.len() > 1 {
                    if message.contains("503") {
                        tracing::info!("normal sleep on 503");
                        tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_ON_BIG_BATCH))
                            .await;
                    } else {
                        tracing::info!(
                            "embedding retry #{} for {} texts: {}",
                            attempt_n,
                            text.len(),
                            message
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_ON_BIG_BATCH))
                            .await;
                    }
                } else {
                    tracing::info!("embedding retry #{} for 1 text: {}", attempt_n, message);
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
    credential_resolver: Option<&EmbeddingCredentialResolver>,
    credential_state: &mut EmbeddingCredentialState,
) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    if text.is_empty() {
        return Ok(vec![]);
    }
    if model.endpoint.is_empty() {
        return Err("No embedding endpoint configured".into());
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
    let mut response =
        send_openai_request(&client_clone, model, &payload, &credential_state.active).await?;

    if matches!(response.status().as_u16(), 401 | 403)
        && credential_state.dynamic
        && !credential_state.refresh_used
    {
        if let Some(resolver) = credential_resolver {
            credential_state.refresh_used = true;
            let rejected_credential = credential_state.active.clone();
            let refreshed = resolver(Some(rejected_credential))
                .await
                .map_err(EmbeddingError::Terminal)?;
            if refreshed.is_empty() {
                return Err(EmbeddingError::Terminal(
                    "embedding command credential is empty".to_string(),
                ));
            }
            credential_state.active = refreshed.clone();
            credential_state.add_secret(&refreshed);
            response =
                send_openai_request(&client_clone, model, &payload, &credential_state.active)
                    .await?;
        }
    }

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let credentials: Vec<&str> = credential_state
            .secrets
            .iter()
            .map(String::as_str)
            .collect();
        let body = redact_credentials(&body, &credentials);
        let error = format!("Embedding request failed with status {}: {}", status, body);
        return if matches!(status.as_u16(), 401 | 403) {
            Err(EmbeddingError::Terminal(error))
        } else {
            Err(EmbeddingError::Retryable(error))
        };
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
        return Err(EmbeddingError::Retryable(format!(
            "Embedding response length mismatch: expected {}, got {}",
            text.len(),
            results.len()
        )));
    }

    Ok(results)
}

async fn send_openai_request(
    client: &reqwest::Client,
    model: &EmbeddingModelConfig,
    payload: &impl serde::Serialize,
    credential: &str,
) -> Result<reqwest::Response, String> {
    let mut request = client.post(&model.endpoint).json(payload);
    if !credential.is_empty() {
        request = request.bearer_auth(credential);
    }
    request
        .send()
        .await
        .map_err(|e| format!("Failed to send embedding request: {}", e))
}

fn redact_credentials(body: &str, credentials: &[&str]) -> String {
    credentials
        .iter()
        .fold(body.to_string(), |redacted, credential| {
            if credential.is_empty() {
                redacted
            } else {
                redacted.replace(*credential, "[REDACTED]")
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn model(endpoint: String, api_key: &str) -> EmbeddingModelConfig {
        EmbeddingModelConfig {
            model_id: "test/embedding".to_string(),
            endpoint,
            endpoint_style: "openai".to_string(),
            embedding_endpoint_style: "openai".to_string(),
            api_key: api_key.to_string(),
            model_name: "embedding".to_string(),
            embedding_size: 1,
            dimensions: None,
            query_prefix: String::new(),
            document_prefix: String::new(),
            rejection_threshold: 1.0,
            embedding_batch: 1,
            n_ctx: 1,
        }
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut request = vec![0u8; 4096];
        let read = socket.read(&mut request).await.unwrap();
        String::from_utf8_lossy(&request[..read]).into_owned()
    }

    async fn write_response(socket: &mut tokio::net::TcpStream, status: &str, body: &str) {
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn dynamic_credential_refreshes_once_after_401() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/embeddings", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        let server = tokio::spawn(async move {
            for expected_credential in ["old-token", "new-token"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                server_requests.fetch_add(1, Ordering::SeqCst);
                assert!(request
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {}", expected_credential)));
                if expected_credential == "old-token" {
                    write_response(&mut socket, "401 Unauthorized", "rejected old-token").await;
                } else {
                    write_response(&mut socket, "200 OK", r#"{"data":[{"embedding":[1.0]}]}"#)
                        .await;
                }
            }
        });
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let calls = resolver_calls.clone();
        let resolver: EmbeddingCredentialResolver = Arc::new(move |rejected| {
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                match rejected.as_deref() {
                    None => Ok("old-token".to_string()),
                    Some("old-token") => Ok("new-token".to_string()),
                    other => Err(format!("unexpected rejected credential: {other:?}")),
                }
            })
        });

        let result = get_embedding_with_retries(
            Arc::new(AMutex::new(reqwest::Client::new())),
            &model(endpoint, "unused-static-token"),
            Some(&resolver),
            vec!["text".to_string()],
            5,
        )
        .await
        .unwrap();

        assert_eq!(result, vec![vec![1.0]]);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 2);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn generic_retry_does_not_reset_auth_refresh_budget() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/embeddings", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        let server = tokio::spawn(async move {
            for (expected_credential, status, body) in [
                ("old-token", "401 Unauthorized", "old-token rejected"),
                ("new-token", "503 Service Unavailable", "retry later"),
                ("new-token", "401 Unauthorized", "new-token rejected"),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                server_requests.fetch_add(1, Ordering::SeqCst);
                assert!(request
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {expected_credential}")));
                write_response(&mut socket, status, body).await;
            }
        });
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let calls = resolver_calls.clone();
        let resolver: EmbeddingCredentialResolver = Arc::new(move |rejected| {
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(if rejected.is_some() {
                    "new-token".to_string()
                } else {
                    "old-token".to_string()
                })
            })
        });

        let error = get_embedding_with_retries(
            Arc::new(AMutex::new(reqwest::Client::new())),
            &model(endpoint, ""),
            Some(&resolver),
            vec!["text".to_string()],
            5,
        )
        .await
        .unwrap_err();

        assert!(error.contains("401 Unauthorized"));
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 2);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn static_api_key_remains_compatible() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/embeddings", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer static-token"));
            write_response(&mut socket, "200 OK", r#"{"data":[{"embedding":[2.0]}]}"#).await;
        });

        let result = get_embedding(
            Arc::new(AMutex::new(reqwest::Client::new())),
            &model(endpoint, "static-token"),
            None,
            vec!["text".to_string()],
        )
        .await
        .unwrap();

        assert_eq!(result, vec![vec![2.0]]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn provider_error_redacts_static_and_dynamic_credentials() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/embeddings", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                let body = if request.contains("dynamic-old") {
                    "rejected dynamic-old and static-secret"
                } else {
                    "rejected dynamic-new, dynamic-old, and static-secret"
                };
                write_response(&mut socket, "403 Forbidden", body).await;
            }
        });
        let resolver: EmbeddingCredentialResolver = Arc::new(move |rejected| {
            Box::pin(async move {
                Ok(if rejected.is_some() {
                    "dynamic-new".to_string()
                } else {
                    "dynamic-old".to_string()
                })
            })
        });

        let error = get_embedding_with_retries(
            Arc::new(AMutex::new(reqwest::Client::new())),
            &model(endpoint, "static-secret"),
            Some(&resolver),
            vec!["text".to_string()],
            5,
        )
        .await
        .unwrap_err();

        assert!(error.contains("403 Forbidden"));
        assert!(error.contains("[REDACTED]"));
        assert!(!error.contains("static-secret"));
        assert!(!error.contains("dynamic-old"));
        assert!(!error.contains("dynamic-new"));
        server.await.unwrap();
    }
}
