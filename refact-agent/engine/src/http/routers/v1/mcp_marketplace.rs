use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use axum::Extension;
use axum::extract::Query;
use axum::response::Json;
use hyper::StatusCode;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock as ARwLock;
use tokio::time::{Duration, Instant};
use std::sync::Mutex;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::mcp::mcp_naming;
use crate::http::routers::v1::mcp_marketplace_sources::{
    load_sources, get_all_sources, smithery_api_key, source_to_api_json, BUNDLED_SOURCE_ID,
    SourceType, MarketplaceSource,
};
#[cfg(test)]
use crate::http::routers::v1::mcp_marketplace_sources::{SMITHERY_SOURCE_ID, OFFICIAL_MCP_SOURCE_ID};

const BUNDLED_CACHE_TTL_SECS: u64 = 3600;
const SMITHERY_CACHE_TTL_SECS: u64 = 900;
const OFFICIAL_MCP_CACHE_TTL_SECS: u64 = 900;

const OFFICIAL_MCP_REGISTRY_URL: &str = "https://registry.modelcontextprotocol.io/v0/servers";

static SOURCE_CACHES: Mutex<Option<HashMap<String, (Instant, Vec<MarketplaceServerWithSource>)>>> =
    Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRecipe {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Env keys whose values should be appended as positional CLI arguments to `command`.
    /// The env values are still written to the generated YAML `env:` section (for UI prompting),
    /// but are also appended in order as extra positional args at the end of the command string.
    #[serde(default)]
    pub args_from_env: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceServer {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub transport: String,
    pub install_recipe: InstallRecipe,
    #[serde(default)]
    pub confirmation_default: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceServerWithSource {
    #[serde(flatten)]
    pub server: MarketplaceServer,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceIndex {
    pub version: u32,
    pub updated_at: String,
    pub servers: Vec<MarketplaceServer>,
}

fn bundled_index() -> MarketplaceIndex {
    serde_json::from_str(include_str!(
        "../../../yaml_configs/mcp_marketplace_index.json"
    ))
    .expect("bundled MCP marketplace index must be valid JSON")
}

fn get_cache() -> HashMap<String, (Instant, Vec<MarketplaceServerWithSource>)> {
    SOURCE_CACHES.lock().unwrap().clone().unwrap_or_default()
}

fn set_cache(cache: HashMap<String, (Instant, Vec<MarketplaceServerWithSource>)>) {
    *SOURCE_CACHES.lock().unwrap() = Some(cache);
}

async fn fetch_refact_index(http_client: &reqwest::Client, url: &str) -> Option<MarketplaceIndex> {
    let resp = http_client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<MarketplaceIndex>().await.ok()
}

#[derive(Deserialize)]
struct SmitheryListResponse {
    servers: Vec<SmitheryServer>,
    pagination: SmitheryPagination,
}

#[derive(Deserialize)]
struct SmitheryServer {
    #[serde(rename = "qualifiedName")]
    qualified_name: String,
    #[serde(rename = "displayName")]
    display_name: String,
    description: String,
    #[serde(rename = "iconUrl")]
    icon_url: Option<String>,
    homepage: Option<String>,
    verified: Option<bool>,
    remote: Option<bool>,
}

#[derive(Deserialize)]
struct SmitheryPagination {
    #[serde(rename = "totalCount")]
    total_count: u32,
}

async fn fetch_smithery_servers(
    http_client: &reqwest::Client,
    api_key: &str,
    query: Option<&str>,
    page: u32,
    page_size: u32,
) -> Result<(Vec<MarketplaceServer>, u32), String> {
    let mut url = format!(
        "https://registry.smithery.ai/servers?page={}&pageSize={}",
        page, page_size
    );
    if let Some(q) = query {
        if !q.is_empty() {
            url.push_str(&format!("&q={}", utf8_percent_encode(q, NON_ALPHANUMERIC)));
        }
    }

    let resp = http_client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("smithery request failed: {}", e))?;

    if resp.status() == 401 {
        return Err("smithery: invalid API key".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("smithery: HTTP {}", resp.status()));
    }

    let data: SmitheryListResponse = resp
        .json()
        .await
        .map_err(|e| format!("smithery parse: {}", e))?;

    let servers: Vec<MarketplaceServer> = data
        .servers
        .into_iter()
        .map(|s| {
            let transport = if s.remote.unwrap_or(false) {
                "http"
            } else {
                "stdio"
            }
            .to_string();
            let publisher = s.qualified_name.split('/').next().unwrap_or("").to_string();
            let mut tags = vec!["smithery".to_string()];
            if s.verified.unwrap_or(false) {
                tags.push("verified".to_string());
            }
            MarketplaceServer {
                id: s.qualified_name,
                name: s.display_name,
                description: s.description,
                publisher,
                tags,
                icon_url: s.icon_url,
                homepage: s.homepage,
                transport,
                install_recipe: InstallRecipe {
                    command: None,
                    url: None,
                    env: HashMap::new(),
                    headers: HashMap::new(),
                    args_from_env: vec![],
                },
                confirmation_default: vec!["*".to_string()],
            }
        })
        .collect();

    Ok((servers, data.pagination.total_count))
}

async fn fetch_smithery_detail(
    http_client: &reqwest::Client,
    qualified_name: &str,
    api_key: &str,
) -> Option<MarketplaceServer> {
    let url = format!(
        "https://registry.smithery.ai/servers/{}",
        utf8_percent_encode(qualified_name, NON_ALPHANUMERIC)
    );
    let resp = http_client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: Value = resp.json().await.ok()?;

    let transport_type = data["connections"]
        .as_array()
        .and_then(|c| c.first())
        .and_then(|c| c["type"].as_str())
        .map(|t| match t {
            "http" | "streamable-http" => "http",
            "sse" => "sse",
            _ => "stdio",
        })
        .unwrap_or("stdio");

    let deployment_url = data["deploymentUrl"].as_str().map(|s| s.to_string());
    let command = if transport_type == "stdio" {
        data["connections"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["command"].as_str())
            .map(|s| s.to_string())
    } else {
        None
    };

    let publisher = qualified_name.split('/').next().unwrap_or("").to_string();
    let tags = if data["security"]["scanPassed"].as_bool().unwrap_or(false) {
        vec!["smithery".to_string(), "verified".to_string()]
    } else {
        vec!["smithery".to_string()]
    };

    Some(MarketplaceServer {
        id: qualified_name.to_string(),
        name: data["displayName"]
            .as_str()
            .unwrap_or(qualified_name)
            .to_string(),
        description: data["description"].as_str().unwrap_or("").to_string(),
        publisher,
        tags,
        icon_url: data["iconUrl"].as_str().map(|s| s.to_string()),
        homepage: data["homepage"].as_str().map(|s| s.to_string()),
        transport: transport_type.to_string(),
        install_recipe: InstallRecipe {
            command,
            url: deployment_url,
            env: HashMap::new(),
            headers: HashMap::new(),
            args_from_env: vec![],
        },
        confirmation_default: vec!["*".to_string()],
    })
}

#[derive(Deserialize)]
struct OfficialRegistryResponse {
    servers: Vec<OfficialRegistryEntry>,
    #[allow(dead_code)]
    metadata: OfficialRegistryMetadata,
}

#[derive(Deserialize)]
struct OfficialRegistryEntry {
    server: OfficialRegistryServer,
}

#[derive(Deserialize)]
struct OfficialRegistryServer {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "websiteUrl")]
    website_url: Option<String>,
    #[serde(default)]
    icons: Vec<OfficialRegistryIcon>,
    #[serde(default)]
    remotes: Vec<OfficialRegistryRemote>,
    #[serde(default)]
    packages: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct OfficialRegistryIcon {
    src: String,
}

#[derive(Deserialize)]
struct OfficialRegistryRemote {
    #[serde(rename = "type")]
    remote_type: String,
    url: String,
}

#[derive(Deserialize)]
struct OfficialRegistryMetadata {
    #[allow(dead_code)]
    #[serde(default, rename = "nextCursor")]
    next_cursor: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    count: u32,
}

async fn fetch_official_registry_servers(
    http_client: &reqwest::Client,
    query: &str,
    _page: u32,
    page_size: u32,
) -> Result<(Vec<MarketplaceServer>, u32), String> {
    let limit = page_size.min(100);
    let url = format!("{}?limit={}", OFFICIAL_MCP_REGISTRY_URL, limit);

    let resp = http_client
        .get(&url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("official-mcp request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("official-mcp: HTTP {}", resp.status()));
    }

    let body: OfficialRegistryResponse = resp
        .json()
        .await
        .map_err(|e| format!("official-mcp parse: {}", e))?;

    let servers: Vec<MarketplaceServer> = body
        .servers
        .into_iter()
        .map(|entry| {
            let s = entry.server;
            let parts: Vec<&str> = s.name.splitn(2, '/').collect();
            let publisher = parts.first().copied().unwrap_or("").to_string();
            let short_name = parts.get(1).copied().unwrap_or(s.name.as_str());
            let display_name = s.title.unwrap_or_else(|| short_name.to_string());

            let (transport, install_url) = s
                .remotes
                .first()
                .map(|r| {
                    let t = match r.remote_type.as_str() {
                        "streamable-http" => "http",
                        "sse" => "sse",
                        _ => "http",
                    };
                    (t.to_string(), Some(r.url.clone()))
                })
                .unwrap_or_else(|| {
                    if !s.packages.is_empty() {
                        ("stdio".to_string(), None)
                    } else {
                        ("stdio".to_string(), None)
                    }
                });

            let icon_url = s.icons.first().map(|i| i.src.clone());

            MarketplaceServer {
                id: s.name.clone(),
                name: display_name,
                description: s.description.unwrap_or_default(),
                publisher,
                tags: vec!["official-mcp".to_string()],
                icon_url,
                homepage: s.website_url,
                transport,
                install_recipe: InstallRecipe {
                    command: None,
                    url: install_url,
                    env: HashMap::new(),
                    headers: HashMap::new(),
                    args_from_env: vec![],
                },
                confirmation_default: vec!["**".to_string()],
            }
        })
        .collect();

    let filtered: Vec<MarketplaceServer> = if query.is_empty() {
        servers
    } else {
        let q = query.to_lowercase();
        servers
            .into_iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&q)
                    || s.description.to_lowercase().contains(&q)
                    || s.id.to_lowercase().contains(&q)
                    || s.publisher.to_lowercase().contains(&q)
            })
            .collect()
    };

    let total = filtered.len() as u32;
    Ok((filtered, total))
}

