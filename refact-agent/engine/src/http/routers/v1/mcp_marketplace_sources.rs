#[cfg(test)]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use axum::Extension;
use axum::extract::Path;
use axum::response::Json;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

pub const BUNDLED_SOURCE_ID: &str = "refact-bundled";
pub const SMITHERY_SOURCE_ID: &str = "smithery";
pub const OFFICIAL_MCP_SOURCE_ID: &str = "official-mcp";
const SOURCES_FILENAME: &str = "marketplace_sources.yaml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    RefactIndex,
    Smithery,
    OfficialMcp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSource {
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub source_type: SourceType,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourcesConfig {
    pub sources: Vec<MarketplaceSource>,
}

pub fn bundled_source() -> MarketplaceSource {
    MarketplaceSource {
        id: BUNDLED_SOURCE_ID.to_string(),
        label: "Refact Built-in".to_string(),
        source_type: SourceType::RefactIndex,
        enabled: true,
        url: None,
        api_key: None,
    }
}

fn default_remote_source() -> MarketplaceSource {
    MarketplaceSource {
        id: "refact".to_string(),
        label: "Refact Curated".to_string(),
        source_type: SourceType::RefactIndex,
        enabled: true,
        url: Some("https://raw.githubusercontent.com/smallcloudai/refact/refs/heads/main/refact-agent/engine/src/yaml_configs/mcp_marketplace_index.json".to_string()),
        api_key: None,
    }
}

fn default_smithery_source() -> MarketplaceSource {
    MarketplaceSource {
        id: SMITHERY_SOURCE_ID.to_string(),
        label: "Smithery.ai".to_string(),
        source_type: SourceType::Smithery,
        enabled: false,
        url: None,
        api_key: None,
    }
}

fn sources_path(config_dir: &PathBuf) -> PathBuf {
    config_dir.join(SOURCES_FILENAME)
}

pub async fn load_sources(config_dir: &PathBuf) -> SourcesConfig {
    let path = sources_path(config_dir);
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(_) => return default_sources_config(),
    };
    match serde_yaml::from_str::<SourcesConfig>(&content) {
        Ok(cfg) => cfg,
        Err(_) => default_sources_config(),
    }
}

fn default_official_mcp_source() -> MarketplaceSource {
    MarketplaceSource {
        id: OFFICIAL_MCP_SOURCE_ID.to_string(),
        label: "MCP Registry".to_string(),
        source_type: SourceType::OfficialMcp,
        enabled: true,
        url: None,
        api_key: None,
    }
}

fn default_sources_config() -> SourcesConfig {
    SourcesConfig {
        sources: vec![
            default_remote_source(),
            default_official_mcp_source(),
            default_smithery_source(),
        ],
    }
}

async fn save_sources(config_dir: &PathBuf, config: &SourcesConfig) -> Result<(), String> {
    let path = sources_path(config_dir);
    let yaml = serde_yaml::to_string(config).map_err(|e| format!("serialize sources: {}", e))?;
    let tmp_path = path.with_extension("yaml.tmp");
    tokio::fs::write(&tmp_path, yaml.as_bytes())
        .await
        .map_err(|e| format!("write sources tmp: {}", e))?;
    #[cfg(target_os = "windows")]
    {
        let _ = tokio::fs::remove_file(&path).await;
    }
    tokio::fs::rename(&tmp_path, &path)
        .await
        .map_err(|e| format!("rename sources: {}", e))?;
    Ok(())
}

pub fn source_to_api_json(source: &MarketplaceSource, removable: bool) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".to_string(), json!(source.id));
    obj.insert("label".to_string(), json!(source.label));
    obj.insert(
        "type".to_string(),
        json!(match source.source_type {
            SourceType::RefactIndex => "refact_index",
            SourceType::Smithery => "smithery",
            SourceType::OfficialMcp => "official_mcp",
        }),
    );
    obj.insert("enabled".to_string(), json!(source.enabled));
    obj.insert("removable".to_string(), json!(removable));
    if let Some(ref url) = source.url {
        obj.insert("url".to_string(), json!(url));
    }
    if source.source_type == SourceType::Smithery {
        let has_key = source
            .api_key
            .as_ref()
            .map(|k| !k.is_empty())
            .unwrap_or(false);
        obj.insert("needs_api_key".to_string(), json!(true));
        obj.insert("has_api_key".to_string(), json!(has_key));
    }
    Value::Object(obj)
}

