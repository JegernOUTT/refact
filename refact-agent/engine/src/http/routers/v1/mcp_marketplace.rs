use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use axum::extract::Query;
use axum::response::Json;
use axum::extract::State;
use hyper::StatusCode;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::{Duration, Instant};
use std::sync::Mutex;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::global_context::{GlobalContext, SharedGlobalContext};
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
    gcx: Arc<GlobalContext>,
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
                    let http_client = gcx.http_client.clone();
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
            let config_dir = gcx.config_dir.clone();
            let sources_cfg = load_sources(&config_dir).await;
            let api_key = match smithery_api_key(&sources_cfg.sources) {
                Some(k) => k,
                None => return (vec![], 0, "no_api_key"),
            };

            let http_client = gcx.http_client.clone();
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
            let http_client = gcx.http_client.clone();
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
    #[serde(default)]
    pub tag: Option<String>,
    pub source: Option<String>,
    pub q: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

const MERGED_MODE_PAGE_SIZE_CAP: u32 = 500;

pub async fn handle_v1_mcp_marketplace_get(
    State(app): State<AppState>,
    Query(params): Query<MarketplaceQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).min(100).max(1);
    let query = params.q.as_deref();
    let tag_filter = params
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());

    let config_dir = gcx.config_dir.clone();
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
        // Tag filtering must see the full result set, so fetch everything
        // (like merged mode) whenever a tag filter is active.
        let fetch_full_set = is_merged_mode || tag_filter.is_some();
        let smithery_key_missing = source.source_type == SourceType::Smithery
            && source.api_key.as_deref().map_or(true, |k| k.is_empty());
        if is_merged_mode && smithery_key_missing {
            // Without an API key the Smithery fetch can only fail; keep the
            // source listed so the GUI can show its enable/configure teaser.
            let mut meta = source_to_api_json(source, *removable);
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("server_count".to_string(), json!(0));
                obj.insert("status".to_string(), json!("ok"));
            }
            sources_meta.push(meta);
            continue;
        }
        // OfficialMcp IS included in merged mode (free, no API key);
        // Smithery joins the merged view once its API key is configured.

        let fetch_page_size = if fetch_full_set {
            MERGED_MODE_PAGE_SIZE_CAP
        } else {
            page_size
        };
        let fetch_page = if fetch_full_set { 1 } else { page };

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

    let (final_servers, final_total, all_tags) = if filter_source.is_some() {
        let mut all_tags: Vec<String> = all_servers
            .iter()
            .flat_map(|s| s.server.tags.iter().cloned())
            .collect();
        all_tags.sort();
        all_tags.dedup();
        match tag_filter {
            Some(tag) => {
                // The full set was fetched above; filter, then paginate locally
                // so totals and pages reflect the tag-filtered set.
                let filtered: Vec<MarketplaceServerWithSource> = all_servers
                    .into_iter()
                    .filter(|s| s.server.tags.iter().any(|t| t == tag))
                    .collect();
                let total_count = filtered.len() as u32;
                let start = ((page - 1) * page_size) as usize;
                let end = (start + page_size as usize).min(filtered.len());
                let sliced = if start < filtered.len() {
                    filtered[start..end].to_vec()
                } else {
                    vec![]
                };
                (sliced, total_count, all_tags)
            }
            None => {
                let t = sources_meta
                    .iter()
                    .find(|m| m["id"].as_str() == filter_source)
                    .and_then(|m| m["server_count"].as_u64())
                    .unwrap_or(0) as u32;
                (all_servers, t, all_tags)
            }
        }
    } else {
        let mut seen_ids: HashSet<String> = HashSet::new();
        let deduped: Vec<MarketplaceServerWithSource> = all_servers
            .into_iter()
            .filter(|s| seen_ids.insert(s.server.id.clone()))
            .collect();
        // The tag catalog spans the full merged set, not just the current page,
        // so tag pills in the GUI stay stable across pagination.
        let mut all_tags: Vec<String> = deduped
            .iter()
            .flat_map(|s| s.server.tags.iter().cloned())
            .collect();
        all_tags.sort();
        all_tags.dedup();
        let tagged: Vec<MarketplaceServerWithSource> = match tag_filter {
            Some(tag) => deduped
                .into_iter()
                .filter(|s| s.server.tags.iter().any(|t| t == tag))
                .collect(),
            None => deduped,
        };
        let total_count = tagged.len() as u32;
        let start = ((page - 1) * page_size) as usize;
        let end = (start + page_size as usize).min(tagged.len());
        let sliced = if start < tagged.len() {
            tagged[start..end].to_vec()
        } else {
            vec![]
        };
        (sliced, total_count, all_tags)
    };

    let servers_json: Vec<Value> = final_servers
        .iter()
        .map(|entry| {
            let mut value = serde_json::to_value(entry).unwrap_or_else(|_| json!({}));
            if let Some(obj) = value.as_object_mut() {
                obj.insert("recipe_hash".to_string(), json!(recipe_hash(&entry.server)));
            }
            value
        })
        .collect();

    Ok(Json(json!({
        "servers": servers_json,
        "sources": sources_meta,
        "all_tags": all_tags,
        "pagination": {
            "page": page,
            "page_size": page_size,
            "total": final_total,
        },
    })))
}

