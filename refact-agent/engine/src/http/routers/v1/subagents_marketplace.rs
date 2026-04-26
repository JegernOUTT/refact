use std::sync::Arc;

use axum::Extension;
use axum::extract::Query;
use axum::response::Json;
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::ext::extensions_marketplace::{
    InstallMarketplaceItemRequest, MarketplaceKind, install_marketplace_item,
    list_marketplace_items,
};
use crate::global_context::GlobalContext;

#[derive(Debug, Deserialize)]
pub struct SubagentsMarketplaceQuery {
    pub source: Option<String>,
    pub q: Option<String>,
}

pub async fn handle_v1_subagents_marketplace_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(params): Query<SubagentsMarketplaceQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (items, sources) = list_marketplace_items(gcx.clone(), MarketplaceKind::Subagent)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let q = params.q.unwrap_or_default().to_lowercase();
    let source = params.source.as_deref();
    let filtered: Vec<_> = items
        .into_iter()
        .filter(|item| {
            let source_ok = source.map(|s| item.source_id == s).unwrap_or(true);
            let q_ok = q.is_empty()
                || item.id.to_lowercase().contains(&q)
                || item.name.to_lowercase().contains(&q)
                || item.description.to_lowercase().contains(&q)
                || item.tags.iter().any(|tag| tag.to_lowercase().contains(&q));
            source_ok && q_ok
        })
        .collect();
    Ok(Json(json!({
        "items": filtered,
        "sources": sources,
    })))
}

pub async fn handle_v1_subagents_marketplace_install(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let req = serde_json::from_slice::<InstallMarketplaceItemRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;
    let installed = install_marketplace_item(gcx, MarketplaceKind::Subagent, req)
        .await
        .map_err(|e| {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else if e.contains("invalid scope") || e.contains("no project root") {
                StatusCode::BAD_REQUEST
            } else if e.contains("already exists") || e.contains("destination already exists") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            ScratchError::new(status, e)
        })?;
    Ok(Json(json!(installed)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::extensions_marketplace::configure_source;
    use crate::global_context::tests::make_test_gcx;

    #[tokio::test]
    async fn test_list_subagents_marketplace_includes_embedded_source() {
        let gcx = make_test_gcx().await;
        let config_dir = gcx.read().await.config_dir.clone();
        configure_source(&config_dir, "refact-starter-subagents", Some(true))
            .await
            .unwrap();

        let result = handle_v1_subagents_marketplace_get(
            Extension(gcx),
            Query(SubagentsMarketplaceQuery {
                source: None,
                q: None,
            }),
        )
        .await
        .unwrap();
        let value = result.0;
        assert!(!value["items"].as_array().unwrap().is_empty());
    }
}
