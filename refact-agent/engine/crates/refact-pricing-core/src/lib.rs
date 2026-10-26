use std::collections::HashMap;

use refact_core::chat_types::{ChatUsage, MeteringUsd};
use refact_core::model_caps::{resolve_model_caps, ModelCapabilities};
use refact_core::provider_types::ModelPricing;

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

pub fn pricing_provider_aliases(provider_name: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::provider_types::ModelPricingTier;

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
