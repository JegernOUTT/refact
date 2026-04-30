use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};

use crate::call_validation::ChatContent;
use crate::llm::adapter::AdapterSettings;
use crate::llm::canonical::LlmRequest;
use crate::llm::params::ReasoningIntent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefactProvider {
    Qwen,
    Zhipu,
}

fn refact_provider(model_id: &str) -> Option<RefactProvider> {
    let (provider, _) = model_id.split_once('/')?;
    match provider {
        "qwen" => Some(RefactProvider::Qwen),
        "zhipu" => Some(RefactProvider::Zhipu),
        _ => None,
    }
}

fn reasoning_requested(intent: &ReasoningIntent) -> bool {
    !matches!(intent, ReasoningIntent::Off | ReasoningIntent::NoReasoning)
}

fn is_github_copilot_endpoint(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    host == "api.githubcopilot.com" || host.starts_with("copilot-api.")
}

pub fn is_github_copilot_request(req: &LlmRequest, settings: &AdapterSettings) -> bool {
    req.model_id.starts_with("github_copilot/")
        || req.model_id.starts_with("github-copilot/")
        || is_github_copilot_endpoint(&settings.endpoint)
}

fn content_has_image(content: &ChatContent) -> bool {
    match content {
        ChatContent::Multimodal(elements) => elements.iter().any(|element| element.is_image()),
        _ => false,
    }
}

fn request_has_image(req: &LlmRequest) -> bool {
    req.messages
        .iter()
        .any(|message| content_has_image(&message.content))
}

fn github_copilot_initiator(req: &LlmRequest) -> &'static str {
    req.messages
        .iter()
        .rev()
        .find(|message| message.role != "context_file")
        .map(|message| {
            if message.role == "user" {
                "user"
            } else {
                "agent"
            }
        })
        .unwrap_or("user")
}

pub fn apply_github_copilot_request_headers(
    headers: &mut HeaderMap,
    req: &LlmRequest,
    settings: &AdapterSettings,
) {
    if !is_github_copilot_request(req, settings) {
        return;
    }

    headers.insert(
        "Openai-Intent",
        HeaderValue::from_static("conversation-edits"),
    );
    headers.insert(
        "x-initiator",
        HeaderValue::from_static(github_copilot_initiator(req)),
    );

    if request_has_image(req) {
        headers.insert("Copilot-Vision-Request", HeaderValue::from_static("true"));
    } else {
        headers.remove("Copilot-Vision-Request");
    }
}

pub fn uses_openai_provider_reasoning_controls(req: &LlmRequest) -> bool {
    matches!(
        refact_provider(&req.model_id),
        Some(RefactProvider::Qwen | RefactProvider::Zhipu)
    )
}

pub fn apply_openai_chat_body_quirks(
    body: &mut Value,
    req: &LlmRequest,
    settings: &AdapterSettings,
) {
    let Some(provider) = refact_provider(&req.model_id) else {
        return;
    };
    let Some(obj) = body.as_object_mut() else {
        return;
    };

    match provider {
        RefactProvider::Qwen => {
            obj.remove("reasoning_effort");
            let enabled = reasoning_requested(&req.reasoning);
            if enabled || settings.supports_reasoning {
                obj.insert("enable_thinking".to_string(), json!(enabled));
                if let ReasoningIntent::BudgetTokens(budget) = req.reasoning {
                    if enabled {
                        obj.insert("thinking_budget".to_string(), json!(budget));
                    }
                } else {
                    obj.remove("thinking_budget");
                }
            }
        }
        RefactProvider::Zhipu => {
            obj.remove("reasoning_effort");
            let enabled = reasoning_requested(&req.reasoning);
            if enabled || settings.supports_reasoning {
                let thinking_type = if enabled { "enabled" } else { "disabled" };
                obj.insert("thinking".to_string(), json!({"type": thinking_type}));
            }
        }
    }
}

fn remove_key_recursively(value: &mut Value, key: &str) {
    match value {
        Value::Object(obj) => {
            obj.remove(key);
            for value in obj.values_mut() {
                remove_key_recursively(value, key);
            }
        }
        Value::Array(items) => {
            for value in items {
                remove_key_recursively(value, key);
            }
        }
        _ => {}
    }
}

fn remove_anthropic_reasoning_blocks(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        content.retain(|block| {
            !matches!(
                block.get("type").and_then(Value::as_str),
                Some("thinking" | "redacted_thinking")
            )
        });
        if content.is_empty() {
            content.push(json!({"type": "text", "text": "(empty)"}));
        }
    }
}

