use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use reqwest::header::AUTHORIZATION;
use reqwest::header::CONTENT_TYPE;
use serde_json::json;
use tracing::info;

use refact_chat_history::history_limit::CompressionStrength;
use refact_core::chat_types::SamplingParameters;
use refact_core::custom_error::MapErrToString;
use refact_core::llm_types::BaseModelRecord;
use refact_core::llm_types::CompletionEndpointStyle;

pub async fn forward_to_openai_style_endpoint(
    model_rec: &BaseModelRecord,
    prompt: &str,
    client: &reqwest::Client,
    sampling_parameters: &SamplingParameters,
) -> Result<serde_json::Value, String> {
    let style = validate_completion_endpoint(model_rec)?;
    let headers = completion_headers(model_rec)?;
    let data = completion_payload(model_rec, prompt, sampling_parameters, false, style);
    log_completion_request(model_rec, sampling_parameters, false, style);

    let req = client
        .post(&model_rec.endpoint)
        .headers(headers)
        .body(data.to_string())
        .send()
        .await;
    let resp = req.map_err_to_string()?;
    let status = resp.status();
    let status_code = status.as_u16();
    let response_txt = resp
        .text()
        .await
        .map_err(|e| format!("reading from socket {}: {}", model_rec.endpoint, e))?;
    if !status.is_success() {
        return Err(format!(
            "HTTP {} from {}: {}",
            status_code, model_rec.endpoint, response_txt
        ));
    }
    let parsed_json: serde_json::Value = match serde_json::from_str(&response_txt) {
        Ok(json) => json,
        Err(e) => {
            return Err(format!(
                "Failed to parse JSON response: {}\n{}",
                e, response_txt
            ))
        }
    };
    Ok(parsed_json)
}

pub async fn forward_to_openai_style_endpoint_streaming(
    model_rec: &BaseModelRecord,
    prompt: &str,
    client: &reqwest::Client,
    sampling_parameters: &SamplingParameters,
) -> Result<reqwest::Response, String> {
    let style = validate_completion_endpoint(model_rec)?;
    let headers = completion_headers(model_rec)?;
    let data = completion_payload(model_rec, prompt, sampling_parameters, true, style);
    log_completion_request(model_rec, sampling_parameters, true, style);

    let response = client
        .post(&model_rec.endpoint)
        .headers(headers)
        .body(data.to_string())
        .send()
        .await
        .map_err(|e| format!("can't stream from {}: {}", model_rec.endpoint, e))?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "HTTP {} from {}: {}",
            status.as_u16(),
            model_rec.endpoint,
            text
        ));
    }
    Ok(response)
}

fn validate_completion_endpoint(
    model_rec: &BaseModelRecord,
) -> Result<CompletionEndpointStyle, String> {
    if model_rec.endpoint.is_empty() {
        return Err(format!(
            "No completion endpoint configured for {}",
            model_rec.id
        ));
    }
    if model_rec.completion_endpoint_style.trim().is_empty()
        && model_rec.endpoint_style.eq_ignore_ascii_case("hf")
    {
        return Err("HuggingFace endpoint style is no longer supported. Please use completion_endpoint_style 'openai_completions' or 'openai_chat_completions'.".to_string());
    }
    let style = model_rec.effective_completion_endpoint_style()?;
    match style {
        CompletionEndpointStyle::OpenaiCompletions
        | CompletionEndpointStyle::OpenaiChatCompletions => Ok(style),
        other => Err(format!(
            "completion_endpoint_style '{}' is recognized but not supported yet",
            other
        )),
    }
}

fn completion_headers(model_rec: &BaseModelRecord) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    if !model_rec.auth_token.is_empty() {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", model_rec.auth_token))
                .map_err(|e| format!("invalid auth_token for authorization header: {}", e))?,
        );
    } else if !model_rec.api_key.is_empty() {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", model_rec.api_key))
                .map_err(|e| format!("invalid api_key for authorization header: {}", e))?,
        );
    }

    let allow_authorization_extra = !headers.contains_key(AUTHORIZATION);
    for (key, value) in &model_rec.extra_headers {
        if key.to_ascii_lowercase().starts_with("x-refact-internal-") {
            continue;
        }
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|e| format!("invalid extra header name '{}': {}", key, e))?;
        if name == CONTENT_TYPE {
            continue;
        }
        if name == AUTHORIZATION && !allow_authorization_extra {
            continue;
        }
        let val = HeaderValue::from_str(value)
            .map_err(|e| format!("invalid extra header value for '{}': {}", key, e))?;
        headers.insert(name, val);
    }
    Ok(headers)
}