pub async fn handle_v1_mcp_marketplace_sources_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let config_dir = gcx.read().await.config_dir.clone();
    let cfg = load_sources(&config_dir).await;

    let mut sources_json = vec![source_to_api_json(&bundled_source(), false)];
    for source in &cfg.sources {
        sources_json.push(source_to_api_json(source, true));
    }

    Ok(Json(json!({ "sources": sources_json })))
}

#[derive(Deserialize)]
pub struct AddSourceRequest {
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

pub async fn handle_v1_mcp_marketplace_sources_post(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<AddSourceRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    if req.id.is_empty() || req.id == BUNDLED_SOURCE_ID {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "invalid source id".to_string(),
        ));
    }

    let source_type = match req.source_type.as_str() {
        "refact_index" => SourceType::RefactIndex,
        "smithery" => SourceType::Smithery,
        "official_mcp" => SourceType::OfficialMcp,
        _ => {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!("unknown source type: {}", req.source_type),
            ))
        }
    };

    if source_type == SourceType::RefactIndex && req.url.is_none() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "refact_index source requires url".to_string(),
        ));
    }

    let config_dir = gcx.read().await.config_dir.clone();
    let mut cfg = load_sources(&config_dir).await;

    let new_source = MarketplaceSource {
        id: req.id.clone(),
        label: req.label,
        source_type,
        enabled: req.enabled,
        url: req.url,
        api_key: None,
    };

    if let Some(existing) = cfg.sources.iter_mut().find(|s| s.id == req.id) {
        *existing = new_source;
    } else {
        cfg.sources.push(new_source);
    }

    save_sources(&config_dir, &cfg)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(json!({ "ok": true })))
}

pub async fn handle_v1_mcp_marketplace_sources_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(source_id): Path<String>,
) -> Result<Json<Value>, ScratchError> {
    if source_id == BUNDLED_SOURCE_ID {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "cannot remove built-in source".to_string(),
        ));
    }

    let config_dir = gcx.read().await.config_dir.clone();
    let mut cfg = load_sources(&config_dir).await;

    let before = cfg.sources.len();
    cfg.sources.retain(|s| s.id != source_id);
    if cfg.sources.len() == before {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("source '{}' not found", source_id),
        ));
    }

    save_sources(&config_dir, &cfg)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct ConfigureSourceRequest {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

pub async fn handle_v1_mcp_marketplace_sources_configure(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(source_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<ConfigureSourceRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    if source_id == BUNDLED_SOURCE_ID {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "cannot configure built-in source".to_string(),
        ));
    }

    let config_dir = gcx.read().await.config_dir.clone();
    let mut cfg = load_sources(&config_dir).await;

    let source = cfg
        .sources
        .iter_mut()
        .find(|s| s.id == source_id)
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("source '{}' not found", source_id),
            )
        })?;

    if let Some(key) = req.api_key {
        source.api_key = if key.is_empty() { None } else { Some(key) };
    }
    if let Some(enabled) = req.enabled {
        source.enabled = enabled;
    }

    save_sources(&config_dir, &cfg)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(json!({ "ok": true })))
}

pub async fn get_all_sources(config_dir: &PathBuf) -> (MarketplaceSource, Vec<MarketplaceSource>) {
    let cfg = load_sources(config_dir).await;
    (bundled_source(), cfg.sources)
}

pub fn smithery_api_key(sources: &[MarketplaceSource]) -> Option<String> {
    sources
        .iter()
        .find(|s| s.id == SMITHERY_SOURCE_ID && s.enabled)
        .and_then(|s| s.api_key.clone())
        .filter(|k| !k.is_empty())
}

#[cfg(test)]
pub fn default_sources_config_for_test() -> SourcesConfig {
    default_sources_config()
}

