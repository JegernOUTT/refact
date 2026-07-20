use std::collections::HashSet;
use std::path::Path;

use crate::config_store;
use crate::identity::provider_identity_from_yaml;
use crate::instance::ProviderInstance;
use crate::traits::ProviderTrait;
use crate::{
    anthropic::AnthropicProvider, openai::OpenAIProvider,
    openai_responses::OpenAIResponsesProvider, openai_codex::OpenAICodexProvider,
    openrouter::OpenRouterProvider, ollama::OllamaProvider, lmstudio::LMStudioProvider,
    vllm::VLLMProvider, groq::GroqProvider, deepseek::DeepseekProvider, doubao::DoubaoProvider,
    xai::XAIProvider, xai_responses::XAIResponsesProvider, google_gemini::GoogleGeminiProvider,
    qwen::QwenProvider, kimi::KimiProvider, zhipu::ZhipuProvider, minimax::MiniMaxProvider,
    github_copilot::GitHubCopilotProvider, custom::CustomProvider, claude_code::ClaudeCodeProvider,
    opencode::OpenCodeProvider,
};

pub const PROVIDER_NAMES: &[&str] = &[
    "anthropic",
    "openai",
    "openai_responses",
    "openai_codex",
    "openrouter",
    "ollama",
    "lmstudio",
    "vllm",
    "groq",
    "deepseek",
    "doubao",
    "xai",
    "xai_responses",
    "google_gemini",
    "qwen",
    "kimi",
    "zhipu",
    "minimax",
    "opencode",
    "github_copilot",
    "custom",
    "claude_code",
];

pub fn create_provider(name: &str) -> Option<Box<dyn ProviderTrait>> {
    match name {
        "anthropic" => Some(Box::new(AnthropicProvider::default())),
        "openai" => Some(Box::new(OpenAIProvider::default())),
        "openai_responses" => Some(Box::new(OpenAIResponsesProvider::default())),
        "openai_codex" => Some(Box::new(OpenAICodexProvider::default())),
        "openrouter" => Some(Box::new(OpenRouterProvider::default())),
        "ollama" => Some(Box::new(OllamaProvider::default())),
        "lmstudio" => Some(Box::new(LMStudioProvider::default())),
        "vllm" => Some(Box::new(VLLMProvider::default())),
        "groq" => Some(Box::new(GroqProvider::default())),
        "deepseek" => Some(Box::new(DeepseekProvider::default())),
        "doubao" => Some(Box::new(DoubaoProvider::default())),
        "xai" => Some(Box::new(XAIProvider::default())),
        "xai_responses" => Some(Box::new(XAIResponsesProvider::default())),
        "google_gemini" => Some(Box::new(GoogleGeminiProvider::default())),
        "qwen" => Some(Box::new(QwenProvider::default())),
        "kimi" => Some(Box::new(KimiProvider::default())),
        "zhipu" => Some(Box::new(ZhipuProvider::default())),
        "minimax" => Some(Box::new(MiniMaxProvider::default())),
        "opencode" => Some(Box::new(OpenCodeProvider::default())),
        "github_copilot" => Some(Box::new(GitHubCopilotProvider::default())),
        "custom" => Some(Box::new(CustomProvider::default())),
        "claude_code" => Some(Box::new(ClaudeCodeProvider::default())),
        _ => None,
    }
}

pub struct ProviderRegistry {
    providers: Vec<Box<dyn ProviderTrait>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn add(&mut self, provider: Box<dyn ProviderTrait>) {
        let name = provider.name().to_string();
        if self.has_instance(&name) {
            self.remove(&name);
        }
        self.providers.push(provider);
    }

    pub fn remove(&mut self, instance_id: &str) -> Option<Box<dyn ProviderTrait>> {
        self.providers
            .iter()
            .position(|provider| provider.name() == instance_id)
            .map(|index| self.providers.remove(index))
    }

