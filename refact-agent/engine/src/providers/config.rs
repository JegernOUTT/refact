use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelTypeDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_new_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost_reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderDefaults {
    #[serde(default)]
    pub chat: ModelTypeDefaults,
    #[serde(default)]
    pub chat_light: ModelTypeDefaults,
    #[serde(default)]
    pub chat_thinking: ModelTypeDefaults,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
}

impl ProviderDefaults {
    pub fn defaults_for_model(
        &self,
        model_id: &str,
        _chat_default_model: &str,
        chat_light_model: &str,
        chat_thinking_model: &str,
    ) -> &ModelTypeDefaults {
        if !chat_thinking_model.is_empty() && model_id == chat_thinking_model {
            &self.chat_thinking
        } else if !chat_light_model.is_empty() && model_id == chat_light_model {
            &self.chat_light
        } else {
            &self.chat
        }
    }

    pub async fn load(config_dir: &Path) -> Result<Self, String> {
        let defaults_path = config_dir.join("providers.d").join("defaults.yaml");
        match tokio::fs::read_to_string(&defaults_path).await {
            Ok(content) => {
                serde_yaml::from_str(&content)
                    .map_err(|e| format!("Failed to parse defaults.yaml: {}", e))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::default())
            }
            Err(e) => Err(format!("Failed to read defaults.yaml: {}", e)),
        }
    }

    pub async fn save(&self, config_dir: &Path) -> Result<(), String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir)
            .await
            .map_err(|e| format!("Failed to create providers.d directory: {}", e))?;

        let defaults_path = providers_dir.join("defaults.yaml");
        let content = serde_yaml::to_string(self)
            .map_err(|e| format!("Failed to serialize defaults: {}", e))?;

        let temp_path = providers_dir.join(format!(
            "defaults.yaml.tmp.{}.{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        tokio::fs::write(&temp_path, &content)
            .await
            .map_err(|e| format!("Failed to write temp file: {}", e))?;

        tokio::fs::rename(&temp_path, &defaults_path)
            .await
            .map_err(|e| {
                let _ = std::fs::remove_file(&temp_path);
                format!("Failed to rename temp file to defaults.yaml: {}", e)
            })
    }
}

pub fn resolve_env_var(value: &str, fallback: &str, context: &str) -> String {
    if value.is_empty() {
        return fallback.to_string();
    }
    if value.starts_with('$') {
        match std::env::var(&value[1..]) {
            Ok(env_val) => env_val,
            Err(e) => {
                tracing::error!("Failed to read env var {} for {}: {}", value, context, e);
                fallback.to_string()
            }
        }
    } else {
        value.to_string()
    }
}
