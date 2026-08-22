use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use reqwest::header::AUTHORIZATION;
use reqwest::header::CONTENT_TYPE;
use serde_json::json;
use tracing::info;

use refact_chat_history::history_limit::CompressionStrength;
use refact_core::chat_types::SamplingParameters;
use refact_core::custom_error::MapErrToString;
use refact_core::llm_types::BaseModelRecord;

#[derive(Debug)]
pub struct OpenAiEndpointError {
    pub status: Option<reqwest::StatusCode>,
    pub message: String,
}

impl OpenAiEndpointError {
    pub fn is_auth_rejection(&self) -> bool {
        matches!(
            self.status,
            Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN)
        )
    }
}

impl std::fmt::Display for OpenAiEndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for OpenAiEndpointError {}

fn redact_credential(text: String, bearer_credential: Option<&str>) -> String {
    match bearer_credential.filter(|value| !value.is_empty()) {
        Some(value) => {
            let mut redacted = text.replace(value, "[REDACTED]");
            if let Ok(escaped) = serde_json::to_string(value) {
                let escaped = escaped.trim_matches('"');
                if escaped != value {
                    redacted = redacted.replace(escaped, "[REDACTED]");
                }
            }
            redacted
        }
        None => text,
    }
}

fn endpoint_error(
    status: Option<reqwest::StatusCode>,
    message: String,
    bearer_credential: Option<&str>,
) -> OpenAiEndpointError {
    OpenAiEndpointError {
        status,
        message: redact_credential(message, bearer_credential),
    }
}

pub async fn forward_to_openai_style_endpoint(
    model_rec: &BaseModelRecord,
    prompt: &str,
    client: &reqwest::Client,
    sampling_parameters: &SamplingParameters,
) -> Result<serde_json::Value, String> {
    let bearer = (!model_rec.api_key.is_empty()).then_some(model_rec.api_key.as_str());
    forward_to_openai_style_endpoint_with_bearer(
        model_rec,
        prompt,
        client,
        sampling_parameters,
        bearer,
    )
    .await
    .map_err(|error| error.message)
}