#[cfg(test)]
pub fn get_source_map(
    bundled: &MarketplaceSource,
    sources: &[MarketplaceSource],
) -> HashMap<String, MarketplaceSource> {
    let mut map = HashMap::new();
    map.insert(bundled.id.clone(), bundled.clone());
    for s in sources {
        map.insert(s.id.clone(), s.clone());
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_sources_include_official_mcp() {
        let cfg = default_sources_config_for_test();
        let official = cfg.sources.iter().find(|s| s.id == OFFICIAL_MCP_SOURCE_ID);
        assert!(
            official.is_some(),
            "official-mcp must be in default sources"
        );
        let official = official.unwrap();
        assert!(official.enabled, "official-mcp must be enabled by default");
        assert_eq!(official.source_type, SourceType::OfficialMcp);
        assert!(
            official.api_key.is_none(),
            "official-mcp must not need an api key"
        );
    }

    #[test]
    fn test_official_mcp_source_json_type_field() {
        let source = MarketplaceSource {
            id: OFFICIAL_MCP_SOURCE_ID.to_string(),
            label: "MCP Registry".to_string(),
            source_type: SourceType::OfficialMcp,
            enabled: true,
            url: None,
            api_key: None,
        };
        let json = source_to_api_json(&source, false);
        assert_eq!(json["type"], "official_mcp");
        assert!(json.get("needs_api_key").is_none());
        assert!(json.get("has_api_key").is_none());
    }

    #[tokio::test]
    async fn test_source_config_persistence() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_path_buf();

        let initial = load_sources(&config_dir).await;
        assert!(
            initial.sources.iter().any(|s| s.id == "refact"),
            "should have refact source by default"
        );

        let mut cfg = initial.clone();
        cfg.sources.push(MarketplaceSource {
            id: "custom".to_string(),
            label: "Custom".to_string(),
            source_type: SourceType::RefactIndex,
            enabled: true,
            url: Some("https://example.com/index.json".to_string()),
            api_key: None,
        });
        save_sources(&config_dir, &cfg).await.unwrap();

        let reloaded = load_sources(&config_dir).await;
        assert!(
            reloaded.sources.iter().any(|s| s.id == "custom"),
            "custom source should persist"
        );
    }

    #[test]
    fn test_bundled_source_not_removable() {
        let bundled = bundled_source();
        assert_eq!(bundled.id, BUNDLED_SOURCE_ID);
        let json = source_to_api_json(&bundled, false);
        assert_eq!(json["removable"], false);
    }

    #[test]
    fn test_smithery_source_needs_api_key() {
        let smithery = default_smithery_source();
        let json = source_to_api_json(&smithery, true);
        assert_eq!(json["needs_api_key"], true);
        assert_eq!(json["has_api_key"], false);
    }

    #[test]
    fn test_smithery_api_key_extraction() {
        let mut sources = vec![default_smithery_source()];
        assert!(
            smithery_api_key(&sources).is_none(),
            "no key when no api_key set"
        );

        sources[0].api_key = Some("sk-test".to_string());
        sources[0].enabled = true;
        assert_eq!(smithery_api_key(&sources), Some("sk-test".to_string()));

        sources[0].enabled = false;
        assert!(smithery_api_key(&sources).is_none(), "no key when disabled");
    }

    #[test]
    fn test_get_source_map() {
        let bundled = bundled_source();
        let extra = MarketplaceSource {
            id: "extra".to_string(),
            label: "Extra".to_string(),
            source_type: SourceType::RefactIndex,
            enabled: true,
            url: Some("https://example.com".to_string()),
            api_key: None,
        };
        let map = get_source_map(&bundled, &[extra]);
        assert!(map.contains_key(BUNDLED_SOURCE_ID));
        assert!(map.contains_key("extra"));
        assert_eq!(map.len(), 2);
    }

    #[tokio::test]
    async fn test_save_sources_overwrites_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_path_buf();

        let cfg1 = SourcesConfig {
            sources: vec![MarketplaceSource {
                id: "first".to_string(),
                label: "First".to_string(),
                source_type: SourceType::RefactIndex,
                enabled: true,
                url: Some("https://example.com/first.json".to_string()),
                api_key: None,
            }],
        };
        save_sources(&config_dir, &cfg1).await.unwrap();

        let loaded1 = load_sources(&config_dir).await;
        assert!(
            loaded1.sources.iter().any(|s| s.id == "first"),
            "first save should persist"
        );

        let cfg2 = SourcesConfig {
            sources: vec![MarketplaceSource {
                id: "second".to_string(),
                label: "Second".to_string(),
                source_type: SourceType::RefactIndex,
                enabled: true,
                url: Some("https://example.com/second.json".to_string()),
                api_key: None,
            }],
        };
        save_sources(&config_dir, &cfg2).await.unwrap();

        let loaded2 = load_sources(&config_dir).await;
        assert!(
            loaded2.sources.iter().any(|s| s.id == "second"),
            "second save should persist"
        );
        assert!(
            !loaded2.sources.iter().any(|s| s.id == "first"),
            "first entry must be overwritten"
        );
    }
}
