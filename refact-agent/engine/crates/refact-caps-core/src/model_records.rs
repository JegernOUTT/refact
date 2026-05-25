use serde::{Deserialize, Serialize};

use refact_core::llm_types::{BaseModelRecord, HasBaseModelRecord, default_true};

#[derive(Debug, Serialize, Clone, Deserialize)]
pub struct ChatModelRecord {
    #[serde(flatten)]
    pub base: BaseModelRecord,

    #[allow(dead_code)]
    #[serde(default = "default_chat_scratchpad", skip_serializing)]
    pub scratchpad: String,
    #[allow(dead_code)]
    #[serde(default, skip_serializing)]
    pub scratchpad_patch: serde_json::Value,

    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_multimodality: bool,
    #[serde(default)]
    pub supports_clicks: bool,
    #[serde(default)]
    pub supports_agent: bool,
    #[serde(default)]
    pub reasoning_effort_options: Option<Vec<String>>,
    #[serde(default)]
    pub supports_thinking_budget: bool,
    #[serde(default)]
    pub supports_adaptive_thinking_budget: bool,
    #[serde(default)]
    pub max_thinking_tokens: Option<usize>,
    #[serde(default)]
    pub default_temperature: Option<f32>,
    #[serde(default)]
    pub default_frequency_penalty: Option<f32>,
    #[serde(default)]
    pub default_max_tokens: Option<usize>,
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
    #[serde(default)]
    pub supports_parallel_tools: bool,
    #[serde(default)]
    pub supports_strict_tools: bool,
    #[serde(default = "default_true")]
    pub supports_temperature: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_provider: Option<String>,
}

impl Default for ChatModelRecord {
    fn default() -> Self {
        Self {
            base: default_base_model_record(),
            scratchpad: default_chat_scratchpad(),
            scratchpad_patch: serde_json::Value::Null,
            supports_tools: false,
            supports_multimodality: false,
            supports_clicks: false,
            supports_agent: false,
            reasoning_effort_options: None,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            max_thinking_tokens: None,
            default_temperature: None,
            default_frequency_penalty: None,
            default_max_tokens: None,
            max_output_tokens: None,
            supports_parallel_tools: false,
            supports_strict_tools: false,
            supports_temperature: default_true(),
            available_providers: Vec::new(),
            selected_provider: None,
        }
    }
}

pub fn default_chat_scratchpad() -> String {
    String::new()
}

impl ChatModelRecord {
    pub fn has_reasoning_support(&self) -> bool {
        self.reasoning_effort_options.is_some()
            || self.supports_thinking_budget
            || self.supports_adaptive_thinking_budget
    }

    pub fn reasoning_type_string(&self) -> Option<String> {
        if self.supports_adaptive_thinking_budget {
            Some("anthropic_effort".to_string())
        } else if self.supports_thinking_budget {
            Some("anthropic_budget".to_string())
        } else if self.reasoning_effort_options.is_some() {
            Some("effort".to_string())
        } else {
            None
        }
    }
}

impl HasBaseModelRecord for ChatModelRecord {
    fn base(&self) -> &BaseModelRecord {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseModelRecord {
        &mut self.base
    }
}

#[derive(Debug, Serialize, Clone, Deserialize)]
pub struct CompletionModelRecord {
    #[serde(flatten)]
    pub base: BaseModelRecord,

    #[serde(default = "default_completion_scratchpad")]
    pub scratchpad: String,
    #[serde(default = "default_completion_scratchpad_patch")]
    pub scratchpad_patch: serde_json::Value,

