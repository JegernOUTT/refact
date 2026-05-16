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
}

pub fn split_model_provider(model_id: &str) -> (String, String) {
    match model_id.split_once('/') {
        Some((provider, model)) => (provider.to_string(), model.to_string()),
        None => ("unknown".to_string(), model_id.to_string()),
    }
}

pub fn canonicalize_mode_for_stats(mode: &str) -> String {
    refact_core::chat_types::canonical_mode_id(mode).unwrap_or_else(|_| mode.to_string())
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
    fn test_canonicalize_mode_for_stats_normalizes_legacy_modes() {
        assert_eq!(canonicalize_mode_for_stats("TASK_AGENT"), "task_agent");
        assert_eq!(canonicalize_mode_for_stats("NO_TOOLS"), "explore");
        assert_eq!(canonicalize_mode_for_stats("plan"), "plan");
    }
}