pub async fn forward_to_openai_style_endpoint_with_bearer(
    model_rec: &BaseModelRecord,
    prompt: &str,
    client: &reqwest::Client,
    sampling_parameters: &SamplingParameters,
    bearer_credential: Option<&str>,
) -> Result<serde_json::Value, OpenAiEndpointError> {
    if model_rec.endpoint.is_empty() {
        return Err(endpoint_error(
            None,
            format!("No endpoint configured for {}", model_rec.id),
            bearer_credential,
        ));
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str("application/json").map_err(|e| {
            endpoint_error(
                None,
                format!("invalid content-type header: {}", e),
                bearer_credential,
            )
        })?,
    );
    if let Some(bearer_credential) = bearer_credential.filter(|value| !value.is_empty()) {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", bearer_credential)).map_err(|e| {
                endpoint_error(
                    None,
                    format!("invalid bearer credential for authorization header: {}", e),
                    Some(bearer_credential),
                )
            })?,
        );
    }
    let mut data = json!({
        "model": model_rec.name.clone(),
        "stream": false,
        "prompt": prompt,
        "echo": false,
    });
    if !sampling_parameters.stop.is_empty() {
        data["stop"] = serde_json::Value::from(sampling_parameters.stop.clone());
    };
    if let Some(n) = sampling_parameters.n {
        data["n"] = serde_json::Value::from(n);
    }
    if let Some(reasoning_effort) = sampling_parameters.reasoning_effort.clone() {
        data["reasoning_effort"] = serde_json::Value::String(reasoning_effort.to_string());
    } else if let Some(thinking) = sampling_parameters.thinking.clone() {
        data["thinking"] = thinking.clone();
    } else if let Some(enable_thinking) = sampling_parameters.enable_thinking {
        data["enable_thinking"] = serde_json::Value::Bool(enable_thinking);
        data["temperature"] = serde_json::Value::from(sampling_parameters.temperature);
    } else if let Some(temperature) = sampling_parameters.temperature {
        data["temperature"] = serde_json::Value::from(temperature);
    }
    data["max_completion_tokens"] = serde_json::Value::from(sampling_parameters.max_new_tokens);
    info!(
        "Request: model={}, reasoning_effort={}, T={}, n={}, stream=false",
        model_rec.name,
        sampling_parameters
            .reasoning_effort
            .clone()
            .map(|x| x.to_string())
            .unwrap_or("none".to_string()),
        sampling_parameters
            .temperature
            .clone()
            .map(|x| x.to_string())
            .unwrap_or("none".to_string()),
        sampling_parameters
            .n
            .clone()
            .map(|x| x.to_string())
            .unwrap_or("none".to_string())
    );
    let req = client
        .post(&model_rec.endpoint)
        .headers(headers)
        .body(data.to_string())
        .send()
        .await;
    let resp = req
        .map_err_to_string()
        .map_err(|message| endpoint_error(None, message, bearer_credential))?;
    let status = resp.status();
    let status_code = status.as_u16();
    let response_txt = resp.text().await.map_err(|e| {
        endpoint_error(
            Some(status),
            format!("reading from socket {}: {}", model_rec.endpoint, e),
            bearer_credential,
        )
    })?;
    if status_code != 200 && status_code != 400 {
        return Err(endpoint_error(
            Some(status),
            format!(
                "{} status={} text {}",
                model_rec.endpoint, status_code, response_txt
            ),
            bearer_credential,
        ));
    }
    let safe_response_txt = redact_credential(response_txt.clone(), bearer_credential);
    if status_code != 200 {
        tracing::info!(
            "forward_to_openai_style_endpoint: {} {}\n{}",
            model_rec.endpoint,
            status_code,
            safe_response_txt
        );
    }
    let response_to_parse = if status_code == 200 {
        &response_txt
    } else {
        &safe_response_txt
    };
    let parsed_json: serde_json::Value = match serde_json::from_str(response_to_parse) {
        Ok(json) => json,
        Err(e) => {
            return Err(endpoint_error(
                Some(status),
                format!(
                    "Failed to parse JSON response: {}\n{}",
                    e, safe_response_txt
                ),
                bearer_credential,
            ))
        }
    };
    if parsed_json.get("error").is_some() {
        return serde_json::from_str(&redact_credential(
            parsed_json.to_string(),
            bearer_credential,
        ))
        .map_err(|e| {
            endpoint_error(
                Some(status),
                format!("Failed to redact JSON error response: {}", e),
                bearer_credential,
            )
        });
    }
    Ok(parsed_json)
}

pub async fn forward_to_openai_style_endpoint_streaming(
    model_rec: &BaseModelRecord,
    prompt: &str,
    client: &reqwest::Client,
    sampling_parameters: &SamplingParameters,
) -> Result<reqwest::Response, String> {
    let bearer = (!model_rec.api_key.is_empty()).then_some(model_rec.api_key.as_str());
    forward_to_openai_style_endpoint_streaming_with_bearer(
        model_rec,
        prompt,
        client,
        sampling_parameters,
        bearer,
    )
    .await
    .map_err(|error| error.message)
}