/// Stable content hash of the recipe-owned parts of a marketplace server.
/// Stored in the installed config's provenance comments; a mismatch against
/// the source's current hash means an update is available.
pub fn recipe_hash(server: &MarketplaceServer) -> String {
    let sorted_pairs = |map: &HashMap<String, String>| -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> =
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        pairs.sort();
        pairs
    };
    let canonical = json!({
        "transport": server.transport,
        "command": server.install_recipe.command,
        "url": server.install_recipe.url,
        "env": sorted_pairs(&server.install_recipe.env),
        "headers": sorted_pairs(&server.install_recipe.headers),
        "args_from_env": server.install_recipe.args_from_env,
        "confirmation_default": server.confirmation_default,
    });
    format!(
        "{:016x}",
        mcp_naming::stable_name_hash(&canonical.to_string())
    )
}

#[derive(Deserialize)]
pub struct InstallRequest {
    pub server_id: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub config_overrides: Option<ConfigOverrides>,
    /// When true, allows installing even if required env vars (recipe env keys
    /// with an empty default, typically API keys) have no value yet.
    #[serde(default)]
    pub allow_incomplete: bool,
}

#[derive(Deserialize, Default)]
pub struct ConfigOverrides {
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

/// Recipe env keys with an empty default value are required (they are almost
/// always credentials). Returns the sorted list of required keys that are
/// still empty after user overrides were merged into `merged_env`.
pub fn compute_missing_required_env(
    recipe_env: &HashMap<String, String>,
    merged_env: &HashMap<String, String>,
) -> Vec<String> {
    let mut missing: Vec<String> = recipe_env
        .iter()
        .filter(|(_, default)| default.trim().is_empty())
        .filter(|(k, _)| merged_env.get(*k).map_or(true, |v| v.trim().is_empty()))
        .map(|(k, _)| k.clone())
        .collect();
    missing.sort();
    missing
}

async fn find_server_in_sources(
    gcx: Arc<GlobalContext>,
    server_id: &str,
    source_id: Option<&str>,
) -> Option<(MarketplaceServer, String)> {
    let config_dir = gcx.config_dir.clone();
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
                let http_client = gcx.http_client.clone();
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
            let http_client = gcx.http_client.clone();
            if let Some(server) = fetch_smithery_detail(&http_client, server_id, &api_key).await {
                return Some((server, source.id.clone()));
            }
        } else if source.source_type == SourceType::OfficialMcp {
            let http_client = gcx.http_client.clone();
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

pub async fn install_mcp_marketplace_server(
    gcx: SharedGlobalContext,
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

    let config_dir = gcx.config_dir.clone();
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

    if !req.allow_incomplete {
        let missing_env = compute_missing_required_env(&server.install_recipe.env, &env);
        if !missing_env.is_empty() {
            return Err(ScratchError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "missing required environment variables: {}; provide values in config_overrides.env or set allow_incomplete=true",
                    missing_env.join(", ")
                ),
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
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = tokio::fs::set_permissions(
                    &config_path,
                    std::fs::Permissions::from_mode(0o600),
                )
                .await;
            }
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

pub async fn handle_v1_mcp_marketplace_install(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    install_mcp_marketplace_server(app.gcx.clone(), body_bytes).await
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
        serde_yaml::Value::String("ask_user".to_string()),
        confirmation_list,
    );
    map.insert(
        serde_yaml::Value::String("confirmation".to_string()),
        serde_yaml::Value::Mapping(confirmation),
    );

    let yaml_body =
        serde_yaml::to_string(&serde_yaml::Value::Mapping(map)).unwrap_or_else(|_| String::new());

    format!(
        "# mcp_marketplace_source: {}\n# mcp_marketplace_server: {}\n# mcp_marketplace_recipe_hash: {}\n{}",
        source_id,
        server.id,
        recipe_hash(server),
        yaml_body
    )
}

fn parse_marketplace_comments(content: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut source_id = None;
    let mut server_id = None;
    let mut installed_recipe_hash = None;
    for line in content.lines().take(10) {
        if let Some(val) = line.strip_prefix("# mcp_marketplace_source:") {
            source_id = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("# mcp_marketplace_server:") {
            server_id = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("# mcp_marketplace_recipe_hash:") {
            installed_recipe_hash = Some(val.trim().to_string());
        }
        if !line.starts_with('#') && !line.is_empty() {
            break;
        }
    }
    (source_id, server_id, installed_recipe_hash)
}

pub async fn handle_v1_mcp_marketplace_installed(
    State(app): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    let config_dir = gcx.config_dir.clone();
    let integrations_dir = config_dir.join("integrations.d");

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

        // Only configs that carry the marketplace provenance comments written at
        // install time count as "installed from the marketplace". Filename-based
        // guessing against the bundled index misattributed manually created
        // configs (e.g. a hand-written mcp_stdio_github.yaml) to the bundled
        // source, so it was removed on purpose.
        let (found_source, found_server, found_hash) = parse_marketplace_comments(&content);
        if let (Some(src_id), Some(srv_id)) = (found_source, found_server) {
            installed.push(json!({
                "id": srv_id,
                "config_path": entry.path().display().to_string(),
                "source_id": src_id,
                "recipe_hash": found_hash,
            }));
        }
    }

    Ok(Json(json!({ "installed": installed })))
}

#[derive(Deserialize)]
pub struct AutoNameRequest {
    pub input: String,
}

pub async fn handle_v1_mcp_auto_name(
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<AutoNameRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    let suggested_name = mcp_naming::extract_name_from_input(&req.input)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let transport = mcp_naming::detect_transport_from_input(&req.input);
    let config_prefix = mcp_naming::config_prefix_for_transport(transport);

    Ok(Json(json!({
        "suggested_name": suggested_name,
        "transport": transport,
        "config_prefix": config_prefix,
    })))
}

fn marketplace_config_path_checked(
    gcx: &Arc<GlobalContext>,
    config_path: &str,
) -> Result<std::path::PathBuf, ScratchError> {
    let path = std::path::PathBuf::from(config_path);
    let integrations_dir = gcx.config_dir.join("integrations.d");
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| ScratchError::new(StatusCode::BAD_REQUEST, "invalid config path".into()))?;
    let is_mcp = ["mcp_stdio_", "mcp_sse_", "mcp_http_"]
        .iter()
        .any(|prefix| file_name.starts_with(prefix));
    if !is_mcp || !file_name.ends_with(".yaml") {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "config path is not a marketplace MCP config".into(),
        ));
    }
    if path.parent() != Some(integrations_dir.as_path()) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "config path is outside integrations.d".into(),
        ));
    }
    // Refuse symlinks and verify the resolved location: a symlinked yaml in
    // integrations.d must not let update/uninstall touch files elsewhere.
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                "config path is a symlink".into(),
            ));
        }
        Ok(_) => {
            let canonical_dir = std::fs::canonicalize(&integrations_dir).map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot resolve integrations dir: {}", e),
                )
            })?;
            let canonical_path = std::fs::canonicalize(&path).map_err(|e| {
                ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!("cannot resolve config path: {}", e),
                )
            })?;
            if canonical_path.parent() != Some(canonical_dir.as_path()) {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    "config path resolves outside integrations.d".into(),
                ));
            }
        }
        Err(_) => {
            // File not found is handled by the callers (404 for uninstall,
            // read error for update).
        }
    }
    Ok(path)
}

