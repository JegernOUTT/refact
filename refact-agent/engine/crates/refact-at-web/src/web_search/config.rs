use std::{collections::BTreeMap, time::Duration};
use super::provider_catalog;
#[derive(Debug, Clone)]
pub struct ProviderRuntimeConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
}
impl Default for ProviderRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct WebSearchConfig {
    pub providers: BTreeMap<String, ProviderRuntimeConfig>,
    pub searxng_url: Option<String>,
    pub deadline: Duration,
    pub provider_timeout: Duration,
}
impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            providers: provider_catalog()
                .iter()
                .map(|p| {
                    (
                        p.id.into(),
                        ProviderRuntimeConfig {
                            enabled: p.default_enabled,
                            api_key: None,
                        },
                    )
                })
                .collect(),
            searxng_url: None,
            deadline: Duration::from_secs(12),
            provider_timeout: Duration::from_secs(6),
        }
    }
}
impl WebSearchConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        for p in provider_catalog() {
            if let Some(c) = config.providers.get_mut(p.id) {
                c.api_key = p.key_env_vars.iter().find_map(|key| {
                    std::env::var(key)
                        .ok()
                        .map(|v| v.trim().to_owned())
                        .filter(|v| !v.is_empty())
                });
            }
        }
        config.searxng_url = std::env::var("SEARXNG_URL")
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty());
        config
    }
}
