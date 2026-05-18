use std::collections::HashMap;
use std::sync::Arc;


use crate::caps::models_dev::load_models_dev_catalog;
use crate::global_context::GlobalContext;

pub use refact_core::model_caps::{
    CachingType, CanonicalNameParts, ModelCapsSource, ModelCapabilities, ResolvedCaps,
    canonicalize_model_name, is_model_supported, model_caps_from_models_dev_catalog,
    model_caps_pricing_metadata, resolve_model_caps, validate_model_caps,
};

pub async fn get_model_caps(
    gcx: Arc<GlobalContext>,
    force_refresh: bool,
) -> Result<HashMap<String, ModelCapabilities>, String> {
    let catalog = load_models_dev_catalog(gcx, force_refresh)
        .await
        .map_err(|e| format!("Failed to load models.dev model capabilities: {e}"))?;
    model_caps_from_models_dev_catalog(&catalog)
}