fn completion_payload(
    model_rec: &BaseModelRecord,
    prompt: &str,
    sampling_parameters: &SamplingParameters,
    stream: bool,
    style: CompletionEndpointStyle,
) -> serde_json::Value {
    let mut data = match style {
        CompletionEndpointStyle::OpenaiCompletions => json!({
            "model": model_rec.name,
            "stream": stream,
            "prompt": prompt,
            "echo": false,
        }),
        CompletionEndpointStyle::OpenaiChatCompletions => json!({
            "model": model_rec.name,
            "stream": stream,
            "messages": [{"role": "user", "content": prompt}],
        }),
        CompletionEndpointStyle::OpenaiResponses => unreachable!(),
    };

    if stream {
        data["stream_options"] = json!({"include_usage": true});
    }
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
    data["max_tokens"] = serde_json::Value::from(sampling_parameters.max_new_tokens);
    data
}

fn log_completion_request(
    model_rec: &BaseModelRecord,
    sampling_parameters: &SamplingParameters,
    stream: bool,
    style: CompletionEndpointStyle,
) {
    info!(
        "Request: model={}, completion_endpoint_style={}, reasoning_effort={}, T={}, n={}, stream={}",
        model_rec.name,
        style,
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
            .unwrap_or("none".to_string()),
        stream,
    );
}