    pub fn has_instance(&self, instance_id: &str) -> bool {
        self.providers
            .iter()
            .any(|provider| provider.name() == instance_id)
    }

    #[allow(dead_code)]
    pub fn instances_for_base(&self, base_provider: &str) -> Vec<&dyn ProviderTrait> {
        self.iter()
            .filter(|(_, provider)| provider.base_provider_name() == base_provider)
            .map(|(_, provider)| provider)
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn ProviderTrait> {
        self.providers
            .iter()
            .find(|p| p.name() == name)
            .map(|p| p.as_ref())
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Box<dyn ProviderTrait>> {
        self.providers.iter_mut().find(|p| p.name() == name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &dyn ProviderTrait)> {
        self.providers.iter().map(|p| (p.name(), p.as_ref()))
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn load_providers_from_config(
    config_dir: &Path,
    http_client: &reqwest::Client,
) -> Result<ProviderRegistry, String> {
    let mut registry = ProviderRegistry::new();

    let providers_dir = config_dir.join("providers.d");
    if !providers_dir.exists() {
        return Ok(registry);
    }

    let mut entries = match tokio::fs::read_dir(&providers_dir).await {
        Ok(e) => e,
        Err(_) => return Ok(registry),
    };
    let mut seen_stems = HashSet::new();

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("yaml") && ext != Some("yml") {
            continue;
        }
        let instance_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let duplicate_key = instance_id.to_ascii_lowercase();
        if !seen_stems.insert(duplicate_key) {
            tracing::warn!(
                "Ignoring duplicate provider config stem '{}' at {}",
                instance_id,
                path.display()
            );
            continue;
        }

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to read provider config {}: {}", path.display(), e);
                continue;
            }
        };

        let yaml: serde_yaml::Value = match serde_yaml::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to parse provider config {}: {}", path.display(), e);
                continue;
            }
        };

        let identity = match provider_identity_from_yaml(instance_id, &yaml) {
            Ok(identity) => identity,
            Err(e) => {
                tracing::warn!("Ignoring provider config {}: {}", path.display(), e);
                continue;
            }
        };

        let mut provider = match create_provider(&identity.base_provider) {
            Some(provider) => provider,
            None => {
                tracing::warn!(
                    "Ignoring provider config {}: unknown base provider '{}'",
                    path.display(),
                    identity.base_provider
                );
                continue;
            }
        };

        if let Err(e) = provider.provider_settings_apply(yaml) {
            tracing::warn!("Failed to apply provider config {}: {}", path.display(), e);
            continue;
        }

        if identity.wrap_instance {
            provider = match identity.display_name {
                Some(display_name) => Box::new(ProviderInstance::new(
                    identity.instance_id,
                    identity.base_provider,
                    display_name,
                    provider,
                )),
                None => Box::new(ProviderInstance::from_inner(identity.instance_id, provider)),
            };
        }

        registry.add(provider);
    }

    for provider in registry.providers.iter_mut() {
        let instance_id = provider.name().to_string();
        if let Err(e) = provider
            .startup_refresh_and_sync(http_client, config_dir, &instance_id)
            .await
        {
            tracing::warn!(
                "Provider '{}' startup refresh failed: {}",
                provider.name(),
                e
            );
        }
    }

    Ok(registry)
}

#[allow(dead_code)]
pub async fn save_provider_config(
    config_dir: &Path,
    name: &str,
    settings: serde_yaml::Value,
) -> Result<(), String> {
    config_store::write_provider_config(config_dir, name, settings).await
}

