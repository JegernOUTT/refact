use std::sync::Arc;

use reqwest::header::USER_AGENT;
use tracing::warn;

use crate::global_context::GlobalContext;

pub use refact_core::models_dev::{
    cost_to_pricing, get_model, get_provider, load_models_dev_catalog_from_cache_or_snapshot,
    load_models_dev_snapshot_catalog, model_cost_to_pricing, models_dev_cache_path,
    models_dev_catalog_to_model_caps, parse_catalog_json, validate_required_project_providers,
    write_models_dev_cache, ModelsDevCatalog, ModelsDevCost, ModelsDevCostTier, ModelsDevLimit,
    ModelsDevModel, ModelsDevModelProvider, ModelsDevModalities, ModelsDevProvider,
    MODELS_DEV_API_URL,
};

pub async fn load_models_dev_catalog(
    gcx: Arc<GlobalContext>,
    force_refresh: bool,
) -> Result<ModelsDevCatalog, String> {
    let (cache_dir, http_client) = {
        (gcx.cache_dir.clone(), gcx.http_client.clone())
    };

    if force_refresh {
        match fetch_models_dev_catalog(&http_client).await {
            Ok((catalog, body)) => {
                if let Err(e) = write_models_dev_cache(&cache_dir, &body).await {
                    warn!("Failed to write models.dev runtime cache: {e}");
                }
                Ok(catalog)
            }
            Err(e) => {
                warn!("Failed to refresh models.dev catalog: {e}; using cache or snapshot");
                load_models_dev_catalog_from_cache_or_snapshot(&cache_dir).await
            }
        }
    } else {
        load_models_dev_catalog_from_cache_or_snapshot(&cache_dir).await
    }
}

async fn fetch_models_dev_catalog(
    http_client: &reqwest::Client,
) -> Result<(ModelsDevCatalog, String), String> {
    use std::time::Duration;
    const FETCH_TIMEOUT_SECS: u64 = 10;
    tokio::time::timeout(Duration::from_secs(FETCH_TIMEOUT_SECS), async {
        let response = http_client
            .get(MODELS_DEV_API_URL)
            .header(USER_AGENT, "refact-lsp models.dev catalog")
            .send()
            .await
            .map_err(|e| format!("Failed to request models.dev catalog: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("models.dev catalog returned HTTP {status}"));
        }
        let body = read_models_dev_response_body(response).await?;
        let catalog = parse_catalog_json(&body).map_err(|e| {
            format!("models.dev live catalog is invalid: {e}")
        })?;
        validate_required_project_providers(&catalog)
            .map_err(|e| format!("models.dev live catalog is incomplete: {e}"))?;
        Ok((catalog, body))
    })
    .await
    .map_err(|_| "Timed out fetching models.dev catalog".to_string())?
}

async fn read_models_dev_response_body(mut response: reqwest::Response) -> Result<String, String> {
    const MODELS_DEV_MAX_CATALOG_BYTES: usize = 25 * 1024 * 1024;
    if let Some(content_length) = response.content_length() {
        let len = usize::try_from(content_length).map_err(|_| {
            format!("models.dev catalog is too large: {content_length} bytes exceeds {MODELS_DEV_MAX_CATALOG_BYTES} byte limit")
        })?;
        if len > MODELS_DEV_MAX_CATALOG_BYTES {
            return Err(format!(
                "models.dev catalog is too large: {len} bytes exceeds {MODELS_DEV_MAX_CATALOG_BYTES} byte limit"
            ));
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Failed to read models.dev catalog response: {e}"))?
    {
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "models.dev catalog response is too large".to_string())?;
        if next_len > MODELS_DEV_MAX_CATALOG_BYTES {
            return Err(format!(
                "models.dev catalog is too large: {next_len} bytes exceeds {MODELS_DEV_MAX_CATALOG_BYTES} byte limit"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|e| format!("models.dev catalog response is not UTF-8: {e}"))
}