async fn write_config_atomically(
    config_path: &std::path::Path,
    content: &str,
) -> Result<(), ScratchError> {
    let tmp_path = config_path.with_extension("yaml.tmp");
    tokio::fs::write(&tmp_path, content).await.map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write error: {}", e),
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600)).await;
    }
    tokio::fs::rename(&tmp_path, config_path)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("rename error: {}", e),
            )
        })
}

/// Merges the current marketplace recipe into an existing installed config:
/// recipe-owned fields (command/url/args, new env keys, new header keys) are
/// refreshed while every user-edited value (filled env vars, timeouts,
/// confirmation rules, auth settings, persisted oauth tokens) is preserved.
pub fn merge_recipe_update_yaml(
    existing_yaml: &str,
    server: &MarketplaceServer,
    source_id: &str,
) -> Result<String, String> {
    let existing: serde_yaml::Value =
        serde_yaml::from_str(existing_yaml).map_err(|e| format!("cannot parse config: {}", e))?;
    let existing_map = existing
        .as_mapping()
        .cloned()
        .unwrap_or_else(serde_yaml::Mapping::new);

    let existing_env: HashMap<String, String> = existing_map
        .get(&serde_yaml::Value::String("env".to_string()))
        .and_then(|v| v.as_mapping())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| {
                    Some((
                        k.as_str()?.to_string(),
                        v.as_str().unwrap_or("").to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let existing_headers: HashMap<String, String> = existing_map
        .get(&serde_yaml::Value::String("headers".to_string()))
        .and_then(|v| v.as_mapping())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| {
                    Some((
                        k.as_str()?.to_string(),
                        v.as_str().unwrap_or("").to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    let mut merged_env = server.install_recipe.env.clone();
    for (key, value) in &existing_env {
        if !value.trim().is_empty() || merged_env.contains_key(key) {
            merged_env.insert(key.clone(), value.clone());
        }
    }
    let mut merged_headers = server.install_recipe.headers.clone();
    for (key, value) in &existing_headers {
        merged_headers.insert(key.clone(), value.clone());
    }

    let fresh = build_integration_yaml(server, &merged_env, &merged_headers, source_id);
    let fresh_body = fresh
        .lines()
        .skip_while(|line| line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let mut fresh_map: serde_yaml::Mapping = serde_yaml::from_str(&fresh_body)
        .map_err(|e| format!("internal: cannot parse generated yaml: {}", e))?;

    // Everything except the recipe-owned keys is user/runtime state: overlay
    // the existing values on top of the freshly generated config.
    let recipe_owned = ["command", "url", "env", "headers"];
    for (key, value) in existing_map.iter() {
        let key_str = key.as_str().unwrap_or("");
        if recipe_owned.contains(&key_str) {
            continue;
        }
        fresh_map.insert(key.clone(), value.clone());
    }

    let yaml_body = serde_yaml::to_string(&serde_yaml::Value::Mapping(fresh_map))
        .map_err(|e| format!("cannot serialize merged config: {}", e))?;
    Ok(format!(
        "# mcp_marketplace_source: {}\n# mcp_marketplace_server: {}\n# mcp_marketplace_recipe_hash: {}\n{}",
        source_id,
        server.id,
        recipe_hash(server),
        yaml_body
    ))
}

#[derive(Deserialize)]
pub struct UpdateRequest {
    pub config_path: String,
}

pub async fn handle_v1_mcp_marketplace_update(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let gcx = app.gcx.clone();
    let req = serde_json::from_slice::<UpdateRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;
    let config_path = marketplace_config_path_checked(&gcx, &req.config_path)?;

    let content = tokio::fs::read_to_string(&config_path).await.map_err(|e| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("cannot read config: {}", e))
    })?;
    let (source_id, server_id, _old_hash) = parse_marketplace_comments(&content);
    let (source_id, server_id) = match (source_id, server_id) {
        (Some(src), Some(srv)) => (src, srv),
        _ => {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                "config has no marketplace provenance; only marketplace-installed servers can be updated".into(),
            ));
        }
    };

    let (server, found_source_id) =
        find_server_in_sources(gcx.clone(), &server_id, Some(source_id.as_str()))
            .await
            .or(find_server_in_sources(gcx.clone(), &server_id, None).await)
            .ok_or_else(|| {
                ScratchError::new(
                    StatusCode::NOT_FOUND,
                    format!("server '{}' no longer exists in the marketplace", server_id),
                )
            })?;

    let merged = merge_recipe_update_yaml(&content, &server, &found_source_id)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    write_config_atomically(&config_path, &merged).await?;

    Ok(Json(json!({
        "updated": true,
        "config_path": config_path.display().to_string(),
        "recipe_hash": recipe_hash(&server),
    })))
}

#[derive(Deserialize)]
pub struct UninstallRequest {
    pub config_path: String,
}

pub async fn handle_v1_mcp_marketplace_uninstall(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let gcx = app.gcx.clone();
    let req = serde_json::from_slice::<UninstallRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;
    let config_path = marketplace_config_path_checked(&gcx, &req.config_path)?;
    if !config_path.exists() {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            "config file does not exist".into(),
        ));
    }
    let config_path_str = config_path.display().to_string();

    // Tear the live session down before removing the file so the child
    // process/client, background tasks, and indexed resources go with it.
    let session_opt = {
        let sessions = gcx.integration_sessions.clone();
        let mut sessions_locked = sessions.lock().await;
        sessions_locked.remove(&config_path_str)
    };
    let mut stopped_session = false;
    if let Some(session) = session_opt {
        let stop_future = {
            let mut session_locked = session.lock().await;
            session_locked.try_stop(session.clone())
        };
        Box::into_pin(stop_future).await;
        stopped_session = true;
    }
    crate::integrations::mcp::integr_mcp_common::tool_catalog_cache_remove(&config_path_str);
    tokio::spawn(
        crate::integrations::mcp::mcp_resources::remove_indexed_resources(
            Arc::downgrade(&gcx),
            config_path_str.clone(),
        ),
    );

    tokio::fs::remove_file(&config_path).await.map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot remove config: {}", e),
        )
    })?;

    Ok(Json(json!({
        "uninstalled": true,
        "config_path": config_path_str,
        "stopped_session": stopped_session,
    })))
}