async fn load_source_servers(
    gcx: Arc<ARwLock<GlobalContext>>,
    source: &MarketplaceSource,
    query: Option<&str>,
    page: u32,
    page_size: u32,
    cache: &mut HashMap<String, (Instant, Vec<MarketplaceServerWithSource>)>,
) -> (Vec<MarketplaceServerWithSource>, u32, &'static str) {
    let ttl = match source.source_type {
        SourceType::Smithery => SMITHERY_CACHE_TTL_SECS,
        SourceType::OfficialMcp => OFFICIAL_MCP_CACHE_TTL_SECS,
        SourceType::RefactIndex => BUNDLED_CACHE_TTL_SECS,
    };

    let query_str = query.unwrap_or("");
    let cache_key = format!("{}:{}", source.id, query_str);

    if let Some((ts, cached)) = cache.get(&cache_key) {
        if ts.elapsed().as_secs() < ttl {
            let total = cached.len() as u32;
            let start = ((page - 1) * page_size) as usize;
            let end = (start + page_size as usize).min(cached.len());
            let page_items = if start < cached.len() {
                cached[start..end].to_vec()
            } else {
                vec![]
            };
            return (page_items, total, "cached");
        }
    }

    match source.source_type {
        SourceType::RefactIndex => {
            let (index, status): (MarketplaceIndex, &'static str) =
                if source.id == BUNDLED_SOURCE_ID {
                    (bundled_index(), "bundled")
                } else {
                    let http_client = gcx.read().await.http_client.clone();
                    match source.url.as_deref() {
                        Some(url) => match fetch_refact_index(&http_client, url).await {
                            Some(idx) => (idx, "remote"),
                            None => (
                                MarketplaceIndex {
                                    version: 1,
                                    updated_at: String::new(),
                                    servers: vec![],
                                },
                                "error",
                            ),
                        },
                        None => (
                            MarketplaceIndex {
                                version: 1,
                                updated_at: String::new(),
                                servers: vec![],
                            },
                            "error",
                        ),
                    }
                };

            let source_id = source.id.clone();
            let all_with_source: Vec<MarketplaceServerWithSource> = index
                .servers
                .into_iter()
                .filter(|s| {
                    if query_str.is_empty() {
                        return true;
                    }
                    let q = query_str.to_lowercase();
                    s.name.to_lowercase().contains(&q)
                        || s.description.to_lowercase().contains(&q)
                        || s.tags.iter().any(|t| t.to_lowercase().contains(&q))
                })
                .map(|s| MarketplaceServerWithSource {
                    server: s,
                    source_id: source_id.clone(),
                })
                .collect();

            let total = all_with_source.len() as u32;
            cache.insert(cache_key, (Instant::now(), all_with_source.clone()));
            let start = ((page - 1) * page_size) as usize;
            let end = (start + page_size as usize).min(all_with_source.len());
            let page_items = if start < all_with_source.len() {
                all_with_source[start..end].to_vec()
            } else {
                vec![]
            };
            (page_items, total, status)
        }
        SourceType::Smithery => {
            let config_dir = gcx.read().await.config_dir.clone();
            let sources_cfg = load_sources(&config_dir).await;
            let api_key = match smithery_api_key(&sources_cfg.sources) {
                Some(k) => k,
                None => return (vec![], 0, "no_api_key"),
            };

            let http_client = gcx.read().await.http_client.clone();
            match fetch_smithery_servers(&http_client, &api_key, query, page, page_size).await {
                Ok((servers, total)) => {
                    let source_id = source.id.clone();
                    let with_source: Vec<MarketplaceServerWithSource> = servers
                        .into_iter()
                        .map(|s| MarketplaceServerWithSource {
                            server: s,
                            source_id: source_id.clone(),
                        })
                        .collect();
                    (with_source, total, "ok")
                }
                Err(_) => (vec![], 0, "error"),
            }
        }
        SourceType::OfficialMcp => {
            let http_client = gcx.read().await.http_client.clone();
            let query_str = query.unwrap_or("");
            match fetch_official_registry_servers(&http_client, query_str, page, page_size).await {
                Ok((servers, _)) => {
                    let source_id = source.id.clone();
                    let all_with_source: Vec<MarketplaceServerWithSource> = servers
                        .into_iter()
                        .map(|s| MarketplaceServerWithSource {
                            server: s,
                            source_id: source_id.clone(),
                        })
                        .collect();
                    let total = all_with_source.len() as u32;
                    cache.insert(cache_key, (Instant::now(), all_with_source.clone()));
                    let start = ((page - 1) * page_size) as usize;
                    let end = (start + page_size as usize).min(all_with_source.len());
                    let page_items = if start < all_with_source.len() {
                        all_with_source[start..end].to_vec()
                    } else {
                        vec![]
                    };
                    (page_items, total, "ok")
                }
                Err(_) => (vec![], 0, "error"),
            }
        }
    }
}

fn validate_env_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

#[derive(Deserialize)]
pub struct MarketplaceQuery {
    pub source: Option<String>,
    pub q: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

const MERGED_MODE_PAGE_SIZE_CAP: u32 = 500;

pub async fn handle_v1_mcp_marketplace_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(params): Query<MarketplaceQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).min(100).max(1);
    let query = params.q.as_deref();

    let config_dir = gcx.read().await.config_dir.clone();
    let (bundled, user_sources) = get_all_sources(&config_dir).await;

    let bundled_removable = false;
    let mut all_sources: Vec<(MarketplaceSource, bool)> = vec![(bundled, bundled_removable)];
    for s in user_sources {
        all_sources.push((s, true));
    }

    let filter_source = params.source.as_deref();
    if let Some(fsrc) = filter_source {
        if !all_sources.iter().any(|(s, _)| s.id == fsrc) {
            return Err((
                StatusCode::NOT_FOUND,
                format!("source '{}' not found", fsrc),
            ));
        }
    }

    let mut cache = get_cache();
    let mut all_servers: Vec<MarketplaceServerWithSource> = vec![];
    let mut sources_meta: Vec<Value> = vec![];

    for (source, removable) in &all_sources {
        if !source.enabled {
            let mut meta = source_to_api_json(source, *removable);
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("server_count".to_string(), json!(0));
                obj.insert("status".to_string(), json!("disabled"));
            }
            sources_meta.push(meta);
            continue;
        }
        if let Some(fsrc) = filter_source {
            if source.id != fsrc {
                let mut meta = source_to_api_json(source, *removable);
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("server_count".to_string(), json!(0));
                    obj.insert("status".to_string(), json!("ok"));
                }
                sources_meta.push(meta);
                continue;
            }
        }

        let is_merged_mode = filter_source.is_none();
        if is_merged_mode && source.source_type == SourceType::Smithery {
            let mut meta = source_to_api_json(source, *removable);
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("server_count".to_string(), json!(0));
                obj.insert("status".to_string(), json!("ok"));
            }
            sources_meta.push(meta);
            continue;
        }
        // OfficialMcp IS included in merged mode (free, no API key)

        let fetch_page_size = if is_merged_mode {
            MERGED_MODE_PAGE_SIZE_CAP
        } else {
            page_size
        };
        let fetch_page = if is_merged_mode { 1 } else { page };

        let (page_items, total, status) = load_source_servers(
            gcx.clone(),
            source,
            query,
            fetch_page,
            fetch_page_size,
            &mut cache,
        )
        .await;

        let mut meta = source_to_api_json(source, *removable);
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("server_count".to_string(), json!(total));
            obj.insert("status".to_string(), json!(status));
        }
        sources_meta.push(meta);

        all_servers.extend(page_items);
    }

    set_cache(cache);

    let (final_servers, final_total) = if filter_source.is_some() {
        let t = sources_meta
            .iter()
            .find(|m| m["id"].as_str() == filter_source)
            .and_then(|m| m["server_count"].as_u64())
            .unwrap_or(0) as u32;
        (all_servers, t)
    } else {
        let mut seen_ids: HashSet<String> = HashSet::new();
        let deduped: Vec<MarketplaceServerWithSource> = all_servers
            .into_iter()
            .filter(|s| seen_ids.insert(s.server.id.clone()))
            .collect();
        let total_count = deduped.len() as u32;
        let start = ((page - 1) * page_size) as usize;
        let end = (start + page_size as usize).min(deduped.len());
        let sliced = if start < deduped.len() {
            deduped[start..end].to_vec()
        } else {
            vec![]
        };
        (sliced, total_count)
    };

    Ok(Json(json!({
        "servers": final_servers,
        "sources": sources_meta,
        "pagination": {
            "page": page,
            "page_size": page_size,
            "total": final_total,
        },
    })))
}