pub fn remove_anthropic_unsupported_fields(body: &mut Value, settings: &AdapterSettings) {
    if !body.is_object() {
        return;
    }

    if !settings.supports_cache_control {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("cache_control");
        }
        remove_key_recursively(body, "cache_control");
    }
    if !settings.supports_tools {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("tools");
            obj.remove("tool_choice");
        }
    }
    if !settings.supports_reasoning {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("thinking");
            obj.remove("output_config");
        }
        remove_anthropic_reasoning_blocks(body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::adapter::AdapterSettings;

    fn settings() -> AdapterSettings {
        AdapterSettings {
            api_key: String::new(),
            auth_token: String::new(),
            endpoint: "https://example.com".to_string(),
            extra_headers: Default::default(),
            model_name: "model".to_string(),
            supports_tools: true,
            supports_reasoning: true,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: true,
        }
    }

    #[test]
    fn non_provider_model_has_no_openai_provider_reasoning_controls() {
        let req =
            LlmRequest::new("openai/o3".to_string(), vec![]).with_reasoning(ReasoningIntent::High);

        assert!(!uses_openai_provider_reasoning_controls(&req));
    }

    #[test]
    fn qwen_budget_sets_thinking_fields() {
        let req = LlmRequest::new("qwen/qwen3".to_string(), vec![])
            .with_reasoning(ReasoningIntent::BudgetTokens(2048));
        let mut body = json!({"reasoning_effort": "high"});

        apply_openai_chat_body_quirks(&mut body, &req, &settings());

        assert_eq!(body["enable_thinking"], true);
        assert_eq!(body["thinking_budget"], 2048);
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn zhipu_off_sets_disabled_for_thinking_capable_models() {
        let req = LlmRequest::new("zhipu/glm-4.7".to_string(), vec![]);
        let mut body = json!({});

        apply_openai_chat_body_quirks(&mut body, &req, &settings());

        assert_eq!(body["thinking"], json!({"type": "disabled"}));
    }

    #[test]
    fn custom_qwen_like_model_has_no_provider_fields() {
        let req = LlmRequest::new("custom/qwen3".to_string(), vec![])
            .with_reasoning(ReasoningIntent::BudgetTokens(2048));
        let mut body = json!({"reasoning_effort": "high"});

        apply_openai_chat_body_quirks(&mut body, &req, &settings());

        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("enable_thinking").is_none());
        assert!(body.get("thinking_budget").is_none());
    }

    #[test]
    fn custom_zhipu_like_model_has_no_provider_fields() {
        let req = LlmRequest::new("custom/glm-4.7".to_string(), vec![])
            .with_reasoning(ReasoningIntent::High);
        let mut body = json!({"reasoning_effort": "high"});

        apply_openai_chat_body_quirks(&mut body, &req, &settings());

        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("thinking").is_none());
    }

    fn contains_key_recursively(value: &Value, key: &str) -> bool {
        match value {
            Value::Object(obj) => {
                obj.contains_key(key)
                    || obj
                        .values()
                        .any(|value| contains_key_recursively(value, key))
            }
            Value::Array(items) => items
                .iter()
                .any(|value| contains_key_recursively(value, key)),
            _ => false,
        }
    }

    #[test]
    fn anthropic_unsupported_flags_strip_top_level_and_nested_fields() {
        let mut body = json!({
            "cache_control": {"type": "ephemeral"},
            "tools": [{"name": "blocked"}],
            "tool_choice": {"type": "any"},
            "thinking": {"type": "enabled", "budget_tokens": 4096},
            "output_config": {"effort": "high"},
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "visible", "cache_control": {"type": "ephemeral"}},
                    {"type": "thinking", "thinking": "hidden", "cache_control": {"type": "ephemeral"}},
                    {"type": "redacted_thinking", "data": "encrypted"}
                ]
            }]
        });
        let mut settings = settings();
        settings.supports_cache_control = false;
        settings.supports_tools = false;
        settings.supports_reasoning = false;

        remove_anthropic_unsupported_fields(&mut body, &settings);

        assert!(body.get("cache_control").is_none());
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("thinking").is_none());
        assert!(body.get("output_config").is_none());
        assert!(!contains_key_recursively(&body, "cache_control"));
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }
}
