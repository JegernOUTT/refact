use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{ChatUsage, MeteringUsd};
use crate::caps::model_caps::{resolve_model_caps, ModelCapabilities};
use crate::global_context::GlobalContext;
use crate::providers::traits::ModelPricing;

pub fn compute_cost(usage: &ChatUsage, pricing: &ModelPricing) -> Option<MeteringUsd> {
    if !pricing.is_valid() {
        return None;
    }

    let billable_input_tokens = usage
        .prompt_tokens
        .saturating_add(usage.cache_read_tokens.unwrap_or(0))
        .saturating_add(usage.cache_creation_tokens.unwrap_or(0));
    let long_context_tier = (billable_input_tokens > 200_000)
        .then(|| pricing.context_over_200k.as_ref())
        .flatten();

    let prompt_rate = long_context_tier
        .and_then(|tier| tier.prompt)
        .unwrap_or(pricing.prompt);
    let generated_rate = long_context_tier
        .and_then(|tier| tier.generated)
        .unwrap_or(pricing.generated);
    let cache_read_rate = long_context_tier
        .and_then(|tier| tier.cache_read)
        .or(pricing.cache_read);
    let cache_creation_rate = long_context_tier
        .and_then(|tier| tier.cache_creation)
        .or(pricing.cache_creation);

    let prompt_usd = (usage.prompt_tokens as f64) * prompt_rate / 1_000_000.0;
    let generated_usd = (usage.completion_tokens as f64) * generated_rate / 1_000_000.0;

    let cache_read_usd = match (usage.cache_read_tokens, cache_read_rate) {
        (Some(tokens), Some(rate)) => Some((tokens as f64) * rate / 1_000_000.0),
        _ => None,
    };

    let cache_creation_usd = match (usage.cache_creation_tokens, cache_creation_rate) {
        (Some(tokens), Some(rate)) => Some((tokens as f64) * rate / 1_000_000.0),
        _ => None,
    };

    let total_usd = prompt_usd
        + generated_usd
        + cache_read_usd.unwrap_or(0.0)
        + cache_creation_usd.unwrap_or(0.0);

    Some(MeteringUsd {
        prompt_usd,
        generated_usd,
        cache_read_usd,
        cache_creation_usd,
        total_usd,
    })
}

pub fn standard_model_pricing_from_caps(
    model_caps: &HashMap<String, ModelCapabilities>,
    model_id: &str,
) -> Option<ModelPricing> {
    if let Some(pricing) =
        resolve_model_caps(model_caps, model_id).and_then(|resolved| resolved.caps.pricing)
    {
        return Some(pricing);
    }

    let Some((provider_name, bare_model_id)) = model_id.split_once('/') else {
        return None;
    };
    for provider_alias in pricing_provider_aliases(provider_name) {
        let qualified = format!("{provider_alias}/{bare_model_id}");
        if let Some(pricing) =
            resolve_model_caps(model_caps, &qualified).and_then(|resolved| resolved.caps.pricing)
        {
            return Some(pricing);
        }
    }

    resolve_model_caps(model_caps, bare_model_id).and_then(|resolved| resolved.caps.pricing)
}