pub async fn delete_provider_config(config_dir: &Path, name: &str) -> Result<(), String> {
    config_store::delete_provider_config(config_dir, name).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn write_provider_config(temp: &TempDir, file_name: &str, yaml: &str) {
        let providers_dir = temp.path().join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(providers_dir.join(file_name), yaml)
            .await
            .unwrap();
    }

    async fn load_registry(temp: &TempDir) -> ProviderRegistry {
        load_providers_from_config(temp.path(), &reqwest::Client::new())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn loads_multiple_openai_instances_from_config_files() {
        let temp = tempfile::tempdir().unwrap();
        write_provider_config(
            &temp,
            "openai.yaml",
            "api_key: sk-main\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;
        write_provider_config(
            &temp,
            "openai_2.yaml",
            "base_provider: openai\ndisplay_name: OpenAI 2\napi_key: sk-two\nenabled: true\nenabled_models:\n  - gpt-4.1-mini\n",
        )
        .await;
        write_provider_config(
            &temp,
            "openai_work.yaml",
            "base_provider: openai\ndisplay_name: Work OpenAI\napi_key: sk-work\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;

        let registry = load_registry(&temp).await;

        assert!(registry.has_instance("openai"));
        assert!(registry.has_instance("openai_2"));
        assert!(registry.has_instance("openai_work"));
        let openai_2 = registry.get("openai_2").unwrap();
        assert_eq!(openai_2.base_provider_name(), "openai");
        assert_eq!(openai_2.display_name(), "OpenAI 2");
        let openai_work = registry.get("openai_work").unwrap();
        assert_eq!(openai_work.base_provider_name(), "openai");
        assert_eq!(openai_work.display_name(), "Work OpenAI");
        assert_eq!(registry.instances_for_base("openai").len(), 3);
    }

    #[tokio::test]
    async fn legacy_singleton_loading_still_uses_builtin_provider() {
        let temp = tempfile::tempdir().unwrap();
        write_provider_config(
            &temp,
            "openai.yaml",
            "api_key: sk-main\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;

        let registry = load_registry(&temp).await;
        let provider = registry.get("openai").unwrap();

        assert_eq!(provider.name(), "openai");
        assert_eq!(provider.base_provider_name(), "openai");
        assert_eq!(provider.display_name(), "OpenAI");
    }

    #[tokio::test]
    async fn alias_without_base_provider_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        write_provider_config(
            &temp,
            "openai_2.yaml",
            "api_key: sk-two\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;

        let registry = load_registry(&temp).await;

        assert!(!registry.has_instance("openai_2"));
    }

    #[tokio::test]
    async fn alias_with_empty_base_provider_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        write_provider_config(
            &temp,
            "openai_2.yaml",
            "base_provider: ''\napi_key: sk-two\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;

        let registry = load_registry(&temp).await;

        assert!(!registry.has_instance("openai_2"));
    }

    #[tokio::test]
    async fn invalid_instance_ids_are_ignored() {
        let temp = tempfile::tempdir().unwrap();
        write_provider_config(
            &temp,
            "openai.bad.yaml",
            "base_provider: openai\napi_key: sk-bad\n",
        )
        .await;
        write_provider_config(
            &temp,
            "_openai.yaml",
            "base_provider: openai\napi_key: sk-bad\n",
        )
        .await;
        write_provider_config(
            &temp,
            "refact.yaml",
            "base_provider: openai\napi_key: sk-bad\n",
        )
        .await;

        let registry = load_registry(&temp).await;

        assert_eq!(registry.iter().count(), 0);
    }

    #[tokio::test]
    async fn case_insensitive_duplicate_stems_are_skipped() {
        let temp = tempfile::tempdir().unwrap();
        write_provider_config(
            &temp,
            "openai.yaml",
            "api_key: sk-main\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;
        write_provider_config(
            &temp,
            "OpenAI.yaml",
            "base_provider: openai\napi_key: sk-dupe\nenabled: true\nenabled_models:\n  - gpt-4.1-mini\n",
        )
        .await;

        let registry = load_registry(&temp).await;

        assert_eq!(registry.iter().count(), 1);
    }

    #[test]
    fn remove_returns_removed_provider() {
        let mut registry = ProviderRegistry::new();
        registry.add(create_provider("openai").unwrap());

        assert!(registry.has_instance("openai"));
        assert!(registry.remove("openai").is_some());
        assert!(!registry.has_instance("openai"));
    }
}