pub async fn forward_to_openai_style_endpoint_streaming_with_bearer(
    model_rec: &BaseModelRecord,
    prompt: &str,
    client: &reqwest::Client,
    sampling_parameters: &SamplingParameters,
    bearer_credential: Option<&str>,
) -> Result<reqwest::Response, OpenAiEndpointError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str("application/json").map_err(|e| {
            endpoint_error(
                None,
                format!("invalid content-type header: {}", e),
                bearer_credential,
            )
        })?,
    );
    if let Some(bearer_credential) = bearer_credential.filter(|value| !value.is_empty()) {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", bearer_credential)).map_err(|e| {
                endpoint_error(
                    None,
                    format!("invalid bearer credential for authorization header: {}", e),
                    Some(bearer_credential),
                )
            })?,
        );
    }

    let mut data = json!({
        "model": model_rec.name,
        "stream": true,
        "stream_options": {"include_usage": true},
        "prompt": prompt,
    });

    if !sampling_parameters.stop.is_empty() {
        data["stop"] = serde_json::Value::from(sampling_parameters.stop.clone());
    };
    if let Some(n) = sampling_parameters.n {
        data["n"] = serde_json::Value::from(n);
    }

    if let Some(reasoning_effort) = sampling_parameters.reasoning_effort.clone() {
        data["reasoning_effort"] = serde_json::Value::String(reasoning_effort.to_string());
    } else if let Some(thinking) = sampling_parameters.thinking.clone() {
        data["thinking"] = thinking.clone();
    } else if let Some(enable_thinking) = sampling_parameters.enable_thinking {
        data["enable_thinking"] = serde_json::Value::Bool(enable_thinking);
        data["temperature"] = serde_json::Value::from(sampling_parameters.temperature);
    } else if let Some(temperature) = sampling_parameters.temperature {
        data["temperature"] = serde_json::Value::from(temperature);
    }
    data["max_completion_tokens"] = serde_json::Value::from(sampling_parameters.max_new_tokens);

    info!(
        "Request: model={}, reasoning_effort={}, T={}, n={}, stream=true",
        model_rec.name,
        sampling_parameters
            .reasoning_effort
            .clone()
            .map(|x| x.to_string())
            .unwrap_or("none".to_string()),
        sampling_parameters
            .temperature
            .clone()
            .map(|x| x.to_string())
            .unwrap_or("none".to_string()),
        sampling_parameters
            .n
            .clone()
            .map(|x| x.to_string())
            .unwrap_or("none".to_string())
    );

    if model_rec.endpoint.is_empty() {
        return Err(endpoint_error(
            None,
            format!("No endpoint configured for {}", model_rec.id),
            bearer_credential,
        ));
    }
    let response = client
        .post(&model_rec.endpoint)
        .headers(headers)
        .body(data.to_string())
        .send()
        .await
        .map_err(|e| {
            endpoint_error(
                None,
                format!("can't stream from {}: {}", model_rec.endpoint, e),
                bearer_credential,
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(endpoint_error(
            Some(status),
            format!("HTTP {} from {}: {}", status, model_rec.endpoint, text),
            bearer_credential,
        ));
    }
    Ok(response)
}

pub fn try_get_compression_from_prompt(_prompt: &str) -> serde_json::Value {
    json!(CompressionStrength::Absent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn empty_endpoint_returns_error() {
        let model_rec = BaseModelRecord {
            id: "test-model".to_string(),
            name: "test".to_string(),
            ..Default::default()
        };
        let client = reqwest::Client::new();
        let sampling_parameters = SamplingParameters::default();

        let result =
            forward_to_openai_style_endpoint(&model_rec, "prompt", &client, &sampling_parameters)
                .await;

        assert_eq!(result.unwrap_err(), "No endpoint configured for test-model");
    }

    #[tokio::test]
    async fn redacts_bearer_credential_from_accepted_400_json() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            let body = r#"{"error":"credential echoed: test-bearer-secret"}"#;
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let model_rec = BaseModelRecord {
            id: "test-model".to_string(),
            name: "test".to_string(),
            endpoint: format!("http://{address}"),
            ..Default::default()
        };

        let result = forward_to_openai_style_endpoint_with_bearer(
            &model_rec,
            "prompt",
            &reqwest::Client::new(),
            &SamplingParameters::default(),
            Some("test-bearer-secret"),
        )
        .await
        .unwrap();
        server.await.unwrap();

        assert_eq!(result["error"], "credential echoed: [REDACTED]");
        assert!(!result.to_string().contains("test-bearer-secret"));
    }

    #[test]
    fn redacts_json_escaped_bearer_credential() {
        let credential = "quoted\"secret\\value";
        let escaped = serde_json::to_string(credential).unwrap();
        let text = format!("{{\"error\":{escaped}}}");
        let redacted = redact_credential(text, Some(credential));

        assert!(!redacted.contains("quoted"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn compression_from_prompt_returns_absent_json_shape() {
        assert_eq!(try_get_compression_from_prompt("prompt"), json!("absent"));
    }
}
