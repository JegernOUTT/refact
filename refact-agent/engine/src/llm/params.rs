use serde::{Deserialize, Serialize};

const DEFAULT_MAX_TOKENS: usize = 4096;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CacheControl {
    #[default]
    Off,
    Ephemeral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_ctx: Option<usize>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub stop: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<usize>,
}

fn default_max_tokens() -> usize {
    DEFAULT_MAX_TOKENS
}

impl Default for CommonParams {
    fn default() -> Self {
        Self {
            n_ctx: None,
            max_tokens: DEFAULT_MAX_TOKENS,
            temperature: None,
            top_p: None,
            frequency_penalty: None,
            stop: Vec::new(),
            n: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningIntent {
    Off,
    NoReasoning,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
    BudgetTokens(usize),
}

impl Default for ReasoningIntent {
    fn default() -> Self {
        Self::Off
    }
}

impl ReasoningIntent {
    pub fn to_openai_effort(&self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::NoReasoning => Some("none"),
            Self::Minimal => Some("minimal"),
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::XHigh => Some("xhigh"),
            Self::Max => Some("xhigh"),
            Self::BudgetTokens(_) => Some("high"),
        }
    }

    pub fn to_anthropic_budget(&self, default_budget: usize) -> Option<usize> {
        match self {
            Self::Off | Self::NoReasoning => None,
            Self::Minimal => Some(default_budget / 8),
            Self::Low => Some(default_budget / 4),
            Self::Medium => Some(default_budget / 2),
            Self::High => Some(default_budget),
            Self::XHigh => Some(default_budget),
            Self::Max => Some(default_budget),
            Self::BudgetTokens(n) => Some(*n),
        }
    }

    pub fn to_anthropic_effort(&self) -> Option<&'static str> {
        match self {
            Self::Off | Self::NoReasoning => None,
            Self::Minimal => Some("low"),
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::XHigh => Some("xhigh"),
            Self::Max => Some("max"),
            Self::BudgetTokens(_) => Some("high"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reasoning_intent_openai() {
        assert_eq!(ReasoningIntent::Off.to_openai_effort(), None);
        assert_eq!(ReasoningIntent::Low.to_openai_effort(), Some("low"));
        assert_eq!(ReasoningIntent::Medium.to_openai_effort(), Some("medium"));
        assert_eq!(ReasoningIntent::High.to_openai_effort(), Some("high"));
        assert_eq!(
            ReasoningIntent::BudgetTokens(5000).to_openai_effort(),
            Some("high")
        );
    }

    #[test]
    fn test_reasoning_intent_anthropic_effort() {
        assert_eq!(ReasoningIntent::Off.to_anthropic_effort(), None);
        assert_eq!(ReasoningIntent::NoReasoning.to_anthropic_effort(), None);
        assert_eq!(ReasoningIntent::Low.to_anthropic_effort(), Some("low"));
        assert_eq!(
            ReasoningIntent::Medium.to_anthropic_effort(),
            Some("medium")
        );
        assert_eq!(ReasoningIntent::High.to_anthropic_effort(), Some("high"));
        assert_eq!(ReasoningIntent::XHigh.to_anthropic_effort(), Some("xhigh"));
        assert_eq!(ReasoningIntent::Max.to_anthropic_effort(), Some("max"));
        assert_eq!(
            ReasoningIntent::BudgetTokens(5000).to_anthropic_effort(),
            Some("high")
        );
    }

    #[test]
    fn test_reasoning_intent_anthropic() {
        assert_eq!(ReasoningIntent::Off.to_anthropic_budget(10000), None);
        assert_eq!(ReasoningIntent::Low.to_anthropic_budget(10000), Some(2500));
        assert_eq!(
            ReasoningIntent::Medium.to_anthropic_budget(10000),
            Some(5000)
        );
        assert_eq!(
            ReasoningIntent::High.to_anthropic_budget(10000),
            Some(10000)
        );
        assert_eq!(
            ReasoningIntent::BudgetTokens(8000).to_anthropic_budget(10000),
            Some(8000)
        );
    }
}
