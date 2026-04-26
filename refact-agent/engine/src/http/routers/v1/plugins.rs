use std::sync::Arc;
use axum::Extension;
use axum::extract::Path;
use axum::response::Json;
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::ext::plugins::{
    add_marketplace, ensure_default_marketplaces, install_plugin, list_marketplace_plugins,
    load_plugins_db, remove_marketplace, uninstall_plugin, validate_plugin_name,
};
use crate::global_context::GlobalContext;

#[derive(Deserialize)]
pub struct AddMarketplaceRequest {
    pub source: String,
}

#[derive(Deserialize)]
pub struct InstallPluginRequest {
    pub plugin: String,
    pub marketplace: String,
}

pub async fn handle_list_marketplaces(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _ = ensure_default_marketplaces(gcx.clone()).await;
    let config_dir = gcx.read().await.config_dir.clone();
    let db = load_plugins_db(&config_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let summaries: Vec<Value> = db
        .marketplaces
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "source": m.source,
                "added_at": m.added_at,
            })
        })
        .collect();
    Ok(Json(json!({ "marketplaces": summaries })))
}

pub async fn handle_add_marketplace(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<AddMarketplaceRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;
    let mj = add_marketplace(gcx, &req.source).await.map_err(|e| {
        if e.contains("invalid") || e.contains("cannot") || e.contains("must match") {
            ScratchError::new(StatusCode::BAD_REQUEST, e)
        } else {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e)
        }
    })?;
    Ok(Json(json!({
        "name": mj.name,
        "plugin_count": mj.plugins.len(),
    })))
}

pub async fn handle_delete_marketplace(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if let Err(e) = validate_plugin_name(&name) {
        return Err((StatusCode::BAD_REQUEST, e));
    }
    remove_marketplace(gcx, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn handle_list_marketplace_plugins(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if let Err(e) = validate_plugin_name(&name) {
        return Err((StatusCode::BAD_REQUEST, e));
    }
    let plugins = list_marketplace_plugins(gcx, &name).await.map_err(|e| {
        if e.contains("not found") {
            (StatusCode::NOT_FOUND, e)
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, e)
        }
    })?;
    let plugins_json: Vec<Value> = plugins
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "description": p.description,
                "version": p.version,
                "tags": p.tags,
                "marketplace": name,
            })
        })
        .collect();
    Ok(Json(json!({ "plugins": plugins_json })))
}

pub async fn handle_install_plugin(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<InstallPluginRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;
    if let Err(e) = validate_plugin_name(&req.plugin) {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, e));
    }
    if let Err(e) = validate_plugin_name(&req.marketplace) {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, e));
    }
    let entry = install_plugin(gcx, &req.plugin, &req.marketplace)
        .await
        .map_err(|e| {
            if e.contains("not found") {
                ScratchError::new(StatusCode::NOT_FOUND, e)
            } else if e.contains("already installed") {
                ScratchError::new(StatusCode::CONFLICT, e)
            } else if e.contains("invalid") || e.contains("cannot") || e.contains("must match") {
                ScratchError::new(StatusCode::BAD_REQUEST, e)
            } else {
                ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e)
            }
        })?;
    Ok(Json(json!({
        "name": entry.name,
        "marketplace": entry.marketplace,
        "version": entry.version,
        "install_dir": entry.install_dir,
        "installed_at": entry.installed_at,
    })))
}

pub async fn handle_list_installed(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let config_dir = gcx.read().await.config_dir.clone();
    let db = load_plugins_db(&config_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "installed": db.installed })))
}

pub async fn handle_uninstall_plugin(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if let Err(e) = validate_plugin_name(&name) {
        return Err((StatusCode::BAD_REQUEST, e));
    }
    uninstall_plugin(gcx, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "deleted": true })))
}