fn pricing_provider_aliases(provider_name: &str) -> Vec<String> {
    let mut aliases = vec![provider_name.replace('_', "-")];
    for suffix in ["_responses", "-responses"] {
        if let Some(stripped) = provider_name.strip_suffix(suffix) {
            aliases.push(stripped.to_string());
            aliases.push(stripped.replace('_', "-"));
        }
    }
    if provider_name == "google_gemini" {
        aliases.push("google".to_string());
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

pub async fn lookup_model_pricing(
    gcx: &Arc<ARwLock<GlobalContext>>,
    model_id: &str,
) -> Option<ModelPricing> {
    if let Some(custom_pricing) = lookup_provider_custom_pricing(gcx, model_id).await {
        return Some(custom_pricing);
    }

    if let Some(pricing) = gcx
        .read()
        .await
        .caps
        .as_ref()
        .and_then(|caps| standard_model_pricing_from_caps(&caps.model_caps, model_id))
    {
        return Some(pricing);
    }

    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .ok()?;
    standard_model_pricing_from_caps(&caps.model_caps, model_id)
}

async fn lookup_provider_custom_pricing(
    gcx: &Arc<ARwLock<GlobalContext>>,
    model_id: &str,
) -> Option<ModelPricing> {
    let (provider_name, model_name) = model_id.split_once('/')?;
    let gcx_locked = gcx.read().await;
    let registry = gcx_locked.providers.read().await;
    registry
        .get(provider_name)
        .and_then(|provider| provider.custom_model_pricing(model_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::model_caps::model_caps_from_models_dev_catalog;
    use crate::caps::models_dev::load_models_dev_snapshot_catalog;
    use crate::caps::CodeAssistantCaps;
    use crate::providers::traits::{CustomModelConfig, ModelPricingTier};

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn caps_with_model_caps(
        model_caps: HashMap<String, ModelCapabilities>,
    ) -> Arc<CodeAssistantCaps> {
        Arc::new(CodeAssistantCaps {
            model_caps: Arc::new(model_caps),
            ..Default::default()
        })
    }

    #[test]
    fn test_models_dev_standard_provider_pricing_resolves_from_caps() {
        let catalog = load_models_dev_snapshot_catalog().unwrap();
        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        for model in [
            "openai/gpt-4o",
            "anthropic/claude-3-5-sonnet-20241022",
            "deepseek/deepseek-chat",
        ] {
            let pricing = standard_model_pricing_from_caps(&caps, model)
                .unwrap_or_else(|| panic!("missing pricing for {model}"));
            assert!(pricing.is_valid());
            assert!(pricing.prompt > 0.0);
            assert!(pricing.generated > 0.0);
        }
    }

    #[test]
    fn test_models_dev_standard_provider_pricing_uses_provider_aliases() {
        let catalog = load_models_dev_snapshot_catalog().unwrap();
        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        let openai = standard_model_pricing_from_caps(&caps, "openai/gpt-4o").unwrap();
        let responses = standard_model_pricing_from_caps(&caps, "openai_responses/gpt-4o").unwrap();
        let gemini =
            standard_model_pricing_from_caps(&caps, "google_gemini/gemini-1.5-flash").unwrap();

        assert_eq!(responses.prompt, openai.prompt);
        assert!(gemini.is_valid());
    }

    #[tokio::test]
    async fn test_models_dev_custom_pricing_overrides_caps_pricing() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let catalog = load_models_dev_snapshot_catalog().unwrap();
        let model_caps = model_caps_from_models_dev_catalog(&catalog).unwrap();
        let mut provider = crate::providers::create_provider("openai").unwrap();
        provider.add_custom_model(
            "gpt-4o".to_string(),
            CustomModelConfig {
                pricing: Some(ModelPricing {
                    prompt: 101.0,
                    generated: 202.0,
                    cache_read: Some(303.0),
                    cache_creation: Some(404.0),
                    context_over_200k: None,
                }),
                ..Default::default()
            },
        );
        let providers = { gcx.read().await.providers.clone() };
        providers.write().await.add(provider);
        {
            let mut gcx = gcx.write().await;
            gcx.caps = Some(caps_with_model_caps(model_caps));
            gcx.caps_last_attempted_ts = now_secs();
        }

        let pricing = lookup_model_pricing(&gcx, "openai/gpt-4o").await.unwrap();

        assert_eq!(pricing.prompt, 101.0);
        assert_eq!(pricing.generated, 202.0);
        assert_eq!(pricing.cache_read, Some(303.0));
        assert_eq!(pricing.cache_creation, Some(404.0));
    }

    #[tokio::test]
    async fn test_models_dev_lookup_pricing_falls_back_to_caps() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let catalog = load_models_dev_snapshot_catalog().unwrap();
        let model_caps = model_caps_from_models_dev_catalog(&catalog).unwrap();
        {
            let mut gcx = gcx.write().await;
            gcx.caps = Some(caps_with_model_caps(model_caps));
            gcx.caps_last_attempted_ts = now_secs();
        }

        let pricing = lookup_model_pricing(&gcx, "openai/gpt-4o").await.unwrap();

        assert!(pricing.is_valid());
        assert!(pricing.prompt > 0.0);
        assert!(pricing.generated > 0.0);
    }

    #[test]
    fn test_compute_cost_uses_long_context_tier_above_200k_input_tokens() {
        let usage = ChatUsage {
            prompt_tokens: 199_000,
            completion_tokens: 1_000,
            total_tokens: 205_000,
            cache_read_tokens: Some(6_000),
            cache_creation_tokens: None,
            metering_usd: None,
        };
        let pricing = ModelPricing {
            prompt: 2.0,
            generated: 8.0,
            cache_read: Some(0.5),
            cache_creation: None,
            context_over_200k: Some(ModelPricingTier {
                prompt: Some(4.0),
                generated: Some(16.0),
                cache_read: Some(1.0),
                cache_creation: None,
            }),
        };

        let metering = compute_cost(&usage, &pricing).unwrap();

        assert_eq!(metering.prompt_usd, 199_000.0 * 4.0 / 1_000_000.0);
        assert_eq!(metering.generated_usd, 1_000.0 * 16.0 / 1_000_000.0);
        assert_eq!(metering.cache_read_usd, Some(6_000.0 * 1.0 / 1_000_000.0));
    }

    #[test]
    fn test_compute_cost_uses_base_tier_at_200k_input_tokens() {
        let usage = ChatUsage {
            prompt_tokens: 200_000,
            completion_tokens: 1_000,
            total_tokens: 201_000,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            metering_usd: None,
        };
        let pricing = ModelPricing {
            prompt: 2.0,
            generated: 8.0,
            context_over_200k: Some(ModelPricingTier {
                prompt: Some(4.0),
                generated: Some(16.0),
                ..Default::default()
            }),
            ..Default::default()
        };

        let metering = compute_cost(&usage, &pricing).unwrap();

        assert_eq!(metering.prompt_usd, 200_000.0 * 2.0 / 1_000_000.0);
        assert_eq!(metering.generated_usd, 1_000.0 * 8.0 / 1_000_000.0);
    }
}
