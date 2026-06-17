use std::time::Duration;

use reqwest::header::{HeaderMap, ACCEPT, CONTENT_TYPE};
use serde_json::Value;
use tracing::{debug, warn};

use crate::call_validation::ChatUsage;
use crate::caps::BaseModelRecord;
use crate::llm::adapter::{AdapterSettings, HttpParts, LlmWireAdapter};
use crate::llm::LlmRequest;
use refact_core::model_caps::{ANTHROPIC_CLOUD_TOKENIZER, CLAUDE_CLOUD_TOKENIZER_ALIAS};

const CLOUD_TOKEN_COUNT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudTokenizerKind {
    Anthropic,
}

#[derive(Debug, Clone)]
pub struct CloudInputTokenCount {
    pub usage: ChatUsage,
    pub output_token_reserve: usize,
}

fn cloud_tokenizer_kind(tokenizer: &str) -> Option<CloudTokenizerKind> {
    match tokenizer.trim().to_ascii_lowercase().as_str() {
        ANTHROPIC_CLOUD_TOKENIZER | CLAUDE_CLOUD_TOKENIZER_ALIAS => {
            Some(CloudTokenizerKind::Anthropic)
        }
        _ => None,
    }
}

fn set_path_segments(url: &str, segments: &[&str]) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    {
        let mut path_segments = parsed.path_segments_mut().ok()?;
        path_segments.clear();
        path_segments.extend(segments.iter().copied());
    }
    Some(parsed.to_string())
}

