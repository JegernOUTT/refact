use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use refact_core::llm_types::{EmbeddingModelRecord, HasBaseModelRecord, WireFormat, default_true};
use refact_core::provider_types::{extra_headers_mapping_to_hash_map, parse_extra_headers_value};

use super::model_records::{
    ChatModelRecord, CompletionModelRecord, DefaultModels, normalize_string,
};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CapsProvider {
    #[serde(default, deserialize_with = "normalize_string")]
    pub name: String,
    #[serde(default, deserialize_with = "normalize_string")]
    pub base_provider: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub supports_completion: bool,

    #[serde(default)]
    pub wire_format: WireFormat,

    #[serde(default = "default_endpoint_style")]
    pub endpoint_style: String,

    #[serde(default)]
    pub completion_endpoint: String,
    #[serde(default)]
    pub chat_endpoint: String,
    #[serde(default)]
    pub embedding_endpoint: String,

    #[serde(default)]
    pub api_key: String,

    #[serde(default)]
    pub tokenizer_api_key: String,

    #[serde(default)]
    pub extra_headers: HashMap<String, String>,

    #[serde(default)]
    pub code_completion_n_ctx: usize,

    #[serde(default)]
    pub completion_models: IndexMap<String, CompletionModelRecord>,
    #[serde(default)]
    pub chat_models: IndexMap<String, ChatModelRecord>,
    #[serde(default)]
    pub embedding_model: EmbeddingModelRecord,

    #[serde(default)]
    pub models_dict_patch: IndexMap<String, serde_json::Value>,

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

    #[serde(default)]
    pub running_models: Vec<String>,
}

impl CapsProvider {
    pub fn defaults(&self) -> DefaultModels {
        DefaultModels {
            completion_default_model: self.completion_default_model.clone(),
            chat_default_model: self.chat_default_model.clone(),
            chat_thinking_model: self.chat_thinking_model.clone(),
            chat_light_model: self.chat_light_model.clone(),
            chat_buddy_model: self.chat_buddy_model.clone(),
        }
    }

    pub fn apply_override(&mut self, value: serde_yaml::Value) -> Result<(), String> {
        set_field_if_exists::<String>(&mut self.base_provider, "base_provider", &value)?;
        set_field_if_exists::<bool>(&mut self.enabled, "enabled", &value)?;
        set_field_if_exists::<WireFormat>(&mut self.wire_format, "wire_format", &value)?;
        set_field_if_exists::<String>(&mut self.endpoint_style, "endpoint_style", &value)?;
        set_field_if_exists::<String>(
            &mut self.completion_endpoint,
            "completion_endpoint",
            &value,
        )?;
        set_field_if_exists::<String>(&mut self.chat_endpoint, "chat_endpoint", &value)?;
        set_field_if_exists::<String>(&mut self.embedding_endpoint, "embedding_endpoint", &value)?;
        set_field_if_exists::<String>(&mut self.api_key, "api_key", &value)?;
        set_field_if_exists::<String>(&mut self.tokenizer_api_key, "tokenizer_api_key", &value)?;
        if let Some(extra_headers) = value.get("extra_headers") {
            let headers = parse_extra_headers_value(extra_headers)?;
            self.extra_headers = extra_headers_mapping_to_hash_map(None, &headers);
        }
        set_field_if_exists::<EmbeddingModelRecord>(
            &mut self.embedding_model,
            "embedding_model",
            &value,
        )?;
        if value.get("embedding_model").is_some() {
            self.embedding_model.base.removable = true;
            self.embedding_model.base.user_configured = true;
        }

        if value.get("enabled_models").is_some() {
            self.running_models.clear();
            extend_collection::<Vec<String>>(&mut self.running_models, "enabled_models", &value)?;
        }
        extend_collection::<Vec<String>>(&mut self.running_models, "running_models", &value)?;
        extend_model_collection::<ChatModelRecord>(
            &mut self.chat_models,
            "chat_models",
            &value,
            &self.running_models,
        )?;
        extend_model_collection::<CompletionModelRecord>(
            &mut self.completion_models,
            "completion_models",
            &value,
            &self.running_models,
        )?;

        match serde_yaml::from_value::<DefaultModels>(value) {
            Ok(dm) => {
                if !dm.completion_default_model.is_empty() {
                    self.completion_default_model = dm.completion_default_model;
                }
                if !dm.chat_default_model.is_empty() {
                    self.chat_default_model = dm.chat_default_model;
                }
                if !dm.chat_thinking_model.is_empty() {
                    self.chat_thinking_model = dm.chat_thinking_model;
                }
                if !dm.chat_light_model.is_empty() {
                    self.chat_light_model = dm.chat_light_model;
                }
                if !dm.chat_buddy_model.is_empty() {
                    self.chat_buddy_model = dm.chat_buddy_model;
                }
            }
            Err(e) => return Err(e.to_string()),
        }

        Ok(())
    }
}

