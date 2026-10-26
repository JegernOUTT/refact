use std::sync::Arc;

use crate::global_context::GlobalContext;
use crate::providers::traits::ModelPricing;

pub use refact_pricing_core::{
    compute_cost, pricing_provider_aliases, standard_model_pricing_from_caps,
};

pub async fn lookup_model_pricing(
    gcx: &Arc<GlobalContext>,
    model_id: &str,
) -> Option<ModelPricing> {
    if let Some(custom_pricing) = lookup_provider_custom_pricing(gcx, model_id).await {
        return Some(custom_pricing);
    }

    let caps_state = gcx.caps_state.clone();
    if let Some(pricing) = caps_state
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
    gcx: &Arc<GlobalContext>,
    model_id: &str,
) -> Option<ModelPricing> {
    let (provider_name, model_name) = model_id.split_once('/')?;
    let registry = gcx.providers.read().await;
    registry
        .get(provider_name)
        .and_then(|provider| provider.custom_model_pricing(model_name))
}

#[cfg(test)]
mod tests {
    use super::*;
}
