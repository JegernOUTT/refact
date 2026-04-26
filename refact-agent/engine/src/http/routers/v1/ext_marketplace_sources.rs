use std::sync::Arc;

use axum::Extension;
use axum::extract::Path;
use axum::response::Json;
use hyper::StatusCode;
use serde_json::json;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::ext::extensions_marketplace::{
    ConfigureMarketplaceSourceRequest, ExtensionsMarketplaceSource, MarketplaceKind,
    MarketplaceParserMode, MarketplaceSourceKind, SaveMarketplaceSourceRequest, configure_source,
    delete_user_source, load_all_sources, normalize_github_source, refresh_source_cache,
    save_user_source, source_id_from_repo,
};
use crate::global_context::GlobalContext;

pub async fn handle_v1_ext_marketplace_sources_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let config_dir = gcx.read().await.config_dir.clone();
    let sources = load_all_sources(&config_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "sources": sources })))
}

pub async fn handle_v1_ext_marketplace_sources_post(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let req = serde_json::from_slice::<SaveMarketplaceSourceRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;
    let (owner_repo, canonical_url) = normalize_github_source(&req.url)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    let id = source_id_from_repo(&owner_repo);
    let label = owner_repo.clone();
    let source = ExtensionsMarketplaceSource {
        id: id.clone(),
        label,
        description: format!("GitHub source {}", owner_repo),
        enabled: req.enabled,
        builtin: false,
        removable: true,
        source_kind: MarketplaceSourceKind::UserGithub,
        repo_url: Some(canonical_url),
        supported_kinds: vec![
            MarketplaceKind::Skill,
            MarketplaceKind::Command,
            MarketplaceKind::Subagent,
        ],
        parser_mode: MarketplaceParserMode::Manifest,
        last_sync_at: None,
        error: None,
    };

    let config_dir = gcx.read().await.config_dir.clone();
    let saved = save_user_source(&config_dir, source)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "ok": true, "source": saved })))
}

pub async fn handle_v1_ext_marketplace_sources_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let config_dir = gcx.read().await.config_dir.clone();
    delete_user_source(&config_dir, &id).await.map_err(|e| {
        let status = if e.contains("cannot delete") {
            StatusCode::BAD_REQUEST
        } else if e.contains("not found") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        ScratchError::new(status, e)
    })?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn handle_v1_ext_marketplace_sources_configure(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let req = serde_json::from_slice::<ConfigureMarketplaceSourceRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;
    let config_dir = gcx.read().await.config_dir.clone();
    configure_source(&config_dir, &id, req.enabled)
        .await
        .map_err(|e| {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            ScratchError::new(status, e)
        })?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn handle_v1_ext_marketplace_sources_refresh(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let (config_dir, cache_dir) = {
        let g = gcx.read().await;
        (g.config_dir.clone(), g.cache_dir.clone())
    };
    refresh_source_cache(&config_dir, &cache_dir, &id)
        .await
        .map_err(|e| {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else if e.contains("cannot be refreshed") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            ScratchError::new(status, e)
        })?;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::extensions_marketplace::load_all_sources;
    use crate::global_context::tests::make_test_gcx;

    #[tokio::test]
    async fn test_add_marketplace_source_persists() {
        let gcx = make_test_gcx().await;
        let body = serde_json::to_vec(&serde_json::json!({
            "url": "https://github.com/example/repo",
            "enabled": true,
        }))
        .unwrap();

        let result = handle_v1_ext_marketplace_sources_post(
            Extension(gcx.clone()),
            hyper::body::Bytes::from(body),
        )
        .await;
        assert!(result.is_ok());

        let config_dir = gcx.read().await.config_dir.clone();
        let sources = load_all_sources(&config_dir).await.unwrap();
        assert!(sources.iter().any(|s| s.id == "example-repo"));
    }

    #[tokio::test]
    async fn test_add_marketplace_source_rejects_non_root_url() {
        let gcx = make_test_gcx().await;
        let body = serde_json::to_vec(&serde_json::json!({
            "url": "https://github.com/example/repo/tree/main",
            "enabled": true,
        }))
        .unwrap();

        let result =
            handle_v1_ext_marketplace_sources_post(Extension(gcx), hyper::body::Bytes::from(body))
                .await;
        assert!(result.is_err());
    }
}