fn path_segments(url: &str) -> Option<Vec<String>> {
    Some(
        url::Url::parse(url)
            .ok()?
            .path_segments()?
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn anthropic_count_url(endpoint: &str) -> Option<String> {
    let mut segments = path_segments(endpoint)?;
    if segments.last().map(String::as_str) == Some("count_tokens") {
        return set_path_segments(
            endpoint,
            &segments.iter().map(String::as_str).collect::<Vec<_>>(),
        );
    }
    if segments.last().map(String::as_str) == Some("messages") {
        segments.push("count_tokens".to_string());
        return set_path_segments(
            endpoint,
            &segments.iter().map(String::as_str).collect::<Vec<_>>(),
        );
    }
    None
}

fn strip_anthropic_count_unsupported_fields(body: &mut Value) {
    if let Some(obj) = body.as_object_mut() {
        for key in [
            "max_tokens",
            "temperature",
            "stop_sequences",
            "stream",
            "metadata",
            "service_tier",
            "container",
            "context_management",
            "mcp_servers",
        ] {
            obj.remove(key);
        }
    }
}

fn body_usize(body: &Value, key: &str) -> Option<usize> {
    body.get(key)?.as_u64().map(|value| value as usize)
}

fn anthropic_count_http_parts(
    req: &LlmRequest,
    settings: &AdapterSettings,
) -> Result<(HttpParts, usize), String> {
    if settings.api_key.is_empty() && settings.auth_token.is_empty() {
        return Err("Anthropic cloud token count requires an API key or auth token".to_string());
    }
    let count_url = anthropic_count_url(&settings.endpoint)
        .ok_or_else(|| "Anthropic cloud token count requires a /messages endpoint".to_string())?;

    let mut http = crate::llm::adapters::anthropic::AnthropicAdapter.build_http(req, settings)?;
    let output_token_reserve =
        body_usize(&http.body, "max_tokens").unwrap_or(req.params.max_tokens);
    http.url = count_url;
    strip_anthropic_count_unsupported_fields(&mut http.body);
    Ok((http, output_token_reserve))
}

fn cloud_count_http_parts(
    req: &LlmRequest,
    model_rec: &BaseModelRecord,
) -> Result<Option<(HttpParts, usize)>, String> {
    let Some(kind) = cloud_tokenizer_kind(&model_rec.tokenizer) else {
        return Ok(None);
    };

    let api_key = if model_rec.tokenizer_api_key.is_empty() {
        model_rec.api_key.clone()
    } else {
        model_rec.tokenizer_api_key.clone()
    };

    let settings = AdapterSettings {
        api_key,
        auth_token: model_rec.auth_token.clone(),
        endpoint: model_rec.endpoint.clone(),
        extra_headers: model_rec.extra_headers.clone(),
        model_name: model_rec.name.clone(),
        supports_tools: true,
        supports_reasoning: true,
        reasoning_type: None,
        supports_temperature: true,
        supports_max_completion_tokens: model_rec.supports_max_completion_tokens,
        eof_is_done: model_rec.eof_is_done,
        supports_web_search: model_rec.supports_web_search,
        supports_cache_control: model_rec.supports_cache_control,
    };

    let parts = match kind {
        CloudTokenizerKind::Anthropic => anthropic_count_http_parts(req, &settings)?,
    };
    Ok(Some(parts))
}

fn response_input_tokens(body: &Value) -> Option<usize> {
    body.get("input_tokens")?.as_u64().map(|n| n as usize)
}

fn header_names(headers: &HeaderMap) -> Vec<String> {
    headers.keys().map(|key| key.as_str().to_string()).collect()
}

async fn send_cloud_count_request(
    client: &reqwest::Client,
    http_parts: &HttpParts,
) -> Result<usize, String> {
    let mut request = client
        .post(&http_parts.url)
        .headers(http_parts.headers.clone())
        .header(ACCEPT, "application/json")
        .json(&http_parts.body)
        .timeout(CLOUD_TOKEN_COUNT_TIMEOUT);

    if !http_parts.headers.contains_key(CONTENT_TYPE) {
        request = request.header(CONTENT_TYPE, "application/json");
    }
    let response = request
        .send()
        .await
        .map_err(|err| format!("cloud token count request failed: {err}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| format!("cloud token count response read failed: {err}"))?;
    if !status.is_success() {
        return Err(format!(
            "cloud token count failed with status {status}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    let body: Value = serde_json::from_str(&text)
        .map_err(|err| format!("cloud token count response was not JSON: {err}"))?;
    response_input_tokens(&body)
        .ok_or_else(|| format!("cloud token count response missing input_tokens: {body}"))
}

pub async fn try_count_input_tokens(
    client: &reqwest::Client,
    req: &LlmRequest,
    model_rec: &BaseModelRecord,
) -> Option<CloudInputTokenCount> {
    let (parts, output_token_reserve) = match cloud_count_http_parts(req, model_rec) {
        Ok(Some(parts)) => parts,
        Ok(None) => return None,
        Err(err) => {
            debug!(model = %model_rec.id, error = %err, "cloud token count not available");
            return None;
        }
    };

    debug!(
        model = %model_rec.id,
        url = %parts.url,
        headers = ?header_names(&parts.headers),
        "cloud token count request"
    );

    match send_cloud_count_request(client, &parts).await {
        Ok(input_tokens) => Some(CloudInputTokenCount {
            usage: ChatUsage {
                prompt_tokens: input_tokens,
                completion_tokens: 0,
                total_tokens: input_tokens,
                cache_creation_tokens: None,
                cache_read_tokens: None,
                metering_usd: None,
            },
            output_token_reserve,
        }),
        Err(err) => {
            warn!(model = %model_rec.id, error = %err, "cloud token count failed; falling back to local estimate/provider usage");
            None
        }
    }
}

pub fn cloud_input_exceeds_context(
    usage: &ChatUsage,
    context_limit: usize,
    output_token_reserve: usize,
) -> bool {
    context_limit > 0 && usage.prompt_tokens.saturating_add(output_token_reserve) > context_limit
}

pub fn cloud_context_limit_message(
    usage: &ChatUsage,
    model_rec: &BaseModelRecord,
    context_limit: usize,
    output_token_reserve: usize,
) -> String {
    format!(
        "Cloud token count for model '{}' is {} input tokens plus {} reserved output tokens, exceeding the configured context window of {} tokens",
        model_rec.id, usage.prompt_tokens, output_token_reserve, context_limit
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::ChatMessage;
    use crate::llm::ReasoningIntent;
    use crate::llm::WireFormat;
    use serde_json::json;

    fn req() -> LlmRequest {
        LlmRequest::new(
            "openai/gpt-4.1".to_string(),
            vec![ChatMessage::new("user".to_string(), "hello".to_string())],
        )
    }

    fn base_model(tokenizer: &str, wire_format: WireFormat, endpoint: &str) -> BaseModelRecord {
        BaseModelRecord {
            id: "openai/gpt-4.1".to_string(),
            name: "gpt-4.1".to_string(),
            tokenizer: tokenizer.to_string(),
            wire_format,
            endpoint: endpoint.to_string(),
            api_key: "sk-test".to_string(),
            supports_cache_control: true,
            ..Default::default()
        }
    }

    #[test]
    fn anthropic_count_parts_use_messages_count_tokens_endpoint() {
        let model = base_model(
            ANTHROPIC_CLOUD_TOKENIZER,
            WireFormat::AnthropicMessages,
            "https://api.anthropic.com/v1/messages",
        );

        let (parts, _output_token_reserve) =
            cloud_count_http_parts(&req(), &model).unwrap().unwrap();

        assert_eq!(
            parts.url,
            "https://api.anthropic.com/v1/messages/count_tokens"
        );
        assert_eq!(parts.body["model"], json!("gpt-4.1"));
        assert!(parts.body.get("messages").is_some());
        assert!(parts.body.get("max_tokens").is_none());
        assert!(parts.body.get("temperature").is_none());
    }

    #[test]
    fn anthropic_count_accepts_already_count_endpoint() {
        let model = base_model(
            ANTHROPIC_CLOUD_TOKENIZER,
            WireFormat::AnthropicMessages,
            "https://api.anthropic.com/v1/messages/count_tokens",
        );

        let (parts, _output_token_reserve) =
            cloud_count_http_parts(&req(), &model).unwrap().unwrap();

        assert_eq!(
            parts.url,
            "https://api.anthropic.com/v1/messages/count_tokens"
        );
    }

    #[test]
    fn anthropic_count_reserve_uses_adapter_adjusted_max_tokens() {
        let model = base_model(
            ANTHROPIC_CLOUD_TOKENIZER,
            WireFormat::AnthropicMessages,
            "https://api.anthropic.com/v1/messages",
        );
        let mut request = req();
        request.reasoning = ReasoningIntent::BudgetTokens(2048);
        request.params.max_tokens = 1024;

        let (_parts, output_token_reserve) =
            cloud_count_http_parts(&request, &model).unwrap().unwrap();

        assert_eq!(output_token_reserve, 3072);
    }

    #[test]
    fn cloud_context_limit_detects_exact_count_overflow() {
        let model = BaseModelRecord {
            id: "openai/gpt-4.1".to_string(),
            n_ctx: 100,
            ..Default::default()
        };
        let usage = ChatUsage {
            prompt_tokens: 101,
            total_tokens: 101,
            ..Default::default()
        };

        assert!(cloud_input_exceeds_context(&usage, model.n_ctx, 0));
        assert!(cloud_context_limit_message(&usage, &model, model.n_ctx, 0).contains("exceeding"));
    }

    #[test]
    fn cloud_context_limit_reserves_requested_output_tokens() {
        let usage = ChatUsage {
            prompt_tokens: 90,
            total_tokens: 90,
            ..Default::default()
        };

        assert!(!cloud_input_exceeds_context(&usage, 100, 10));
        assert!(cloud_input_exceeds_context(&usage, 100, 11));
    }
}