pub fn set_field_if_exists<T: for<'de> serde::Deserialize<'de>>(
    target: &mut T,
    field: &str,
    value: &serde_yaml::Value,
) -> Result<(), String> {
    if let Some(val) = value.get(field) {
        *target = serde_yaml::from_value(val.clone())
            .map_err(|_| format!("Field '{}' has incorrect type", field))?;
    }
    Ok(())
}

pub fn extend_collection<C: for<'de> serde::Deserialize<'de> + Extend<C::Item> + IntoIterator>(
    target: &mut C,
    field: &str,
    value: &serde_yaml::Value,
) -> Result<(), String> {
    if let Some(value) = value.get(field) {
        let imported_collection = serde_yaml::from_value::<C>(value.clone())
            .map_err(|_| format!("Invalid format for {field}"))?;

        target.extend(imported_collection);
    }
    Ok(())
}

pub fn extend_model_collection<T: for<'de> serde::Deserialize<'de> + HasBaseModelRecord>(
    target: &mut IndexMap<String, T>,
    field: &str,
    value: &serde_yaml::Value,
    prev_running_models: &[String],
) -> Result<(), String> {
    if let Some(value) = value.get(field) {
        let imported_collection = serde_yaml::from_value::<IndexMap<String, T>>(value.clone())
            .map_err(|_| format!("Invalid format for {field}"))?;

        for (key, mut model) in imported_collection {
            model.base_mut().user_configured = true;
            if !target.contains_key(&key) && !prev_running_models.contains(&key) {
                model.base_mut().removable = true;
            }
            target.insert(key, model);
        }
    }
    Ok(())
}

pub fn default_endpoint_style() -> String {
    "openai".to_string()
}

#[derive(Deserialize)]
pub struct CompletionPresets {
    pub completion_models: IndexMap<String, CompletionModelRecord>,
}

#[derive(Deserialize)]
pub struct EmbeddingPresets {
    pub embedding_models: IndexMap<String, EmbeddingModelRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::llm_types::BaseModelRecord;

    #[test]
    fn provider_defaults_returns_inline_model_selection() {
        let provider = CapsProvider {
            completion_default_model: "coder".to_string(),
            chat_default_model: "chat".to_string(),
            chat_thinking_model: "think".to_string(),
            chat_light_model: "light".to_string(),
            chat_buddy_model: "buddy".to_string(),
            ..Default::default()
        };

        let defaults = provider.defaults();

        assert_eq!(defaults.completion_default_model, "coder");
        assert_eq!(defaults.chat_default_model, "chat");
        assert_eq!(defaults.chat_thinking_model, "think");
        assert_eq!(defaults.chat_light_model, "light");
        assert_eq!(defaults.chat_buddy_model, "buddy");
    }