    #[serde(default)]
    pub model_family: Option<CompletionModelFamily>,
}

impl Default for CompletionModelRecord {
    fn default() -> Self {
        Self {
            base: default_base_model_record(),
            scratchpad: default_completion_scratchpad(),
            scratchpad_patch: default_completion_scratchpad_patch(),
            model_family: None,
        }
    }
}

fn default_base_model_record() -> BaseModelRecord {
    BaseModelRecord {
        enabled: default_true(),
        supports_cache_control: default_true(),
        ..Default::default()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionModelFamily {
    #[serde(rename = "qwen2.5-coder-base")]
    Qwen2_5CoderBase,
    #[serde(rename = "starcoder")]
    Starcoder,
    #[serde(rename = "deepseek-coder")]
    DeepseekCoder,
}

pub fn default_completion_scratchpad() -> String {
    "FIM-PSM".to_string()
}

pub fn default_completion_scratchpad_patch() -> serde_json::Value {
    serde_json::json!({
        "context_format": "chat",
        "rag_ratio": 0.5
    })
}

impl HasBaseModelRecord for CompletionModelRecord {
    fn base(&self) -> &BaseModelRecord {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseModelRecord {
        &mut self.base
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CapsMetadata {
    #[serde(default = "default_pricing")]
    pub pricing: serde_json::Value,
    #[serde(default)]
    pub features: Vec<String>,
}

pub fn default_pricing() -> serde_json::Value {
    serde_json::json!({})
}

impl Default for CapsMetadata {
    fn default() -> Self {
        Self {
            pricing: default_pricing(),
            features: Vec::new(),
        }
    }
}

pub fn default_hf_tokenizer_template() -> String {
    "https://huggingface.co/$HF_MODEL/resolve/main/tokenizer.json".to_string()
}

pub fn normalize_string<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let s: String = String::deserialize(deserializer)?;
    Ok(s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect())
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct DefaultModels {
    #[serde(
        default,
        alias = "code_completion_default_model",
        alias = "completion_model"
    )]
    pub completion_default_model: String,
    #[serde(default, alias = "code_chat_default_model", alias = "chat_model")]
    pub chat_default_model: String,
    #[serde(default)]
    pub chat_thinking_model: String,
    #[serde(default)]
    pub chat_light_model: String,
    #[serde(default)]
    pub chat_buddy_model: String,
}

impl DefaultModels {
    fn qualify_model(model: &str, provider_name: Option<&str>) -> String {
        let Some(provider) = provider_name else {
            return model.to_string();
        };
        if model.is_empty() {
            return String::new();
        }
        if model.starts_with(&format!("{}/", provider)) {
            model.to_string()
        } else {
            format!("{}/{}", provider, model)
        }
    }

    pub fn apply_override(&mut self, other: &DefaultModels, provider_name: Option<&str>) {
        if !other.completion_default_model.is_empty() {
            self.completion_default_model =
                Self::qualify_model(&other.completion_default_model, provider_name);
        }
        if !other.chat_default_model.is_empty() {
            self.chat_default_model = Self::qualify_model(&other.chat_default_model, provider_name);
        }
        if !other.chat_thinking_model.is_empty() {
            self.chat_thinking_model =
                Self::qualify_model(&other.chat_thinking_model, provider_name);
        }
        if !other.chat_light_model.is_empty() {
            self.chat_light_model = Self::qualify_model(&other.chat_light_model, provider_name);
        }
        if !other.chat_buddy_model.is_empty() {
            self.chat_buddy_model = Self::qualify_model(&other.chat_buddy_model, provider_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct NormalizedName {
        #[serde(deserialize_with = "normalize_string")]
        name: String,
    }

    #[test]
    fn reasoning_support_and_type_strings_follow_precedence() {
        let none = ChatModelRecord::default();
        assert!(!none.has_reasoning_support());
        assert_eq!(none.reasoning_type_string(), None);

        let effort = ChatModelRecord {
            reasoning_effort_options: Some(vec!["low".to_string(), "high".to_string()]),
            ..Default::default()
        };
        assert!(effort.has_reasoning_support());
        assert_eq!(effort.reasoning_type_string().as_deref(), Some("effort"));

        let budget = ChatModelRecord {
            reasoning_effort_options: Some(vec!["medium".to_string()]),
            supports_thinking_budget: true,
            ..Default::default()
        };
        assert!(budget.has_reasoning_support());
        assert_eq!(
            budget.reasoning_type_string().as_deref(),
            Some("anthropic_budget")
        );

        let adaptive = ChatModelRecord {
            reasoning_effort_options: Some(vec!["medium".to_string()]),
            supports_thinking_budget: true,
            supports_adaptive_thinking_budget: true,
            ..Default::default()
        };
        assert!(adaptive.has_reasoning_support());
        assert_eq!(
            adaptive.reasoning_type_string().as_deref(),
            Some("anthropic_effort")
        );
    }

    #[test]
    fn completion_family_serde_roundtrip_preserves_flattened_fields() {
        let record = CompletionModelRecord {
            base: BaseModelRecord {
                n_ctx: 8192,
                name: "qwen".to_string(),
                tokenizer: "hf://tokenizer".to_string(),
                ..Default::default()
            },
            scratchpad: "custom".to_string(),
            scratchpad_patch: serde_json::json!({"fim_prefix": "<fim>", "rag_ratio": 0.25}),
            model_family: Some(CompletionModelFamily::Qwen2_5CoderBase),
        };

        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(
            value.get("model_family").and_then(|v| v.as_str()),
            Some("qwen2.5-coder-base")
        );
        assert_eq!(value.get("n_ctx").and_then(|v| v.as_u64()), Some(8192));

        let decoded: CompletionModelRecord = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.base.n_ctx, 8192);
        assert_eq!(decoded.base.name, "qwen");
        assert_eq!(decoded.base.tokenizer, "hf://tokenizer");
        assert_eq!(decoded.scratchpad, "custom");
        assert_eq!(
            decoded
                .scratchpad_patch
                .get("fim_prefix")
                .and_then(|v| v.as_str()),
            Some("<fim>")
        );
        assert_eq!(
            decoded.model_family,
            Some(CompletionModelFamily::Qwen2_5CoderBase)
        );
    }

    #[test]
    fn model_record_defaults_match_empty_serde_defaults_for_non_trivial_fields() {
        assert_eq!(default_chat_scratchpad(), "");
        assert_eq!(default_completion_scratchpad(), "FIM-PSM");
        assert_eq!(
            default_completion_scratchpad_patch(),
            serde_json::json!({"context_format": "chat", "rag_ratio": 0.5})
        );
        assert_eq!(
            default_hf_tokenizer_template(),
            "https://huggingface.co/$HF_MODEL/resolve/main/tokenizer.json"
        );

        let default_chat = ChatModelRecord::default();
        let chat: ChatModelRecord = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(default_chat.scratchpad, "");
        assert_eq!(chat.scratchpad, "");
        assert!(default_chat.base.enabled);
        assert!(chat.base.enabled);
        assert!(default_chat.base.supports_cache_control);
        assert!(chat.base.supports_cache_control);
        assert!(default_chat.supports_temperature);
        assert!(chat.supports_temperature);

        let default_completion = CompletionModelRecord::default();
        let completion: CompletionModelRecord =
            serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(default_completion.scratchpad, "FIM-PSM");
        assert_eq!(completion.scratchpad, "FIM-PSM");
        assert_eq!(
            default_completion.scratchpad_patch,
            serde_json::json!({"context_format": "chat", "rag_ratio": 0.5})
        );
        assert_eq!(
            completion.scratchpad_patch,
            serde_json::json!({"context_format": "chat", "rag_ratio": 0.5})
        );
        assert!(default_completion.base.enabled);
        assert!(completion.base.enabled);
        assert!(default_completion.base.supports_cache_control);
        assert!(completion.base.supports_cache_control);
    }

    #[test]
    fn default_pricing_shape_is_empty_object() {
        assert_eq!(default_pricing(), serde_json::json!({}));

        let metadata = CapsMetadata::default();
        assert_eq!(metadata.pricing, serde_json::json!({}));
        assert!(metadata.features.is_empty());

        let decoded: CapsMetadata = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(decoded.pricing, serde_json::json!({}));
        assert!(decoded.features.is_empty());
    }

    #[test]
    fn default_models_serde_aliases_and_defaults_work() {
        let decoded: DefaultModels = serde_json::from_value(serde_json::json!({
            "completion_model": "starcoder",
            "chat_model": "gpt-4.1",
            "chat_light_model": "gpt-4.1-mini"
        }))
        .unwrap();
        assert_eq!(decoded.completion_default_model, "starcoder");
        assert_eq!(decoded.chat_default_model, "gpt-4.1");
        assert_eq!(decoded.chat_thinking_model, "");
        assert_eq!(decoded.chat_light_model, "gpt-4.1-mini");
        assert_eq!(decoded.chat_buddy_model, "");

        let mut target = DefaultModels::default();
        target.apply_override(&decoded, Some("openai"));
        assert_eq!(target.completion_default_model, "openai/starcoder");
        assert_eq!(target.chat_default_model, "openai/gpt-4.1");
        assert_eq!(target.chat_light_model, "openai/gpt-4.1-mini");
        assert_eq!(target.chat_thinking_model, "");
        assert_eq!(target.chat_buddy_model, "");

        let mut already_qualified = DefaultModels::default();
        already_qualified.apply_override(
            &DefaultModels {
                chat_default_model: "openai/gpt-4.1".to_string(),
                ..Default::default()
            },
            Some("openai"),
        );
        assert_eq!(already_qualified.chat_default_model, "openai/gpt-4.1");
    }

    #[test]
    fn normalize_string_lowercases_and_replaces_separators() {
        let decoded: NormalizedName = serde_json::from_value(serde_json::json!({
            "name": "OpenAI Responses-2"
        }))
        .unwrap();
        assert_eq!(decoded.name, "openai_responses_2");
    }
}