pub fn try_get_compression_from_prompt(_prompt: &str) -> serde_json::Value {
    json!(CompressionStrength::Absent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Read;
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    struct CapturedRequest {
        headers: HashMap<String, String>,
        body: serde_json::Value,
    }

    fn model_rec(endpoint: String, style: &str) -> BaseModelRecord {
        BaseModelRecord {
            id: "custom/provider-qualified".to_string(),
            name: "upstream-name".to_string(),
            endpoint,
            completion_endpoint_style: style.to_string(),
            ..Default::default()
        }
    }

    fn sampling() -> SamplingParameters {
        SamplingParameters {
            max_new_tokens: 37,
            temperature: Some(0.3),
            n: Some(2),
            stop: vec!["STOP".to_string()],
            ..Default::default()
        }
    }

    fn serve_once(status: &str, body: &'static str) -> (String, mpsc::Receiver<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let status = status.to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0_u8; 1024];
            loop {
                let n = stream.read(&mut tmp).unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(header_end) = find_header_end(&buf) {
                    let headers_txt = String::from_utf8_lossy(&buf[..header_end]).to_string();
                    let content_len = headers_txt
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            if name.eq_ignore_ascii_case("content-length") {
                                value.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    let body_start = header_end + 4;
                    while buf.len() < body_start + content_len {
                        let n = stream.read(&mut tmp).unwrap();
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let headers = headers_txt
                        .lines()
                        .skip(1)
                        .filter_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            Some((name.to_ascii_lowercase(), value.trim().to_string()))
                        })
                        .collect::<HashMap<_, _>>();
                    let request_body = &buf[body_start..body_start + content_len];
                    let parsed = serde_json::from_slice(request_body).unwrap();
                    tx.send(CapturedRequest {
                        headers,
                        body: parsed,
                    })
                    .unwrap();
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                status,
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        (format!("http://{}", addr), rx)
    }

    fn find_header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n")
    }

    #[tokio::test]
    async fn legacy_completions_non_stream_payload_uses_prompt_and_max_tokens() {
        let (endpoint, rx) = serve_once(
            "200 OK",
            r#"{"choices":[{"text":"done","finish_reason":"stop"}]}"#,
        );
        let model_rec = model_rec(endpoint, "openai_completions");
        let client = reqwest::Client::new();

        let result = forward_to_openai_style_endpoint(&model_rec, "prompt", &client, &sampling())
            .await
            .unwrap();

        assert_eq!(result["choices"][0]["text"], "done");
        let req = rx.recv().unwrap();
        assert_eq!(req.body["model"], "upstream-name");
        assert_eq!(req.body["prompt"], "prompt");
        assert_eq!(req.body["max_tokens"], 37);
        assert!(req.body.get("max_completion_tokens").is_none());
        assert_eq!(req.body["stream"], false);
        assert_eq!(req.body["stop"], json!(["STOP"]));
        assert!((req.body["temperature"].as_f64().unwrap() - 0.3).abs() < 0.000_001);
        assert_eq!(req.body["n"], 2);
    }

    #[tokio::test]
    async fn legacy_completions_streaming_payload_uses_prompt_shape() {
        let (endpoint, rx) = serve_once("200 OK", "data: [DONE]\n\n");
        let model_rec = model_rec(endpoint, "openai_completions");
        let client = reqwest::Client::new();

        let _response =
            forward_to_openai_style_endpoint_streaming(&model_rec, "prompt", &client, &sampling())
                .await
                .unwrap();

        let req = rx.recv().unwrap();
        assert_eq!(req.body["stream"], true);
        assert_eq!(req.body["stream_options"], json!({"include_usage": true}));
        assert_eq!(req.body["prompt"], "prompt");
        assert!(req.body.get("messages").is_none());
    }

    #[tokio::test]
    async fn chat_completion_wrapper_payload_uses_messages_and_parses_response() {
        let (endpoint, rx) = serve_once(
            "200 OK",
            r#"{"choices":[{"message":{"content":"wrapped"},"finish_reason":"length"}]}"#,
        );
        let model_rec = model_rec(endpoint, "openai_chat_completions");
        let client = reqwest::Client::new();

        let result = forward_to_openai_style_endpoint(&model_rec, "prompt", &client, &sampling())
            .await
            .unwrap();

        assert_eq!(result["choices"][0]["message"]["content"], "wrapped");
        let req = rx.recv().unwrap();
        assert!(req.body.get("prompt").is_none());
        assert_eq!(
            req.body["messages"],
            json!([{ "role": "user", "content": "prompt" }])
        );
        assert_eq!(req.body["max_tokens"], 37);
    }

    #[tokio::test]
    async fn bearer_auth_and_extra_headers_follow_precedence() {
        let (endpoint, rx) = serve_once(
            "200 OK",
            r#"{"choices":[{"text":"done","finish_reason":"stop"}]}"#,
        );
        let mut model_rec = model_rec(endpoint, "openai_completions");
        model_rec.api_key = "api-key".to_string();
        model_rec.auth_token = "auth-token".to_string();
        model_rec.extra_headers.insert(
            "Authorization".to_string(),
            "Bearer custom-authorization".to_string(),
        );
        model_rec
            .extra_headers
            .insert("X-Tenant".to_string(), "team-a".to_string());
        let client = reqwest::Client::new();

        forward_to_openai_style_endpoint(&model_rec, "prompt", &client, &sampling())
            .await
            .unwrap();

        let req = rx.recv().unwrap();
        assert_eq!(
            req.headers.get("authorization").unwrap(),
            "Bearer auth-token"
        );
        assert_eq!(req.headers.get("x-tenant").unwrap(), "team-a");
    }

    #[tokio::test]
    async fn custom_authorization_header_is_used_without_bearer_credentials() {
        let (endpoint, rx) = serve_once(
            "200 OK",
            r#"{"choices":[{"text":"done","finish_reason":"stop"}]}"#,
        );
        let mut model_rec = model_rec(endpoint, "openai_completions");
        model_rec.extra_headers.insert(
            "Authorization".to_string(),
            "Bearer custom-authorization".to_string(),
        );
        let client = reqwest::Client::new();

        forward_to_openai_style_endpoint(&model_rec, "prompt", &client, &sampling())
            .await
            .unwrap();

        let req = rx.recv().unwrap();
        assert_eq!(
            req.headers.get("authorization").unwrap(),
            "Bearer custom-authorization"
        );
    }

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

        assert_eq!(
            result.unwrap_err(),
            "No completion endpoint configured for test-model"
        );
    }

    #[tokio::test]
    async fn unsupported_completion_endpoint_style_returns_clear_error() {
        let model_rec = BaseModelRecord {
            id: "test-model".to_string(),
            name: "test".to_string(),
            endpoint: "http://127.0.0.1:1".to_string(),
            completion_endpoint_style: "openai_responses".to_string(),
            ..Default::default()
        };
        let client = reqwest::Client::new();

        let result =
            forward_to_openai_style_endpoint(&model_rec, "prompt", &client, &sampling()).await;

        assert_eq!(
            result.unwrap_err(),
            "completion_endpoint_style 'openai_responses' is recognized but not supported yet"
        );
    }

    #[tokio::test]
    async fn response_statuses_are_mapped_to_http_errors() {
        for status in [
            "400 Bad Request",
            "401 Unauthorized",
            "429 Too Many Requests",
            "500 Internal Server Error",
        ] {
            let (endpoint, _rx) = serve_once(status, r#"{"error":"bad"}"#);
            let model_rec = model_rec(endpoint.clone(), "openai_completions");
            let client = reqwest::Client::new();

            let err = forward_to_openai_style_endpoint(&model_rec, "prompt", &client, &sampling())
                .await
                .unwrap_err();

            let code = status.split(' ').next().unwrap();
            assert!(err.contains(&format!("HTTP {} from {}", code, endpoint)));
            assert!(err.contains(r#"{"error":"bad"}"#));
        }
    }

    #[test]
    fn compression_from_prompt_returns_absent_json_shape() {
        assert_eq!(try_get_compression_from_prompt("prompt"), json!("absent"));
    }
}