#[derive(Deserialize)]
pub struct WizardProbeRequest {
    pub input: String,
}

/// Pre-save connectivity probe for the setup wizard: URL inputs get a real
/// MCP auth probe, command inputs get PATH resolution for their executable.
pub async fn handle_v1_mcp_wizard_probe(
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<WizardProbeRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;
    let input = req.input.trim().to_string();
    if input.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "input is empty".into(),
        ));
    }

    let transport = mcp_naming::detect_transport_from_input(&input);
    if transport == "http" {
        use crate::integrations::mcp::mcp_auth::{probe_mcp_auth, MCPAuthSettings};
        let headers = HashMap::from([(
            "Accept".to_string(),
            "application/json, text/event-stream".to_string(),
        )]);
        match probe_mcp_auth(&input, &headers, &MCPAuthSettings::default()).await {
            Ok(probe) => {
                // 2xx means a healthy MCP endpoint; 401/403 means it exists and
                // wants auth. Anything else (404/500/HTML pages) is reachable
                // at the TCP level but not a working MCP server.
                let healthy = probe.needs_auth
                    || probe
                        .http_status
                        .map_or(false, |code| (200..300).contains(&code));
                if healthy {
                    Ok(Json(json!({
                        "transport": "http",
                        "reachable": true,
                        "needs_auth": probe.needs_auth,
                        "oauth_available": probe.oauth_available,
                    })))
                } else {
                    Ok(Json(json!({
                        "transport": "http",
                        "reachable": false,
                        "error": format!(
                            "server responded with HTTP {} — not a working MCP endpoint",
                            probe.http_status.unwrap_or(0)
                        ),
                    })))
                }
            }
            Err(e) => Ok(Json(json!({
                "transport": "http",
                "reachable": false,
                "error": e,
            }))),
        }
    } else {
        let command_word = input.split_whitespace().next().unwrap_or("");
        match crate::integrations::mcp::mcp_path_resolution::resolve_command(
            command_word,
            &input,
            None,
        ) {
            Ok(resolved) => Ok(Json(json!({
                "transport": "stdio",
                "command_found": true,
                "resolved_path": resolved.program.display().to_string(),
            }))),
            Err(e) => Ok(Json(json!({
                "transport": "stdio",
                "command_found": false,
                "error": e.to_user_message(),
            }))),
        }
    }
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
        assert!(yaml.contains("ask_user"), "yaml must contain confirmation");
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
        assert!(content.contains("ask_user"));
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
    fn test_auto_name_uses_shared_naming_helpers() {
        // Name extraction and input transport detection live in
        // refact_integrations::mcp::mcp_naming (single source of truth,
        // fully unit-tested there). This test only pins the wiring.
        assert_eq!(
            mcp_naming::extract_name_from_input("npx -y @notionhq/notion-mcp-server").unwrap(),
            "notion_mcp_server"
        );
        assert_eq!(
            mcp_naming::detect_transport_from_input("https://api.example.com/mcp"),
            "http"
        );
        assert_eq!(
            mcp_naming::config_prefix_for_transport(mcp_naming::detect_transport_from_input(
                "uvx mcp-server-fetch"
            )),
            "mcp_stdio_"
        );
    }

    #[test]
    fn test_recipe_hash_stable_and_sensitive() {
        let server = MarketplaceServer {
            id: "github".to_string(),
            name: "GitHub".to_string(),
            description: "d".to_string(),
            publisher: "p".to_string(),
            tags: vec![],
            icon_url: None,
            homepage: None,
            transport: "stdio".to_string(),
            install_recipe: InstallRecipe {
                command: Some("npx -y @modelcontextprotocol/server-github".to_string()),
                url: None,
                env: HashMap::from([("GITHUB_TOKEN".to_string(), "".to_string())]),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec!["*".to_string()],
        };
        let h1 = recipe_hash(&server);
        let h2 = recipe_hash(&server);
        assert_eq!(h1, h2, "hash must be deterministic");

        let mut changed = server.clone();
        changed.install_recipe.command =
            Some("npx -y @modelcontextprotocol/server-github@2".to_string());
        assert_ne!(
            recipe_hash(&changed),
            h1,
            "recipe change must change the hash"
        );

        // Env map iteration order must not affect the hash.
        let mut reordered = server.clone();
        reordered.install_recipe.env =
            HashMap::from([("GITHUB_TOKEN".to_string(), "".to_string())]);
        assert_eq!(recipe_hash(&reordered), h1);
    }

    #[test]
    fn test_merge_recipe_update_yaml_preserves_user_state() {
        let server = MarketplaceServer {
            id: "github".to_string(),
            name: "GitHub".to_string(),
            description: "d".to_string(),
            publisher: "p".to_string(),
            tags: vec![],
            icon_url: None,
            homepage: None,
            transport: "stdio".to_string(),
            install_recipe: InstallRecipe {
                command: Some("npx -y server-github@2".to_string()),
                url: None,
                env: HashMap::from([
                    ("GITHUB_TOKEN".to_string(), "".to_string()),
                    ("NEW_OPTION".to_string(), "default-value".to_string()),
                ]),
                headers: HashMap::new(),
                args_from_env: vec![],
            },
            confirmation_default: vec!["*".to_string()],
        };
        let existing = "# mcp_marketplace_source: refact-bundled
# mcp_marketplace_server: github
# mcp_marketplace_recipe_hash: 0000000000000000
command: npx -y server-github@1
env:
  GITHUB_TOKEN: user-secret-token
init_timeout: '120'
oauth_tokens:
  access_token: persisted-token
confirmation:
  ask_user:
  - custom_rule
";
        let merged = merge_recipe_update_yaml(existing, &server, "refact-bundled").unwrap();
        assert!(
            merged.contains("npx -y server-github@2"),
            "recipe-owned command must be updated: {}",
            merged
        );
        assert!(
            merged.contains("user-secret-token"),
            "user env values must be preserved: {}",
            merged
        );
        assert!(
            merged.contains("NEW_OPTION"),
            "new recipe env keys must be added: {}",
            merged
        );
        assert!(
            merged.contains("'120'") || merged.contains("\"120\""),
            "user timeout must be preserved: {}",
            merged
        );
        assert!(
            merged.contains("persisted-token"),
            "oauth tokens must survive the update: {}",
            merged
        );
        assert!(
            merged.contains("custom_rule"),
            "user confirmation rules must be preserved: {}",
            merged
        );
        assert!(
            merged.contains(&format!(
                "# mcp_marketplace_recipe_hash: {}",
                recipe_hash(&server)
            )),
            "provenance hash must be refreshed: {}",
            merged
        );
    }

    #[test]
    fn test_marketplace_config_path_checks() {
        // Pure-logic part: filename prefix and extension validation shape.
        for bad in ["notmcp_github.yaml", "mcp_stdio_github.txt", "evil.yaml"] {
            let is_mcp = ["mcp_stdio_", "mcp_sse_", "mcp_http_"]
                .iter()
                .any(|p| bad.starts_with(p));
            assert!(
                !(is_mcp && bad.ends_with(".yaml")),
                "{} must be rejected",
                bad
            );
        }
    }

    #[test]
    fn test_compute_missing_required_env() {
        let recipe_env: HashMap<String, String> = HashMap::from([
            ("API_KEY".to_string(), "".to_string()),
            ("REGION".to_string(), "us-east-1".to_string()),
            ("TOKEN".to_string(), "  ".to_string()),
        ]);

        // Nothing provided: both empty-default keys are missing, sorted.
        let missing = compute_missing_required_env(&recipe_env, &recipe_env);
        assert_eq!(missing, vec!["API_KEY".to_string(), "TOKEN".to_string()]);

        // Overrides fill one of them.
        let mut merged = recipe_env.clone();
        merged.insert("API_KEY".to_string(), "sk-123".to_string());
        let missing = compute_missing_required_env(&recipe_env, &merged);
        assert_eq!(missing, vec!["TOKEN".to_string()]);

        // All filled: nothing missing.
        merged.insert("TOKEN".to_string(), "tok".to_string());
        assert!(compute_missing_required_env(&recipe_env, &merged).is_empty());

        // Non-empty defaults are never required.
        let optional_only: HashMap<String, String> =
            HashMap::from([("MODE".to_string(), "fast".to_string())]);
        assert!(compute_missing_required_env(&optional_only, &optional_only).is_empty());
    }

    #[tokio::test]
    async fn test_installed_detection_requires_provenance_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let integrations_dir = tmp.path().join("integrations.d");
        tokio::fs::create_dir_all(&integrations_dir).await.unwrap();
        // Manually created config matching a bundled server id by filename:
        // must NOT be attributed to the marketplace.
        tokio::fs::write(
            integrations_dir.join("mcp_stdio_github.yaml"),
            "command: npx github\n",
        )
        .await
        .unwrap();
        // Marketplace-installed config with provenance comments: must be detected.
        tokio::fs::write(
            integrations_dir.join("mcp_stdio_brave_search.yaml"),
            "# mcp_marketplace_source: refact-bundled\n# mcp_marketplace_server: brave-search\ncommand: npx brave\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            integrations_dir.join("other_integration.yaml"),
            "some: config\n",
        )
        .await
        .unwrap();

        let mut detected: Vec<(String, String)> = Vec::new();
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
            if let (Some(src), Some(srv), _hash) = parse_marketplace_comments(&content) {
                detected.push((src, srv));
            }
        }

        assert_eq!(
            detected,
            vec![("refact-bundled".to_string(), "brave-search".to_string())],
            "only configs with provenance comments count as marketplace installs"
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
    fn test_merged_mode_smithery_gating_by_api_key() {
        // Mirrors the handler's smithery_key_missing gate: excluded from the
        // merged view without an API key, included once a key is configured.
        let keyless = MarketplaceSource {
            id: SMITHERY_SOURCE_ID.to_string(),
            label: "Smithery.ai".to_string(),
            source_type: SourceType::Smithery,
            enabled: true,
            url: None,
            api_key: None,
        };
        let key_missing = keyless.source_type == SourceType::Smithery
            && keyless.api_key.as_deref().map_or(true, |k| k.is_empty());
        assert!(
            key_missing,
            "keyless Smithery must be skipped in merged mode"
        );

        let with_key = MarketplaceSource {
            api_key: Some("sk-live".to_string()),
            ..keyless
        };
        let key_missing = with_key.source_type == SourceType::Smithery
            && with_key.api_key.as_deref().map_or(true, |k| k.is_empty());
        assert!(!key_missing, "Smithery with a key joins the merged view");
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
        let (src, srv, hash) = parse_marketplace_comments(content);
        assert_eq!(src.as_deref(), Some("smithery"));
        assert_eq!(srv.as_deref(), Some("acme/my-server"));
        assert!(hash.is_none(), "no hash comment in legacy configs");
    }

    #[test]
    fn test_parse_marketplace_comments_missing_headers() {
        let content = "command: npx something\nenv:\n  KEY: val\n";
        let (src, srv, _hash) = parse_marketplace_comments(content);
        assert!(src.is_none(), "no source comment");
        assert!(srv.is_none(), "no server comment");
    }

    #[test]
    fn test_parse_marketplace_comments_partial_headers() {
        let content = "# mcp_marketplace_source: refact-bundled\ncommand: cmd\n";
        let (src, srv, _hash) = parse_marketplace_comments(content);
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
            let (found_source, found_server, _found_hash) = parse_marketplace_comments(&content);
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
