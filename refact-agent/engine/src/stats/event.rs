use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallEvent {
    pub id: String,
    pub ts_start: String,
    pub ts_end: String,
    pub duration_ms: u64,
    pub chat_id: String,
    pub root_chat_id: Option<String>,
    pub mode: String,
    pub task_id: Option<String>,
    pub task_role: Option<String>,
    pub agent_id: Option<String>,
    pub card_id: Option<String>,
    pub model_id: String,
    pub provider: String,
    pub model: String,
    pub messages_count: usize,
    pub tools_count: usize,
    pub max_tokens: usize,
    pub temperature: Option<f32>,
    pub success: bool,
    pub error_message: Option<String>,
    pub finish_reason: Option<String>,
    pub attempt_n: usize,
    pub retry_reason: Option<String>,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub cache_read_tokens: Option<usize>,
    pub cache_creation_tokens: Option<usize>,
    pub total_tokens: usize,
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub cost_coins: Option<f64>,
}

pub fn sum_metering_coins(extra: &serde_json::Map<String, serde_json::Value>) -> Option<f64> {
    let mut total = 0.0f64;
    let mut found = false;
    for (key, value) in extra {
        if key.starts_with("metering_coins_") && key != "metering_balance" {
            if let Some(v) = value.as_f64() {
                total += v;
                found = true;
            }
        }
    }
    if found {
        Some(total)
    } else {
        None
    }
}

pub fn split_model_provider(model_id: &str) -> (String, String) {
    match model_id.split_once('/') {
        Some((provider, model)) => (provider.to_string(), model.to_string()),
        None => ("unknown".to_string(), model_id.to_string()),
    }
}

pub fn canonicalize_mode_for_stats(mode: &str) -> String {
    crate::call_validation::canonical_mode_id(mode).unwrap_or_else(|_| mode.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_model_provider_normal() {
        let (provider, model) = split_model_provider("anthropic/claude-3");
        assert_eq!(provider, "anthropic");
        assert_eq!(model, "claude-3");
    }

    #[test]
    fn test_split_model_provider_no_slash() {
        let (provider, model) = split_model_provider("gpt-4");
        assert_eq!(provider, "unknown");
        assert_eq!(model, "gpt-4");
    }

    #[test]
    fn test_split_model_provider_sub_model() {
        let (provider, model) = split_model_provider("provider/sub/model");
        assert_eq!(provider, "provider");
        assert_eq!(model, "sub/model");
    }

    #[test]
    fn test_split_model_provider_empty() {
        let (provider, model) = split_model_provider("");
        assert_eq!(provider, "unknown");
        assert_eq!(model, "");
    }

    #[test]
    fn test_sum_metering_coins_basic() {
        let mut map = serde_json::Map::new();
        map.insert("metering_coins_prompt".to_string(), serde_json::json!(10.0));
        map.insert(
            "metering_coins_generated".to_string(),
            serde_json::json!(5.0),
        );
        let result = sum_metering_coins(&map);
        assert_eq!(result, Some(15.0));
    }

    #[test]
    fn test_sum_metering_coins_excludes_balance() {
        let mut map = serde_json::Map::new();
        map.insert("metering_coins_prompt".to_string(), serde_json::json!(10.0));
        map.insert("metering_balance".to_string(), serde_json::json!(1000.0));
        let result = sum_metering_coins(&map);
        assert_eq!(result, Some(10.0));
    }

    #[test]
    fn test_sum_metering_coins_empty_map() {
        let map = serde_json::Map::new();
        let result = sum_metering_coins(&map);
        assert_eq!(result, None);
    }

    #[test]
    fn test_sum_metering_coins_no_coins_keys() {
        let mut map = serde_json::Map::new();
        map.insert("metering_balance".to_string(), serde_json::json!(1000.0));
        map.insert("other_field".to_string(), serde_json::json!(5.0));
        let result = sum_metering_coins(&map);
        assert_eq!(result, None);
    }

    #[test]
    fn test_llm_call_event_backward_compat_no_cost_coins() {
        let json = r#"{"id":"test","ts_start":"2026-01-01T00:00:00Z","ts_end":"2026-01-01T00:00:01Z","duration_ms":1000,"chat_id":"chat-1","root_chat_id":null,"mode":"agent","task_id":null,"task_role":null,"agent_id":null,"card_id":null,"model_id":"anthropic/claude-3","provider":"anthropic","model":"claude-3","messages_count":1,"tools_count":0,"max_tokens":4096,"temperature":null,"success":true,"error_message":null,"finish_reason":"stop","attempt_n":1,"retry_reason":null,"prompt_tokens":100,"completion_tokens":50,"cache_read_tokens":null,"cache_creation_tokens":null,"total_tokens":150,"cost_usd":0.001}"#;
        let event: LlmCallEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.cost_coins, None);
    }

    #[test]
    fn test_canonicalize_mode_for_stats_normalizes_legacy_modes() {
        assert_eq!(canonicalize_mode_for_stats("TASK_AGENT"), "task_agent");
        assert_eq!(canonicalize_mode_for_stats("NO_TOOLS"), "explore");
        assert_eq!(canonicalize_mode_for_stats("plan"), "plan");
    }
}