#[derive(Deserialize)]
pub struct InstallRequest {
    pub server_id: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub config_overrides: Option<ConfigOverrides>,
}

#[derive(Deserialize, Default)]
pub struct ConfigOverrides {
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

async fn find_server_in_sources(
    gcx: Arc<ARwLock<GlobalContext>>,
    server_id: &str,
    source_id: Option<&str>,
) -> Option<(MarketplaceServer, String)> {
    let config_dir = gcx.read().await.config_dir.clone();
    let (bundled, user_sources) = get_all_sources(&config_dir).await;

    let mut all_sources: Vec<MarketplaceSource> = vec![bundled];
    all_sources.extend(user_sources);

    let sources_to_search: Vec<&MarketplaceSource> = if let Some(sid) = source_id {
        all_sources.iter().filter(|s| s.id == sid).collect()
    } else {
        all_sources.iter().collect()
    };

    for source in sources_to_search {
        if source.source_type == SourceType::RefactIndex {
            let index = if source.id == BUNDLED_SOURCE_ID {
                bundled_index()
            } else {
                let http_client = gcx.read().await.http_client.clone();
                match source.url.as_deref() {
                    Some(url) => match fetch_refact_index(&http_client, url).await {
                        Some(idx) => idx,
                        None => continue,
                    },
                    None => continue,
                }
            };
            if let Some(server) = index.servers.into_iter().find(|s| s.id == server_id) {
                return Some((server, source.id.clone()));
            }
        } else if source.source_type == SourceType::Smithery {
            let cfg = load_sources(&config_dir).await;
            let api_key = match smithery_api_key(&cfg.sources) {
                Some(k) => k,
                None => continue,
            };
            let http_client = gcx.read().await.http_client.clone();
            if let Some(server) = fetch_smithery_detail(&http_client, server_id, &api_key).await {
                return Some((server, source.id.clone()));
            }
        } else if source.source_type == SourceType::OfficialMcp {
            let http_client = gcx.read().await.http_client.clone();
            if let Ok((servers, _)) =
                fetch_official_registry_servers(&http_client, "", 1, 100).await
            {
                if let Some(server) = servers.into_iter().find(|s| s.id == server_id) {
                    return Some((server, source.id.clone()));
                }
            }
        }
    }
    None
}

pub async fn handle_v1_mcp_marketplace_install(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<InstallRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    if mcp_naming::validate_server_id(&req.server_id).is_err() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "invalid server_id".to_string(),
        ));
    }

    let (server, found_source_id) =
        find_server_in_sources(gcx.clone(), &req.server_id, req.source_id.as_deref())
            .await
            .ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("server '{}' not found in marketplace", req.server_id),
                )
            })?;

    match server.transport.as_str() {
        "http" | "streamable-http" | "sse" => {
            if server.install_recipe.url.is_none() {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "server '{}' has transport '{}' but no url in recipe",
                        server.id, server.transport
                    ),
                ));
            }
        }
        _ => {
            if server.install_recipe.command.is_none() {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "server '{}' has transport 'stdio' but no command in recipe",
                        server.id
                    ),
                ));
            }
        }
    }

    let config_dir = gcx.read().await.config_dir.clone();
    let integrations_dir = config_dir.join("integrations.d");
    tokio::fs::create_dir_all(&integrations_dir)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cannot create integrations dir: {}", e),
            )
        })?;

    let prefix = match server.transport.as_str() {
        "http" | "streamable-http" => "mcp_http",
        "sse" => "mcp_sse",
        _ => "mcp_stdio",
    };
    let safe_id = server.id.replace(['/', '-', '.'], "_");
    let filename = format!("{}_{}.yaml", prefix, safe_id);
    let config_path = integrations_dir.join(&filename);

    let mut env = server.install_recipe.env.clone();
    let mut headers = server.install_recipe.headers.clone();
    if let Some(overrides) = &req.config_overrides {
        for (k, v) in &overrides.env {
            if !validate_env_key(k) {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!("invalid env key: {:?}", k),
                ));
            }
            env.insert(k.clone(), v.clone());
        }
        for (k, v) in &overrides.headers {
            if !validate_env_key(k) {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!("invalid header key: {:?}", k),
                ));
            }
            headers.insert(k.clone(), v.clone());
        }
    }
    for k in env.keys() {
        if !validate_env_key(k) {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!("invalid env key in recipe: {:?}", k),
            ));
        }
    }
    for k in headers.keys() {
        if !validate_env_key(k) {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!("invalid header key in recipe: {:?}", k),
            ));
        }
    }

    let yaml_content = build_integration_yaml(&server, &env, &headers, &found_source_id);
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config_path)
        .await
    {
        Ok(mut file) => {
            use tokio::io::AsyncWriteExt;
            file.write_all(yaml_content.as_bytes()).await.map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write error: {}", e),
                )
            })?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                format!("config file '{}' already exists", filename),
            ));
        }
        Err(e) => {
            return Err(ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create error: {}", e),
            ));
        }
    }

    Ok(Json(json!({
        "installed": true,
        "config_path": config_path.display().to_string(),
    })))
}

fn build_integration_yaml(
    server: &MarketplaceServer,
    env: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    source_id: &str,
) -> String {
    let mut map = serde_yaml::Mapping::new();

    match server.transport.as_str() {
        "http" | "streamable-http" => {
            if let Some(ref url) = server.install_recipe.url {
                map.insert(
                    serde_yaml::Value::String("url".to_string()),
                    serde_yaml::Value::String(url.clone()),
                );
            }
            let headers_map: serde_yaml::Mapping = headers
                .iter()
                .map(|(k, v)| {
                    (
                        serde_yaml::Value::String(k.clone()),
                        serde_yaml::Value::String(v.clone()),
                    )
                })
                .collect();
            map.insert(
                serde_yaml::Value::String("headers".to_string()),
                serde_yaml::Value::Mapping(headers_map),
            );
            map.insert(
                serde_yaml::Value::String("auth_type".to_string()),
                serde_yaml::Value::String("none".to_string()),
            );
        }
        "sse" => {
            if let Some(ref url) = server.install_recipe.url {
                map.insert(
                    serde_yaml::Value::String("url".to_string()),
                    serde_yaml::Value::String(url.clone()),
                );
            }
            let headers_map: serde_yaml::Mapping = headers
                .iter()
                .map(|(k, v)| {
                    (
                        serde_yaml::Value::String(k.clone()),
                        serde_yaml::Value::String(v.clone()),
                    )
                })
                .collect();
            map.insert(
                serde_yaml::Value::String("headers".to_string()),
                serde_yaml::Value::Mapping(headers_map),
            );
            map.insert(
                serde_yaml::Value::String("auth_type".to_string()),
                serde_yaml::Value::String("none".to_string()),
            );
        }
        _ => {
            if let Some(ref cmd) = server.install_recipe.command {
                let full_cmd = if server.install_recipe.args_from_env.is_empty() {
                    cmd.clone()
                } else {
                    let extra: Vec<&str> = server
                        .install_recipe
                        .args_from_env
                        .iter()
                        .filter_map(|k| env.get(k).map(|v| v.as_str()))
                        .collect();
                    if extra.is_empty() {
                        cmd.clone()
                    } else {
                        format!("{} {}", cmd, extra.join(" "))
                    }
                };
                map.insert(
                    serde_yaml::Value::String("command".to_string()),
                    serde_yaml::Value::String(full_cmd),
                );
            }
            let env_map: serde_yaml::Mapping = env
                .iter()
                .map(|(k, v)| {
                    (
                        serde_yaml::Value::String(k.clone()),
                        serde_yaml::Value::String(v.clone()),
                    )
                })
                .collect();
            map.insert(
                serde_yaml::Value::String("env".to_string()),
                serde_yaml::Value::Mapping(env_map),
            );
        }
    }

    map.insert(
        serde_yaml::Value::String("init_timeout".to_string()),
        serde_yaml::Value::String("60".to_string()),
    );
    map.insert(
        serde_yaml::Value::String("request_timeout".to_string()),
        serde_yaml::Value::String("30".to_string()),
    );

    let mut available = serde_yaml::Mapping::new();
    available.insert(
        serde_yaml::Value::String("on_your_laptop".to_string()),
        serde_yaml::Value::Bool(true),
    );
    available.insert(
        serde_yaml::Value::String("when_isolated".to_string()),
        serde_yaml::Value::Bool(false),
    );
    map.insert(
        serde_yaml::Value::String("available".to_string()),
        serde_yaml::Value::Mapping(available),
    );

    let confirmation_list: serde_yaml::Value = serde_yaml::Value::Sequence(
        server
            .confirmation_default
            .iter()
            .map(|s| serde_yaml::Value::String(s.clone()))
            .collect(),
    );
    let mut confirmation = serde_yaml::Mapping::new();
    confirmation.insert(
        serde_yaml::Value::String("ask_user_default".to_string()),
        confirmation_list,
    );
    map.insert(
        serde_yaml::Value::String("confirmation".to_string()),
        serde_yaml::Value::Mapping(confirmation),
    );

    let yaml_body =
        serde_yaml::to_string(&serde_yaml::Value::Mapping(map)).unwrap_or_else(|_| String::new());

    format!(
        "# mcp_marketplace_source: {}\n# mcp_marketplace_server: {}\n{}",
        source_id, server.id, yaml_body
    )
}

