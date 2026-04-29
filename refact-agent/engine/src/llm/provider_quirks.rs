use serde_json::{json, Value};

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

pub fn remove_anthropic_unsupported_fields(body: &mut Value, settings: &AdapterSettings) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };

    if !settings.supports_cache_control {
        obj.remove("cache_control");
    }
    if !settings.supports_tools {
        obj.remove("tools");
        obj.remove("tool_choice");
    }
    if !settings.supports_reasoning {
        obj.remove("thinking");
        obj.remove("output_config");
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
}