    #[test]
    fn apply_override_keeps_override_precedence_and_aliases() {
        let mut provider = CapsProvider {
            base_provider: "template".to_string(),
            enabled: true,
            wire_format: WireFormat::OpenaiChatCompletions,
            endpoint_style: "openai".to_string(),
            completion_endpoint: "old-completion".to_string(),
            chat_endpoint: "old-chat".to_string(),
            embedding_endpoint: "old-embedding".to_string(),
            api_key: "old-key".to_string(),
            tokenizer_api_key: "old-tokenizer".to_string(),
            running_models: vec!["template-model".to_string()],
            completion_default_model: "old-completion-model".to_string(),
            chat_default_model: "old-chat-model".to_string(),
            ..Default::default()
        };

        let value = serde_yaml::from_str(
            r#"
base_provider: openai
enabled: false
wire_format: anthropic_messages
endpoint_style: anthropic
completion_endpoint: new-completion
chat_endpoint: new-chat
embedding_endpoint: new-embedding
api_key: new-key
tokenizer_api_key: new-tokenizer
extra_headers:
  X-Test: value
embedding_model:
  name: embed
  n_ctx: 8192
enabled_models:
  - enabled-one
running_models:
  - running-two
chat_models:
  custom-chat:
    n_ctx: 100
completion_models:
  custom-completion:
    n_ctx: 200
completion_model: completion-default
chat_model: chat-default
chat_thinking_model: thinking-default
chat_light_model: light-default
chat_buddy_model: buddy-default
"#,
        )
        .unwrap();

        provider.apply_override(value).unwrap();

        assert_eq!(provider.base_provider, "openai");
        assert!(!provider.enabled);
        assert_eq!(provider.wire_format, WireFormat::AnthropicMessages);
        assert_eq!(provider.endpoint_style, "anthropic");
        assert_eq!(provider.completion_endpoint, "new-completion");
        assert_eq!(provider.chat_endpoint, "new-chat");
        assert_eq!(provider.embedding_endpoint, "new-embedding");
        assert_eq!(provider.api_key, "new-key");
        assert_eq!(provider.tokenizer_api_key, "new-tokenizer");
        assert_eq!(provider.extra_headers.get("X-Test").map(String::as_str), Some("value"));
        assert_eq!(provider.embedding_model.base.name, "embed");
        assert!(provider.embedding_model.base.removable);
        assert!(provider.embedding_model.base.user_configured);
        assert_eq!(provider.running_models, vec!["enabled-one", "running-two"]);
        assert!(provider.chat_models.contains_key("custom-chat"));
        assert!(provider.completion_models.contains_key("custom-completion"));
        assert_eq!(provider.completion_default_model, "completion-default");
        assert_eq!(provider.chat_default_model, "chat-default");
        assert_eq!(provider.chat_thinking_model, "thinking-default");
        assert_eq!(provider.chat_light_model, "light-default");
        assert_eq!(provider.chat_buddy_model, "buddy-default");
    }

    #[test]
    fn endpoint_style_deserializes_to_openai_by_default() {
        let provider: CapsProvider = serde_yaml::from_str("{}").unwrap();

        assert_eq!(provider.endpoint_style, "openai");
    }

    #[test]
    fn set_field_if_exists_sets_existing_field_and_rejects_wrong_type() {
        let value = serde_yaml::from_str("enabled: false\nname: updated\n").unwrap();
        let mut enabled = true;
        let mut name = "old".to_string();

        set_field_if_exists(&mut enabled, "enabled", &value).unwrap();
        set_field_if_exists(&mut name, "name", &value).unwrap();

        assert!(!enabled);
        assert_eq!(name, "updated");

        let mut count = 1usize;
        let error = set_field_if_exists(&mut count, "name", &value).unwrap_err();
        assert_eq!(error, "Field 'name' has incorrect type");
    }

    #[test]
    fn extend_collection_preserves_existing_then_imported_order() {
        let value = serde_yaml::from_str("models:\n  - b\n  - c\n").unwrap();
        let mut models = vec!["a".to_string()];

        extend_collection::<Vec<String>>(&mut models, "models", &value).unwrap();

        assert_eq!(models, vec!["a", "b", "c"]);
    }

    #[test]
    fn extend_model_collection_marks_added_models_and_preserves_order() {
        let value = serde_yaml::from_str(
            r#"
chat_models:
  existing:
    n_ctx: 20
  selected:
    n_ctx: 30
  added:
    n_ctx: 40
"#,
        )
        .unwrap();
        let mut target = IndexMap::new();
        target.insert(
            "existing".to_string(),
            ChatModelRecord {
                base: BaseModelRecord {
                    n_ctx: 10,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let prev_running_models = vec!["selected".to_string()];

        extend_model_collection(&mut target, "chat_models", &value, &prev_running_models).unwrap();

        let keys = target.keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys, vec!["existing", "selected", "added"]);
        assert_eq!(target["existing"].base.n_ctx, 20);
        assert!(target["existing"].base.user_configured);
        assert!(!target["existing"].base.removable);
        assert!(target["selected"].base.user_configured);
        assert!(!target["selected"].base.removable);
        assert!(target["added"].base.user_configured);
        assert!(target["added"].base.removable);
    }
}