fn parse_marketplace_comments(content: &str) -> (Option<String>, Option<String>) {
    let mut source_id = None;
    let mut server_id = None;
    for line in content.lines().take(10) {
        if let Some(val) = line.strip_prefix("# mcp_marketplace_source:") {
            source_id = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("# mcp_marketplace_server:") {
            server_id = Some(val.trim().to_string());
        }
        if !line.starts_with('#') && !line.is_empty() {
            break;
        }
    }
    (source_id, server_id)
}

pub async fn handle_v1_mcp_marketplace_installed(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let config_dir = gcx.read().await.config_dir.clone();
    let integrations_dir = config_dir.join("integrations.d");

    let bundled = bundled_index();
    let index_ids: std::collections::HashSet<String> =
        bundled.servers.iter().map(|s| s.id.clone()).collect();
    let mut installed = Vec::new();

    let read_dir = match tokio::fs::read_dir(&integrations_dir).await {
        Ok(rd) => rd,
        Err(_) => {
            return Ok(Json(json!({ "installed": installed })));
        }
    };

    let mut rd = read_dir;
    while let Ok(Some(entry)) = rd.next_entry().await {
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        if !fname_str.ends_with(".yaml") {
            continue;
        }
        let is_mcp = ["mcp_stdio_", "mcp_sse_", "mcp_http_"]
            .iter()
            .any(|p| fname_str.starts_with(p));
        if !is_mcp {
            continue;
        }

        let content = match tokio::fs::read_to_string(entry.path()).await {
            Ok(c) => c,
            Err(_) => continue,
        };

        let (found_source, found_server) = parse_marketplace_comments(&content);
        if let (Some(src_id), Some(srv_id)) = (found_source, found_server) {
            installed.push(json!({
                "id": srv_id,
                "config_path": entry.path().display().to_string(),
                "source_id": src_id,
            }));
            continue;
        }

        for prefix in &["mcp_stdio_", "mcp_sse_", "mcp_http_"] {
            if let Some(rest) = fname_str.strip_prefix(prefix) {
                let id_candidate = rest.trim_end_matches(".yaml").replace('_', "-");
                if index_ids.contains(&id_candidate) {
                    installed.push(json!({
                        "id": id_candidate,
                        "config_path": entry.path().display().to_string(),
                        "source_id": BUNDLED_SOURCE_ID,
                    }));
                }
                break;
            }
        }
    }

    Ok(Json(json!({ "installed": installed })))
}

#[derive(Deserialize)]
pub struct AutoNameRequest {
    pub input: String,
}

pub fn detect_transport(input: &str) -> &'static str {
    let trimmed = input.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        "http"
    } else {
        "stdio"
    }
}

pub fn extract_name_from_input(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("input is empty".to_string());
    }

    let raw = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        extract_name_from_url(trimmed)
    } else {
        extract_name_from_command(trimmed)
    };

    let sanitized = sanitize_name(&raw);
    if sanitized.is_empty() {
        return Err("could not extract a valid name from input".to_string());
    }
    Ok(sanitized)
}

fn extract_name_from_url(url: &str) -> String {
    let without_scheme = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host_and_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host = if host_and_port.starts_with('[') {
        host_and_port
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or("mcp")
    } else {
        host_and_port.split(':').next().unwrap_or(host_and_port)
    };

    if host == "localhost" {
        return "localhost".to_string();
    }

    let is_ip = host
        .split('.')
        .all(|seg| seg.chars().all(|c| c.is_ascii_digit()));
    if is_ip || host.starts_with('[') {
        return host.replace('[', "").replace(']', "").replace(':', "_");
    }

    let parts: Vec<&str> = host.split('.').collect();
    // Country-code SLD pattern: e.g. example.co.uk, example.com.au
    // Only trigger when last segment is 2-char country code AND second-to-last is a short
    // known SLD (co, com, org, net, ac, gov, edu) — not for domains like mcp.myservice.io
    if parts.len() >= 3 {
        let last = parts[parts.len() - 1];
        let second_last = parts[parts.len() - 2];
        let is_country_code_sld = last.len() == 2
            && matches!(
                second_last,
                "co" | "com" | "org" | "net" | "ac" | "gov" | "edu" | "or" | "ne"
            );
        if is_country_code_sld {
            return parts[parts.len() - 3].to_string();
        }
    }
    if parts.len() >= 2 {
        parts[parts.len() - 2].to_string()
    } else {
        parts.first().copied().unwrap_or("mcp").to_string()
    }
}

fn extract_name_from_command(cmd: &str) -> String {
    let args: Vec<&str> = cmd.split_whitespace().collect();
    let mut candidate = "";
    for (i, arg) in args.iter().enumerate() {
        if *arg == "run" || *arg == "-y" || *arg == "-i" || *arg == "--rm" || *arg == "-it" {
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        if i > 0
            && (args[i - 1] == "-e"
                || args[i - 1] == "--env"
                || args[i - 1] == "-p"
                || args[i - 1] == "--port")
        {
            continue;
        }
        candidate = arg;
        if *arg != "npx"
            && *arg != "uvx"
            && *arg != "docker"
            && *arg != "node"
            && *arg != "python"
            && *arg != "python3"
        {
            break;
        }
    }
    let name = candidate.rsplit('/').next().unwrap_or(candidate);
    let name = name.trim_end_matches(".js");
    let name = name.trim_start_matches('@');
    let name = if let Some(slash_pos) = name.find('/') {
        &name[slash_pos + 1..]
    } else {
        name
    };
    strip_mcp_prefixes(name)
}

fn strip_mcp_prefixes(s: &str) -> String {
    let stripped = s
        .trim_start_matches("mcp-server-")
        .trim_start_matches("server-mcp-")
        .trim_start_matches("mcp-")
        .trim_start_matches("server-");
    stripped.to_string()
}

fn sanitize_name(s: &str) -> String {
    let snake: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let snake = snake.trim_matches('_').to_string();
    let snake: String = {
        let mut prev_underscore = false;
        snake
            .chars()
            .filter(|c| {
                if *c == '_' {
                    if prev_underscore {
                        return false;
                    }
                    prev_underscore = true;
                } else {
                    prev_underscore = false;
                }
                true
            })
            .collect()
    };
    if snake.len() > 40 {
        snake[..40].to_string()
    } else {
        snake
    }
}

pub async fn handle_v1_mcp_auto_name(
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<AutoNameRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    let suggested_name = extract_name_from_input(&req.input)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let transport = detect_transport(&req.input);
    let config_prefix = match transport {
        "http" => "mcp_http_",
        _ => "mcp_stdio_",
    };

    Ok(Json(json!({
        "suggested_name": suggested_name,
        "transport": transport,
        "config_prefix": config_prefix,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundled_index_parses() {
        let index = bundled_index();
        assert!(index.version >= 1, "version must be >= 1");
        assert!(
            index.servers.len() >= 30,
            "must have at least 30 servers, got {}",
            index.servers.len()
        );
    }

    #[test]
    fn test_bundled_index_all_servers_have_required_fields() {
        let index = bundled_index();
        for server in &index.servers {
            assert!(!server.id.is_empty(), "server id must not be empty");
            assert!(
                !server.name.is_empty(),
                "server name must not be empty for id={}",
                server.id
            );
            assert!(
                !server.description.is_empty(),
                "server description must not be empty for id={}",
                server.id
            );
            assert!(
                !server.transport.is_empty(),
                "server transport must not be empty for id={}",
                server.id
            );
        }
    }

    #[test]
    fn test_bundled_index_no_duplicate_ids() {
        let index = bundled_index();
        let mut ids = std::collections::HashSet::new();
        for server in &index.servers {
            assert!(
                ids.insert(server.id.clone()),
                "duplicate server id: {}",
                server.id
            );
        }
    }

    #[test]
    fn test_bundled_index_expanded() {
        let index = bundled_index();
        assert!(
            index.servers.len() >= 30,
            "bundled index must have at least 30 servers"
        );
    }

    #[test]
    fn test_validate_server_id() {
        assert!(
            mcp_naming::validate_server_id("github").is_ok(),
            "valid name"
        );
        assert!(
            mcp_naming::validate_server_id("my-server").is_ok(),
            "valid name with dash"
        );
        assert!(
            mcp_naming::validate_server_id("").is_err(),
            "empty name invalid"
        );
        assert!(
            mcp_naming::validate_server_id("../evil").is_err(),
            "path traversal invalid"
        );
        assert!(
            mcp_naming::validate_server_id("a/b").is_ok(),
            "slash valid for smithery IDs"
        );
        assert!(
            mcp_naming::validate_server_id("a\\b").is_err(),
            "backslash invalid"
        );
    }

    #[test]
    fn test_build_integration_yaml_stdio_with_env() {
        let server = MarketplaceServer {
            id: "github".to_string(),
            name: "GitHub".to_string(),
            description: "GitHub MCP server".to_string(),
            publisher: "github".to_string(),
            tags: vec!["vcs".to_string()],
            icon_url: None,
            homepage: None,
            transport: "stdio".to_string(),
            install_recipe: InstallRecipe {
                command: Some("npx -y @modelcontextprotocol/server-github".to_string()),
                url: None,
                env: HashMap::new(),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec!["*".to_string()],
        };
        let mut env = HashMap::new();
        env.insert(
            "GITHUB_PERSONAL_ACCESS_TOKEN".to_string(),
            "ghp_test".to_string(),
        );
        let yaml = build_integration_yaml(&server, &env, &HashMap::new(), "refact-bundled");
        assert!(
            yaml.contains("npx -y @modelcontextprotocol/server-github"),
            "yaml must contain command"
        );
        assert!(
            yaml.contains("GITHUB_PERSONAL_ACCESS_TOKEN"),
            "yaml must contain env key"
        );
        assert!(yaml.contains("ghp_test"), "yaml must contain env value");
        assert!(
            yaml.contains("init_timeout"),
            "yaml must contain init_timeout"
        );
        assert!(
            yaml.contains("request_timeout"),
            "yaml must contain request_timeout"
        );
        assert!(
            yaml.contains("ask_user_default"),
            "yaml must contain confirmation"
        );
        assert!(
            yaml.contains("# mcp_marketplace_source: refact-bundled"),
            "yaml must have source comment"
        );
        assert!(
            yaml.contains("# mcp_marketplace_server: github"),
            "yaml must have server comment"
        );
    }

    #[test]
    fn test_build_integration_yaml_empty_env() {
        let server = MarketplaceServer {
            id: "fetch".to_string(),
            name: "Fetch".to_string(),
            description: "Fetch server".to_string(),
            publisher: "anthropic".to_string(),
            tags: vec![],
            icon_url: None,
            homepage: None,
            transport: "stdio".to_string(),
            install_recipe: InstallRecipe {
                command: Some("uvx mcp-server-fetch".to_string()),
                url: None,
                env: HashMap::new(),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec!["*".to_string()],
        };
        let yaml =
            build_integration_yaml(&server, &HashMap::new(), &HashMap::new(), "refact-bundled");
        assert!(yaml.contains("env:"), "yaml must contain env section");
    }

    #[test]
    fn test_build_integration_yaml_http_with_url() {
        let server = MarketplaceServer {
            id: "test-http".to_string(),
            name: "Test HTTP".to_string(),
            description: "HTTP MCP server".to_string(),
            publisher: "test".to_string(),
            tags: vec![],
            icon_url: None,
            homepage: None,
            transport: "http".to_string(),
            install_recipe: InstallRecipe {
                command: None,
                url: Some("http://localhost:3000/mcp".to_string()),
                env: HashMap::new(),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec![],
        };
        let yaml =
            build_integration_yaml(&server, &HashMap::new(), &HashMap::new(), "refact-bundled");
        assert!(yaml.contains("url:"), "yaml must contain url");
        assert!(yaml.contains("auth_type:"), "yaml must contain auth_type");
        assert!(yaml.contains("headers:"), "yaml must contain headers");
        assert!(
            !yaml.contains("command:"),
            "yaml must not contain command for http"
        );
        assert!(!yaml.contains("env:"), "yaml must not contain env for http");
    }

    #[test]
    fn test_build_integration_yaml_sse_with_headers() {
        let server = MarketplaceServer {
            id: "test-sse".to_string(),
            name: "Test SSE".to_string(),
            description: "SSE MCP server".to_string(),
            publisher: "test".to_string(),
            tags: vec![],
            icon_url: None,
            homepage: None,
            transport: "sse".to_string(),
            install_recipe: InstallRecipe {
                command: None,
                url: Some("https://api.example.com/sse".to_string()),
                env: HashMap::new(),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec![],
        };
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token123".to_string());
        let yaml = build_integration_yaml(&server, &HashMap::new(), &headers, "refact-bundled");
        assert!(yaml.contains("url:"), "yaml must contain url");
        assert!(
            yaml.contains("Authorization"),
            "yaml must contain Authorization header"
        );
        assert!(yaml.contains("token123"), "yaml must contain token value");
        assert!(yaml.contains("auth_type:"), "yaml must contain auth_type");
    }

    #[test]
    fn test_install_response_contract() {
        let response = json!({ "installed": true, "config_path": "/some/path.yaml" });
        assert_eq!(response["installed"], true);
        assert!(response["config_path"].as_str().is_some());
        assert!(response.get("success").is_none());
    }

    #[test]
    fn test_smithery_response_mapping() {
        let server = MarketplaceServer {
            id: "owner/hello-world".to_string(),
            name: "Hello World".to_string(),
            description: "A test server".to_string(),
            publisher: "owner".to_string(),
            tags: vec!["smithery".to_string()],
            icon_url: None,
            homepage: None,
            transport: "stdio".to_string(),
            install_recipe: InstallRecipe {
                command: None,
                url: None,
                env: HashMap::new(),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec!["*".to_string()],
        };
        assert_eq!(server.id, "owner/hello-world");
        assert_eq!(server.publisher, "owner");
        assert!(server.tags.contains(&"smithery".to_string()));
    }

    #[test]
    fn test_multi_source_merge() {
        let bundled_server = MarketplaceServerWithSource {
            server: MarketplaceServer {
                id: "github".to_string(),
                name: "GitHub".to_string(),
                description: "desc".to_string(),
                publisher: "github".to_string(),
                tags: vec![],
                icon_url: None,
                homepage: None,
                transport: "stdio".to_string(),
                install_recipe: InstallRecipe {
                    command: Some("cmd".to_string()),
                    url: None,
                    env: HashMap::new(),
                    headers: HashMap::new(),
                    args_from_env: vec![],
                },
                confirmation_default: vec![],
            },
            source_id: BUNDLED_SOURCE_ID.to_string(),
        };
        let smithery_server = MarketplaceServerWithSource {
            server: MarketplaceServer {
                id: "smithery/hello".to_string(),
                name: "Hello".to_string(),
                description: "desc".to_string(),
                publisher: "smithery".to_string(),
                tags: vec!["smithery".to_string()],
                icon_url: None,
                homepage: None,
                transport: "http".to_string(),
                install_recipe: InstallRecipe {
                    command: None,
                    url: Some("https://ex.com".to_string()),
                    env: HashMap::new(),
                    headers: HashMap::new(),
                    args_from_env: vec![],
                },
                confirmation_default: vec![],
            },
            source_id: SMITHERY_SOURCE_ID.to_string(),
        };
        let all = vec![bundled_server, smithery_server];
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].source_id, BUNDLED_SOURCE_ID);
        assert_eq!(all[1].source_id, SMITHERY_SOURCE_ID);
    }

    #[test]
    fn test_source_id_tracking() {
        let server = MarketplaceServerWithSource {
            server: MarketplaceServer {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: "desc".to_string(),
                publisher: "test".to_string(),
                tags: vec![],
                icon_url: None,
                homepage: None,
                transport: "stdio".to_string(),
                install_recipe: InstallRecipe {
                    command: Some("cmd".to_string()),
                    url: None,
                    env: HashMap::new(),
                    headers: HashMap::new(),
                    args_from_env: vec![],
                },
                confirmation_default: vec![],
            },
            source_id: "my-source".to_string(),
        };
        let json = serde_json::to_value(&server).unwrap();
        assert_eq!(json["source_id"], "my-source");
        assert_eq!(json["id"], "test");
    }

    #[test]
    fn test_source_cache_independence() {
        let mut cache: HashMap<String, (Instant, Vec<MarketplaceServerWithSource>)> =
            HashMap::new();
        cache.insert("source-a:".to_string(), (Instant::now(), vec![]));
        cache.insert("source-b:".to_string(), (Instant::now(), vec![]));
        assert!(cache.contains_key("source-a:"));
        assert!(cache.contains_key("source-b:"));
        cache.remove("source-a:");
        assert!(
            !cache.contains_key("source-a:"),
            "removing source-a doesn't affect source-b"
        );
        assert!(cache.contains_key("source-b:"));
    }

    #[test]
    fn test_install_with_source_id() {
        let server = MarketplaceServer {
            id: "github".to_string(),
            name: "GitHub".to_string(),
            description: "desc".to_string(),
            publisher: "github".to_string(),
            tags: vec![],
            icon_url: None,
            homepage: None,
            transport: "stdio".to_string(),
            install_recipe: InstallRecipe {
                command: Some("npx github".to_string()),
                url: None,
                env: HashMap::new(),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec![],
        };
        let yaml =
            build_integration_yaml(&server, &HashMap::new(), &HashMap::new(), "refact-bundled");
        assert!(yaml.contains("command:"));
    }

    #[tokio::test]
    async fn test_install_creates_correct_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let integrations_dir = tmp.path().join("integrations.d");
        tokio::fs::create_dir_all(&integrations_dir).await.unwrap();

        let server = MarketplaceServer {
            id: "brave-search".to_string(),
            name: "Brave Search".to_string(),
            description: "Web search".to_string(),
            publisher: "anthropic".to_string(),
            tags: vec!["search".to_string()],
            icon_url: None,
            homepage: None,
            transport: "stdio".to_string(),
            install_recipe: InstallRecipe {
                command: Some("npx -y @modelcontextprotocol/server-brave-search".to_string()),
                url: None,
                env: {
                    let mut m = HashMap::new();
                    m.insert("BRAVE_API_KEY".to_string(), "".to_string());
                    m
                },
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec!["*".to_string()],
        };

        let mut env = server.install_recipe.env.clone();
        env.insert("BRAVE_API_KEY".to_string(), "test-key-123".to_string());

        let yaml = build_integration_yaml(&server, &env, &HashMap::new(), "refact-bundled");
        let config_path = integrations_dir.join("mcp_stdio_brave_search.yaml");
        tokio::fs::write(&config_path, &yaml).await.unwrap();

        let content = tokio::fs::read_to_string(&config_path).await.unwrap();
        assert!(content.contains("npx -y @modelcontextprotocol/server-brave-search"));
        assert!(content.contains("BRAVE_API_KEY"));
        assert!(content.contains("test-key-123"));
        assert!(content.contains("init_timeout"));
        assert!(content.contains("request_timeout"));
        assert!(content.contains("ask_user_default"));
    }

    #[tokio::test]
    async fn test_install_no_clobber_race_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let integrations_dir = tmp.path().join("integrations.d");
        tokio::fs::create_dir_all(&integrations_dir).await.unwrap();
        let path = integrations_dir.join("mcp_stdio_github.yaml");
        tokio::fs::write(&path, "existing: true\n").await.unwrap();

        let result = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn test_auto_name_from_npx_command() {
        let name = extract_name_from_input("npx -y @notionhq/notion-mcp-server").unwrap();
        assert_eq!(name, "notion_mcp_server");
    }

    #[test]
    fn test_auto_name_from_uvx_command() {
        let name = extract_name_from_input("uvx mcp-server-fetch").unwrap();
        assert_eq!(name, "fetch");
    }

    #[test]
    fn test_auto_name_from_url() {
        let name = extract_name_from_input("https://api.example.com/mcp").unwrap();
        assert_eq!(name, "example");
    }

    #[test]
    fn test_extract_name_from_url_country_code_tld() {
        assert_eq!(
            extract_name_from_url("https://api.example.co.uk/mcp"),
            "example"
        );
    }

    #[test]
    fn test_extract_name_from_url_localhost() {
        assert_eq!(extract_name_from_url("http://localhost:3000"), "localhost");
        assert_eq!(
            extract_name_from_url("http://localhost:3000/path"),
            "localhost"
        );
    }

    #[test]
    fn test_extract_name_from_url_ip_address() {
        let name = extract_name_from_url("http://192.168.1.1:8080/mcp");
        assert_eq!(name, "192.168.1.1");
    }

    #[test]
    fn test_extract_name_from_url_ipv6() {
        let name = extract_name_from_url("http://[::1]:3000/mcp");
        assert!(
            !name.contains('['),
            "brackets must be stripped from ipv6 result"
        );
        assert!(
            !name.contains(']'),
            "brackets must be stripped from ipv6 result"
        );
    }

    #[test]
    fn test_extract_name_from_url_simple_domain() {
        assert_eq!(
            extract_name_from_url("https://api.example.com/path"),
            "example"
        );
        assert_eq!(
            extract_name_from_url("https://mcp.myservice.io/v1"),
            "myservice"
        );
    }

    #[test]
    fn test_auto_name_from_docker_command() {
        let name = extract_name_from_input("docker run -i --rm mcp/server-github").unwrap();
        assert_eq!(name, "github");
    }

    #[test]
    fn test_auto_name_sanitization() {
        let name = extract_name_from_input("npx -y @my-org/my-cool-tool!").unwrap();
        assert!(name.chars().all(|c| c.is_alphanumeric() || c == '_'));
        assert!(!name.starts_with('_'));
        assert!(!name.ends_with('_'));
    }

    #[test]
    fn test_transport_detection_url() {
        assert_eq!(detect_transport("https://api.example.com/mcp"), "http");
        assert_eq!(detect_transport("http://localhost:3000/mcp"), "http");
    }

    #[test]
    fn test_transport_detection_command() {
        assert_eq!(
            detect_transport("npx -y @notionhq/notion-mcp-server"),
            "stdio"
        );
        assert_eq!(detect_transport("uvx mcp-server-fetch"), "stdio");
        assert_eq!(detect_transport("docker run -i --rm mcp/github"), "stdio");
    }

    #[test]
    fn test_auto_name_empty_input() {
        let result = extract_name_from_input("");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_installed_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let integrations_dir = tmp.path().join("integrations.d");
        tokio::fs::create_dir_all(&integrations_dir).await.unwrap();
        tokio::fs::write(
            integrations_dir.join("mcp_stdio_github.yaml"),
            "command: npx github\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            integrations_dir.join("mcp_stdio_brave_search.yaml"),
            "command: npx brave\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            integrations_dir.join("other_integration.yaml"),
            "some: config\n",
        )
        .await
        .unwrap();

        let index = bundled_index();
        let index_ids: std::collections::HashSet<String> =
            index.servers.iter().map(|s| s.id.clone()).collect();

        let mut installed_ids = Vec::new();
        let mut rd = tokio::fs::read_dir(&integrations_dir).await.unwrap();
        while let Ok(Some(entry)) = rd.next_entry().await {
            let fname = entry.file_name();
            let fname_str = fname.to_string_lossy().to_string();
            if !fname_str.ends_with(".yaml") {
                continue;
            }
            for prefix in &["mcp_stdio_", "mcp_sse_", "mcp_http_"] {
                if let Some(rest) = fname_str.strip_prefix(prefix) {
                    let id_candidate = rest.trim_end_matches(".yaml").replace('_', "-");
                    if index_ids.contains(&id_candidate) {
                        installed_ids.push(id_candidate);
                    }
                    break;
                }
            }
        }
        assert!(
            installed_ids.contains(&"github".to_string()),
            "must detect github as installed"
        );
        assert!(
            installed_ids.contains(&"brave-search".to_string()),
            "must detect brave-search as installed"
        );
        assert!(
            !installed_ids.contains(&"other".to_string()),
            "must not detect non-mcp integrations"
        );
    }

    #[test]
    fn test_sources_response_has_required_fields() {
        use crate::http::routers::v1::mcp_marketplace_sources::{bundled_source, source_to_api_json};
        let bundled = bundled_source();
        let json = source_to_api_json(&bundled, false);
        assert!(json.get("id").is_some(), "must have id");
        assert!(json.get("label").is_some(), "must have label");
        assert!(json.get("type").is_some(), "must have type");
        assert!(json.get("enabled").is_some(), "must have enabled");
        assert!(json.get("removable").is_some(), "must have removable");
        assert_eq!(json["removable"], false);
        assert_eq!(json["type"], "refact_index");
    }

    #[test]
    fn test_smithery_source_has_api_key_fields() {
        use crate::http::routers::v1::mcp_marketplace_sources::{
            source_to_api_json, SMITHERY_SOURCE_ID,
        };
        let mut smithery = MarketplaceSource {
            id: SMITHERY_SOURCE_ID.to_string(),
            label: "Smithery.ai".to_string(),
            source_type: SourceType::Smithery,
            enabled: false,
            url: None,
            api_key: None,
        };
        let json_no_key = source_to_api_json(&smithery, true);
        assert_eq!(
            json_no_key["needs_api_key"], true,
            "smithery must need api key"
        );
        assert_eq!(
            json_no_key["has_api_key"], false,
            "no api key configured initially"
        );
        assert!(
            json_no_key.get("api_key_configured").is_none(),
            "must not use old field name"
        );

        smithery.api_key = Some("sk-test".to_string());
        let json_with_key = source_to_api_json(&smithery, true);
        assert_eq!(
            json_with_key["has_api_key"], true,
            "has_api_key must be true when key is set"
        );
    }

    #[test]
    fn test_merged_mode_deduplicates_servers() {
        let make_server = |id: &str, source: &str| MarketplaceServerWithSource {
            server: MarketplaceServer {
                id: id.to_string(),
                name: id.to_string(),
                description: "desc".to_string(),
                publisher: "pub".to_string(),
                tags: vec![],
                icon_url: None,
                homepage: None,
                transport: "stdio".to_string(),
                install_recipe: InstallRecipe {
                    command: Some("cmd".to_string()),
                    url: None,
                    env: HashMap::new(),
                    headers: HashMap::new(),
                    args_from_env: vec![],
                },
                confirmation_default: vec![],
            },
            source_id: source.to_string(),
        };

        let all_servers = vec![
            make_server("github", "refact-bundled"),
            make_server("github", "refact"),
            make_server("brave-search", "refact-bundled"),
        ];

        let mut seen_ids: HashSet<String> = HashSet::new();
        let deduped: Vec<MarketplaceServerWithSource> = all_servers
            .into_iter()
            .filter(|s| seen_ids.insert(s.server.id.clone()))
            .collect();

        assert_eq!(deduped.len(), 2, "duplicate github must be removed");
        assert!(
            deduped.iter().any(|s| s.server.id == "github"),
            "github must be present"
        );
        assert!(
            deduped.iter().any(|s| s.server.id == "brave-search"),
            "brave-search must be present"
        );
        let github = deduped.iter().find(|s| s.server.id == "github").unwrap();
        assert_eq!(github.source_id, "refact-bundled", "first occurrence wins");
    }

    #[test]
    fn test_merged_mode_excludes_smithery() {
        let smithery_source = MarketplaceSource {
            id: SMITHERY_SOURCE_ID.to_string(),
            label: "Smithery.ai".to_string(),
            source_type: SourceType::Smithery,
            enabled: true,
            url: None,
            api_key: Some("sk-test".to_string()),
        };
        let is_merged_mode = true;
        let should_skip = is_merged_mode && smithery_source.source_type == SourceType::Smithery;
        assert!(should_skip, "Smithery must be excluded in merged mode");

        let refact_source = MarketplaceSource {
            id: "refact-bundled".to_string(),
            label: "Refact Built-in".to_string(),
            source_type: SourceType::RefactIndex,
            enabled: true,
            url: None,
            api_key: None,
        };
        let should_skip_refact =
            is_merged_mode && refact_source.source_type == SourceType::Smithery;
        assert!(
            !should_skip_refact,
            "RefactIndex must not be excluded in merged mode"
        );
    }

    #[test]
    fn test_has_api_key_field_name_not_api_key_configured() {
        use crate::http::routers::v1::mcp_marketplace_sources::{
            source_to_api_json, SMITHERY_SOURCE_ID,
        };
        let smithery = MarketplaceSource {
            id: SMITHERY_SOURCE_ID.to_string(),
            label: "Smithery.ai".to_string(),
            source_type: SourceType::Smithery,
            enabled: true,
            url: None,
            api_key: Some("sk-test".to_string()),
        };
        let json = source_to_api_json(&smithery, true);
        assert!(
            json.get("has_api_key").is_some(),
            "field must be named has_api_key"
        );
        assert!(
            json.get("api_key_configured").is_none(),
            "old field name api_key_configured must not exist"
        );
    }

    #[test]
    fn test_build_integration_yaml_no_injection_malicious_env_key() {
        let server = MarketplaceServer {
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "desc".to_string(),
            publisher: "pub".to_string(),
            tags: vec![],
            icon_url: None,
            homepage: None,
            transport: "stdio".to_string(),
            install_recipe: InstallRecipe {
                command: Some("cmd".to_string()),
                url: None,
                env: HashMap::new(),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec![],
        };
        let mut env = HashMap::new();
        env.insert("MY_KEY".to_string(), "safe_value".to_string());
        let yaml = build_integration_yaml(&server, &env, &HashMap::new(), "refact-bundled");
        let parsed: serde_yaml::Value = serde_yaml::from_str(
            &yaml
                .lines()
                .filter(|l| !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let env_section = &parsed["env"];
        assert!(
            env_section["MY_KEY"].as_str().is_some(),
            "MY_KEY must be present"
        );
        assert_eq!(env_section["MY_KEY"].as_str().unwrap(), "safe_value");
        assert!(
            env_section.get("evil_field").is_none(),
            "injection must not create new YAML fields"
        );
    }

    #[test]
    fn test_build_integration_yaml_smithery_id_with_slash() {
        let server = MarketplaceServer {
            id: "acme/my-server".to_string(),
            name: "My Server".to_string(),
            description: "desc".to_string(),
            publisher: "acme".to_string(),
            tags: vec!["smithery".to_string()],
            icon_url: None,
            homepage: None,
            transport: "stdio".to_string(),
            install_recipe: InstallRecipe {
                command: Some("npx @acme/my-server".to_string()),
                url: None,
                env: HashMap::new(),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec!["*".to_string()],
        };
        let yaml = build_integration_yaml(&server, &HashMap::new(), &HashMap::new(), "smithery");
        assert!(
            yaml.contains("# mcp_marketplace_source: smithery"),
            "must have source comment"
        );
        assert!(
            yaml.contains("# mcp_marketplace_server: acme/my-server"),
            "must preserve slash in server ID"
        );
        assert!(
            yaml.contains("npx @acme/my-server"),
            "command must be present"
        );
    }

    #[test]
    fn test_parse_marketplace_comments_reads_headers() {
        let content = "# mcp_marketplace_source: smithery\n# mcp_marketplace_server: acme/my-server\ncommand: cmd\n";
        let (src, srv) = parse_marketplace_comments(content);
        assert_eq!(src.as_deref(), Some("smithery"));
        assert_eq!(srv.as_deref(), Some("acme/my-server"));
    }

    #[test]
    fn test_parse_marketplace_comments_missing_headers() {
        let content = "command: npx something\nenv:\n  KEY: val\n";
        let (src, srv) = parse_marketplace_comments(content);
        assert!(src.is_none(), "no source comment");
        assert!(srv.is_none(), "no server comment");
    }

    #[test]
    fn test_parse_marketplace_comments_partial_headers() {
        let content = "# mcp_marketplace_source: refact-bundled\ncommand: cmd\n";
        let (src, srv) = parse_marketplace_comments(content);
        assert_eq!(src.as_deref(), Some("refact-bundled"));
        assert!(srv.is_none(), "no server comment");
    }

    #[tokio::test]
    async fn test_installed_detection_reads_comment_headers() {
        let tmp = tempfile::tempdir().unwrap();
        let integrations_dir = tmp.path().join("integrations.d");
        tokio::fs::create_dir_all(&integrations_dir).await.unwrap();

        let smithery_yaml = "# mcp_marketplace_source: smithery\n# mcp_marketplace_server: acme/my-server\ncommand: npx @acme/my-server\n";
        tokio::fs::write(
            integrations_dir.join("mcp_stdio_acme_my_server.yaml"),
            smithery_yaml,
        )
        .await
        .unwrap();

        let bundled_yaml = "command: npx github\n";
        tokio::fs::write(integrations_dir.join("mcp_stdio_github.yaml"), bundled_yaml)
            .await
            .unwrap();

        let mut smithery_found = None;
        let bundled = bundled_index();
        let index_ids: std::collections::HashSet<String> =
            bundled.servers.iter().map(|s| s.id.clone()).collect();

        let mut rd = tokio::fs::read_dir(&integrations_dir).await.unwrap();
        while let Ok(Some(entry)) = rd.next_entry().await {
            let fname = entry.file_name();
            let fname_str = fname.to_string_lossy().to_string();
            if !fname_str.ends_with(".yaml") {
                continue;
            }
            let is_mcp = ["mcp_stdio_", "mcp_sse_", "mcp_http_"]
                .iter()
                .any(|p| fname_str.starts_with(p));
            if !is_mcp {
                continue;
            }
            let content = tokio::fs::read_to_string(entry.path()).await.unwrap();
            let (found_source, found_server) = parse_marketplace_comments(&content);
            if let (Some(src_id), Some(srv_id)) = (found_source, found_server) {
                if srv_id == "acme/my-server" {
                    smithery_found = Some(src_id);
                }
                continue;
            }
            for prefix in &["mcp_stdio_", "mcp_sse_", "mcp_http_"] {
                if let Some(rest) = fname_str.strip_prefix(prefix) {
                    let id_candidate = rest.trim_end_matches(".yaml").replace('_', "-");
                    if index_ids.contains(&id_candidate) {
                        assert_eq!(id_candidate, "github");
                    }
                    break;
                }
            }
        }
        assert_eq!(
            smithery_found.as_deref(),
            Some("smithery"),
            "must detect smithery server via comment headers"
        );
    }

    #[test]
    fn test_validate_env_key_valid() {
        assert!(validate_env_key("MY_KEY"), "simple env key");
        assert!(validate_env_key("GITHUB_TOKEN"), "env key with underscore");
        assert!(
            validate_env_key("_PRIVATE"),
            "env key starting with underscore"
        );
        assert!(validate_env_key("API-KEY"), "env key with dash");
    }

    #[test]
    fn test_validate_env_key_invalid() {
        assert!(!validate_env_key(""), "empty key invalid");
        assert!(!validate_env_key("evil:\nfield"), "newline in key invalid");
        assert!(
            !validate_env_key("evil: true\n  injection"),
            "injection invalid"
        );
        assert!(
            !validate_env_key("1STARTS_WITH_NUM"),
            "key starting with number invalid"
        );
    }

    #[test]
    fn test_official_mcp_default_source_enabled() {
        use crate::http::routers::v1::mcp_marketplace_sources::{
            default_sources_config_for_test, OFFICIAL_MCP_SOURCE_ID,
        };
        let cfg = default_sources_config_for_test();
        let official = cfg.sources.iter().find(|s| s.id == OFFICIAL_MCP_SOURCE_ID);
        assert!(
            official.is_some(),
            "official-mcp must be in default sources"
        );
        let official = official.unwrap();
        assert!(official.enabled, "official-mcp must be enabled by default");
        assert!(
            official.api_key.is_none(),
            "official-mcp must not require api key"
        );
        assert_eq!(official.source_type, SourceType::OfficialMcp);
    }

    #[test]
    fn test_official_mcp_source_json() {
        use crate::http::routers::v1::mcp_marketplace_sources::{
            source_to_api_json, OFFICIAL_MCP_SOURCE_ID,
        };
        let source = MarketplaceSource {
            id: OFFICIAL_MCP_SOURCE_ID.to_string(),
            label: "MCP Registry".to_string(),
            source_type: SourceType::OfficialMcp,
            enabled: true,
            url: None,
            api_key: None,
        };
        let json = source_to_api_json(&source, false);
        assert_eq!(
            json["type"], "official_mcp",
            "type must serialize as official_mcp"
        );
        assert_eq!(json["enabled"], true);
        assert!(
            json.get("needs_api_key").is_none(),
            "official-mcp must not have needs_api_key"
        );
        assert!(
            json.get("has_api_key").is_none(),
            "official-mcp must not have has_api_key"
        );
    }

    #[test]
    fn test_official_mcp_registry_response_mapping() {
        let json = r#"{
            "servers": [{
                "server": {
                    "name": "namespace/my-server",
                    "title": "My Server",
                    "description": "A test server",
                    "websiteUrl": "https://example.com",
                    "icons": [{"src": "https://example.com/icon.png"}],
                    "remotes": [{"type": "streamable-http", "url": "https://api.example.com/mcp"}],
                    "packages": []
                }
            }, {
                "server": {
                    "name": "other/stdio-server",
                    "description": "A stdio server",
                    "remotes": [],
                    "packages": [{"registry_name": "npm", "name": "@other/stdio-server"}]
                }
            }],
            "metadata": {"nextCursor": null, "count": 2}
        }"#;
        let resp: OfficialRegistryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers.len(), 2);
        assert_eq!(resp.metadata.count, 2);

        let s = &resp.servers[0].server;
        assert_eq!(s.name, "namespace/my-server");
        assert_eq!(s.title.as_deref(), Some("My Server"));
        assert_eq!(s.remotes[0].remote_type, "streamable-http");
        assert_eq!(s.remotes[0].url, "https://api.example.com/mcp");

        let s2 = &resp.servers[1].server;
        assert!(s2.remotes.is_empty());
        assert_eq!(s2.packages.len(), 1);
    }

    #[test]
    fn test_official_mcp_server_mapping() {
        let entry = OfficialRegistryEntry {
            server: OfficialRegistryServer {
                name: "acme/my-tool".to_string(),
                title: Some("My Tool".to_string()),
                description: Some("Does stuff".to_string()),
                website_url: Some("https://acme.com".to_string()),
                icons: vec![OfficialRegistryIcon {
                    src: "https://acme.com/icon.png".to_string(),
                }],
                remotes: vec![OfficialRegistryRemote {
                    remote_type: "streamable-http".to_string(),
                    url: "https://api.acme.com/mcp".to_string(),
                }],
                packages: vec![],
            },
        };
        let s = entry.server;
        let parts: Vec<&str> = s.name.splitn(2, '/').collect();
        let publisher = parts.first().copied().unwrap_or("").to_string();
        let short_name = parts.get(1).copied().unwrap_or(s.name.as_str());
        let display_name = s.title.unwrap_or_else(|| short_name.to_string());
        let (transport, install_url) = s
            .remotes
            .first()
            .map(|r| {
                let t = match r.remote_type.as_str() {
                    "streamable-http" => "http",
                    "sse" => "sse",
                    _ => "http",
                };
                (t.to_string(), Some(r.url.clone()))
            })
            .unwrap_or_else(|| ("stdio".to_string(), None));

        assert_eq!(publisher, "acme");
        assert_eq!(display_name, "My Tool");
        assert_eq!(transport, "http");
        assert_eq!(install_url.as_deref(), Some("https://api.acme.com/mcp"));
    }

    #[test]
    fn test_official_mcp_client_side_search_filter() {
        let servers = vec![
            MarketplaceServer {
                id: "acme/github-tool".to_string(),
                name: "GitHub Tool".to_string(),
                description: "Integrates with GitHub".to_string(),
                publisher: "acme".to_string(),
                tags: vec!["official-mcp".to_string()],
                icon_url: None,
                homepage: None,
                transport: "http".to_string(),
                install_recipe: InstallRecipe {
                    command: None,
                    url: Some("https://api.acme.com/mcp".to_string()),
                    env: HashMap::new(),
                    headers: HashMap::new(),
                    args_from_env: vec![],
                },
                confirmation_default: vec!["**".to_string()],
            },
            MarketplaceServer {
                id: "other/slack-tool".to_string(),
                name: "Slack Integration".to_string(),
                description: "Chat via Slack".to_string(),
                publisher: "other".to_string(),
                tags: vec!["official-mcp".to_string()],
                icon_url: None,
                homepage: None,
                transport: "stdio".to_string(),
                install_recipe: InstallRecipe {
                    command: None,
                    url: None,
                    env: HashMap::new(),
                    headers: HashMap::new(),
                    args_from_env: vec![],
                },
                confirmation_default: vec!["**".to_string()],
            },
        ];

        let q = "github".to_lowercase();
        let filtered: Vec<_> = servers
            .iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&q)
                    || s.description.to_lowercase().contains(&q)
                    || s.id.to_lowercase().contains(&q)
                    || s.publisher.to_lowercase().contains(&q)
            })
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "acme/github-tool");
    }

    #[test]
    fn test_official_mcp_included_in_merged_mode() {
        let official_source = MarketplaceSource {
            id: OFFICIAL_MCP_SOURCE_ID.to_string(),
            label: "MCP Registry".to_string(),
            source_type: SourceType::OfficialMcp,
            enabled: true,
            url: None,
            api_key: None,
        };
        let is_merged_mode = true;
        let should_skip = is_merged_mode && official_source.source_type == SourceType::Smithery;
        assert!(
            !should_skip,
            "OfficialMcp must NOT be excluded in merged mode"
        );
    }
}
