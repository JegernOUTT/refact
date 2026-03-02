use std::collections::HashMap;
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
use crate::http::routers::v1::mcp_marketplace_sources::{
    load_sources, get_all_sources, smithery_api_key,
    BUNDLED_SOURCE_ID, SourceType, MarketplaceSource,
};
#[cfg(test)]
use crate::http::routers::v1::mcp_marketplace_sources::SMITHERY_SOURCE_ID;

const BUNDLED_CACHE_TTL_SECS: u64 = 3600;
const SMITHERY_CACHE_TTL_SECS: u64 = 900;

static SOURCE_CACHES: Mutex<Option<HashMap<String, (Instant, Vec<MarketplaceServerWithSource>)>>> = Mutex::new(None);

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
    serde_json::from_str(include_str!("../../../yaml_configs/mcp_marketplace_index.json"))
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

    let data: SmitheryListResponse = resp.json().await
        .map_err(|e| format!("smithery parse: {}", e))?;

    let servers: Vec<MarketplaceServer> = data.servers.into_iter().map(|s| {
        let transport = if s.remote.unwrap_or(false) { "http" } else { "stdio" }.to_string();
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
            },
            confirmation_default: vec!["*".to_string()],
        }
    }).collect();

    Ok((servers, data.pagination.total_count))
}

async fn fetch_smithery_detail(http_client: &reqwest::Client, qualified_name: &str, api_key: &str) -> Option<MarketplaceServer> {
    let url = format!("https://registry.smithery.ai/servers/{}", utf8_percent_encode(qualified_name, NON_ALPHANUMERIC));
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

    let transport_type = data["connections"].as_array()
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
        data["connections"].as_array()
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
        name: data["displayName"].as_str().unwrap_or(qualified_name).to_string(),
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
        },
        confirmation_default: vec!["*".to_string()],
    })
}

async fn load_source_servers(
    gcx: Arc<ARwLock<GlobalContext>>,
    source: &MarketplaceSource,
    query: Option<&str>,
    page: u32,
    page_size: u32,
    cache: &mut HashMap<String, (Instant, Vec<MarketplaceServerWithSource>)>,
) -> (Vec<MarketplaceServerWithSource>, u32, &'static str) {
    let ttl = if source.source_type == SourceType::Smithery {
        SMITHERY_CACHE_TTL_SECS
    } else {
        BUNDLED_CACHE_TTL_SECS
    };

    let query_str = query.unwrap_or("");
    let cache_key = format!("{}:{}", source.id, query_str);

    if let Some((ts, cached)) = cache.get(&cache_key) {
        if ts.elapsed().as_secs() < ttl {
            let total = cached.len() as u32;
            let start = ((page - 1) * page_size) as usize;
            let end = (start + page_size as usize).min(cached.len());
            let page_items = if start < cached.len() { cached[start..end].to_vec() } else { vec![] };
            return (page_items, total, "cached");
        }
    }

    match source.source_type {
        SourceType::RefactIndex => {
            let (index, status): (MarketplaceIndex, &'static str) = if source.id == BUNDLED_SOURCE_ID {
                (bundled_index(), "bundled")
            } else {
                let http_client = gcx.read().await.http_client.clone();
                match source.url.as_deref() {
                    Some(url) => match fetch_refact_index(&http_client, url).await {
                        Some(idx) => (idx, "remote"),
                        None => (MarketplaceIndex { version: 1, updated_at: String::new(), servers: vec![] }, "error"),
                    },
                    None => (MarketplaceIndex { version: 1, updated_at: String::new(), servers: vec![] }, "error"),
                }
            };

            let source_id = source.id.clone();
            let all_with_source: Vec<MarketplaceServerWithSource> = index.servers.into_iter()
                .filter(|s| {
                    if query_str.is_empty() { return true; }
                    let q = query_str.to_lowercase();
                    s.name.to_lowercase().contains(&q)
                        || s.description.to_lowercase().contains(&q)
                        || s.tags.iter().any(|t| t.to_lowercase().contains(&q))
                })
                .map(|s| MarketplaceServerWithSource { server: s, source_id: source_id.clone() })
                .collect();

            let total = all_with_source.len() as u32;
            cache.insert(cache_key, (Instant::now(), all_with_source.clone()));
            let start = ((page - 1) * page_size) as usize;
            let end = (start + page_size as usize).min(all_with_source.len());
            let page_items = if start < all_with_source.len() { all_with_source[start..end].to_vec() } else { vec![] };
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
                    let with_source: Vec<MarketplaceServerWithSource> = servers.into_iter()
                        .map(|s| MarketplaceServerWithSource { server: s, source_id: source_id.clone() })
                        .collect();
                    (with_source, total, "ok")
                }
                Err(_) => (vec![], 0, "error"),
            }
        }
    }
}

fn validate_config_name(name: &str) -> bool {
    !name.is_empty() && !name.contains("..") && !name.contains('/') && !name.contains('\\')
}

#[derive(Deserialize)]
pub struct MarketplaceQuery {
    pub source: Option<String>,
    pub q: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn handle_v1_mcp_marketplace_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(params): Query<MarketplaceQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).min(100).max(1);
    let query = params.q.as_deref();

    let config_dir = gcx.read().await.config_dir.clone();
    let (bundled, user_sources) = get_all_sources(&config_dir).await;

    let mut all_sources: Vec<MarketplaceSource> = vec![bundled];
    all_sources.extend(user_sources);

    let filter_source = params.source.as_deref();
    if let Some(fsrc) = filter_source {
        if !all_sources.iter().any(|s| s.id == fsrc) {
            return Err((StatusCode::NOT_FOUND, format!("source '{}' not found", fsrc)));
        }
    }

    let mut cache = get_cache();
    let mut all_servers: Vec<MarketplaceServerWithSource> = vec![];
    let mut sources_meta: Vec<Value> = vec![];

    for source in &all_sources {
        if !source.enabled {
            continue;
        }
        if let Some(fsrc) = filter_source {
            if source.id != fsrc {
                continue;
            }
        }

        let (page_items, total, status) = load_source_servers(
            gcx.clone(),
            source,
            query,
            if filter_source.is_some() { page } else { 1 },
            if filter_source.is_some() { page_size } else { u32::MAX },
            &mut cache,
        ).await;

        sources_meta.push(json!({
            "id": source.id,
            "label": source.label,
            "server_count": total,
            "status": status,
        }));

        all_servers.extend(page_items);
    }

    set_cache(cache);

    let total_count = all_servers.len() as u32;
    let (final_servers, final_total) = if filter_source.is_some() {
        let t = sources_meta.first().and_then(|m| m["server_count"].as_u64()).unwrap_or(0) as u32;
        (all_servers, t)
    } else {
        let start = ((page - 1) * page_size) as usize;
        let end = (start + page_size as usize).min(all_servers.len());
        let sliced = if start < all_servers.len() { all_servers[start..end].to_vec() } else { vec![] };
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

    if !validate_config_name(&req.server_id) {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, "invalid server_id".to_string()));
    }

    let (server, _found_source_id) = find_server_in_sources(
        gcx.clone(),
        &req.server_id,
        req.source_id.as_deref(),
    ).await.ok_or_else(|| ScratchError::new(StatusCode::NOT_FOUND, format!("server '{}' not found in marketplace", req.server_id)))?;

    match server.transport.as_str() {
        "http" | "streamable-http" | "sse" => {
            if server.install_recipe.url.is_none() {
                return Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("server '{}' has transport '{}' but no url in recipe", server.id, server.transport)));
            }
        }
        _ => {
            if server.install_recipe.command.is_none() {
                return Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("server '{}' has transport 'stdio' but no command in recipe", server.id)));
            }
        }
    }

    let config_dir = gcx.read().await.config_dir.clone();
    let integrations_dir = config_dir.join("integrations.d");
    tokio::fs::create_dir_all(&integrations_dir).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("cannot create integrations dir: {}", e)))?;

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
            env.insert(k.clone(), v.clone());
        }
        for (k, v) in &overrides.headers {
            headers.insert(k.clone(), v.clone());
        }
    }

    let yaml_content = build_integration_yaml(&server, &env, &headers);
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config_path)
        .await
    {
        Ok(mut file) => {
            use tokio::io::AsyncWriteExt;
            file.write_all(yaml_content.as_bytes()).await
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("write error: {}", e)))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ScratchError::new(StatusCode::CONFLICT, format!("config file '{}' already exists", filename)));
        }
        Err(e) => {
            return Err(ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("create error: {}", e)));
        }
    }

    Ok(Json(json!({
        "installed": true,
        "config_path": config_path.display().to_string(),
    })))
}

fn build_integration_yaml(server: &MarketplaceServer, env: &HashMap<String, String>, headers: &HashMap<String, String>) -> String {
    let mut lines = Vec::new();
    match server.transport.as_str() {
        "http" | "streamable-http" => {
            if let Some(ref url) = server.install_recipe.url {
                lines.push(format!("url: {:?}", url));
            }
            lines.push("headers:".to_string());
            if headers.is_empty() {
                lines.push("  {}".to_string());
            } else {
                for (k, v) in headers {
                    lines.push(format!("  {}: {:?}", k, v));
                }
            }
            lines.push("auth_type: \"none\"".to_string());
        }
        "sse" => {
            if let Some(ref url) = server.install_recipe.url {
                lines.push(format!("url: {:?}", url));
            }
            lines.push("headers:".to_string());
            if headers.is_empty() {
                lines.push("  {}".to_string());
            } else {
                for (k, v) in headers {
                    lines.push(format!("  {}: {:?}", k, v));
                }
            }
            lines.push("auth_type: \"none\"".to_string());
        }
        _ => {
            if let Some(ref cmd) = server.install_recipe.command {
                lines.push(format!("command: {:?}", cmd));
            }
            lines.push("env:".to_string());
            if env.is_empty() {
                lines.push("  {}".to_string());
            } else {
                for (k, v) in env {
                    lines.push(format!("  {}: {:?}", k, v));
                }
            }
        }
    }
    lines.push("init_timeout: \"60\"".to_string());
    lines.push("request_timeout: \"30\"".to_string());
    lines.push("available:".to_string());
    lines.push("  on_your_laptop: true".to_string());
    lines.push("  when_isolated: false".to_string());
    lines.push("confirmation:".to_string());
    if server.confirmation_default.is_empty() {
        lines.push("  ask_user_default: []".to_string());
    } else {
        let items: Vec<String> = server.confirmation_default.iter().map(|s| format!("\"{}\"", s)).collect();
        lines.push(format!("  ask_user_default: [{}]", items.join(", ")));
    }
    lines.join("\n") + "\n"
}

pub async fn handle_v1_mcp_marketplace_installed(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let config_dir = gcx.read().await.config_dir.clone();
    let integrations_dir = config_dir.join("integrations.d");

    let bundled = bundled_index();
    let index_ids: std::collections::HashSet<String> = bundled.servers.iter().map(|s| s.id.clone()).collect();
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
        for prefix in &["mcp_stdio_", "mcp_sse_", "mcp_http_"] {
            if let Some(rest) = fname_str.strip_prefix(prefix) {
                let id_candidate = rest.trim_end_matches(".yaml").replace('_', "-");
                if index_ids.contains(&id_candidate) {
                    let server = bundled.servers.iter().find(|s| s.id == id_candidate).unwrap();
                    installed.push(json!({
                        "id": id_candidate,
                        "name": server.name,
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
    let host = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host = host.split(':').next().unwrap_or(host);
    let parts: Vec<&str> = host.split('.').collect();
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
        if i > 0 && (args[i - 1] == "-e" || args[i - 1] == "--env" || args[i - 1] == "-p" || args[i - 1] == "--port") {
            continue;
        }
        candidate = arg;
        if *arg != "npx" && *arg != "uvx" && *arg != "docker" && *arg != "node" && *arg != "python" && *arg != "python3" {
            break;
        }
    }
    let name = candidate
        .rsplit('/')
        .next()
        .unwrap_or(candidate);
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
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();
    let snake = snake.trim_matches('_').to_string();
    let snake: String = {
        let mut prev_underscore = false;
        snake.chars().filter(|c| {
            if *c == '_' {
                if prev_underscore {
                    return false;
                }
                prev_underscore = true;
            } else {
                prev_underscore = false;
            }
            true
        }).collect()
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
        assert!(index.servers.len() >= 30, "must have at least 30 servers, got {}", index.servers.len());
    }

    #[test]
    fn test_bundled_index_all_servers_have_required_fields() {
        let index = bundled_index();
        for server in &index.servers {
            assert!(!server.id.is_empty(), "server id must not be empty");
            assert!(!server.name.is_empty(), "server name must not be empty for id={}", server.id);
            assert!(!server.description.is_empty(), "server description must not be empty for id={}", server.id);
            assert!(!server.transport.is_empty(), "server transport must not be empty for id={}", server.id);
        }
    }

    #[test]
    fn test_bundled_index_no_duplicate_ids() {
        let index = bundled_index();
        let mut ids = std::collections::HashSet::new();
        for server in &index.servers {
            assert!(ids.insert(server.id.clone()), "duplicate server id: {}", server.id);
        }
    }

    #[test]
    fn test_bundled_index_expanded() {
        let index = bundled_index();
        assert!(index.servers.len() >= 30, "bundled index must have at least 30 servers");
    }

    #[test]
    fn test_validate_config_name() {
        assert!(validate_config_name("github"), "valid name");
        assert!(validate_config_name("my-server"), "valid name with dash");
        assert!(!validate_config_name(""), "empty name invalid");
        assert!(!validate_config_name("../evil"), "path traversal invalid");
        assert!(!validate_config_name("a/b"), "slash invalid");
        assert!(!validate_config_name("a\\b"), "backslash invalid");
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
            },
            confirmation_default: vec!["*".to_string()],
        };
        let mut env = HashMap::new();
        env.insert("GITHUB_PERSONAL_ACCESS_TOKEN".to_string(), "ghp_test".to_string());
        let yaml = build_integration_yaml(&server, &env, &HashMap::new());
        assert!(yaml.contains("npx -y @modelcontextprotocol/server-github"), "yaml must contain command");
        assert!(yaml.contains("GITHUB_PERSONAL_ACCESS_TOKEN"), "yaml must contain env key");
        assert!(yaml.contains("ghp_test"), "yaml must contain env value");
        assert!(yaml.contains("init_timeout"), "yaml must contain init_timeout");
        assert!(yaml.contains("request_timeout"), "yaml must contain request_timeout");
        assert!(yaml.contains("ask_user_default"), "yaml must contain confirmation");
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
            },
            confirmation_default: vec!["*".to_string()],
        };
        let yaml = build_integration_yaml(&server, &HashMap::new(), &HashMap::new());
        assert!(yaml.contains("env:"), "yaml must contain env section");
        assert!(yaml.contains("{}"), "yaml must show empty env as {{}}");
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
            },
            confirmation_default: vec![],
        };
        let yaml = build_integration_yaml(&server, &HashMap::new(), &HashMap::new());
        assert!(yaml.contains("url:"), "yaml must contain url");
        assert!(yaml.contains("auth_type:"), "yaml must contain auth_type");
        assert!(yaml.contains("headers:"), "yaml must contain headers");
        assert!(!yaml.contains("command:"), "yaml must not contain command for http");
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
            },
            confirmation_default: vec![],
        };
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token123".to_string());
        let yaml = build_integration_yaml(&server, &HashMap::new(), &headers);
        assert!(yaml.contains("url:"), "yaml must contain url");
        assert!(yaml.contains("Authorization"), "yaml must contain Authorization header");
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
                install_recipe: InstallRecipe { command: Some("cmd".to_string()), url: None, env: HashMap::new(), headers: HashMap::new() },
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
                install_recipe: InstallRecipe { command: None, url: Some("https://ex.com".to_string()), env: HashMap::new(), headers: HashMap::new() },
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
                install_recipe: InstallRecipe { command: Some("cmd".to_string()), url: None, env: HashMap::new(), headers: HashMap::new() },
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
        let mut cache: HashMap<String, (Instant, Vec<MarketplaceServerWithSource>)> = HashMap::new();
        cache.insert("source-a:".to_string(), (Instant::now(), vec![]));
        cache.insert("source-b:".to_string(), (Instant::now(), vec![]));
        assert!(cache.contains_key("source-a:"));
        assert!(cache.contains_key("source-b:"));
        cache.remove("source-a:");
        assert!(!cache.contains_key("source-a:"), "removing source-a doesn't affect source-b");
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
            },
            confirmation_default: vec![],
        };
        let yaml = build_integration_yaml(&server, &HashMap::new(), &HashMap::new());
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
                env: { let mut m = HashMap::new(); m.insert("BRAVE_API_KEY".to_string(), "".to_string()); m },
                headers: HashMap::new(),
            },
            confirmation_default: vec!["*".to_string()],
        };

        let mut env = server.install_recipe.env.clone();
        env.insert("BRAVE_API_KEY".to_string(), "test-key-123".to_string());

        let yaml = build_integration_yaml(&server, &env, &HashMap::new());
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
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
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
        assert_eq!(detect_transport("npx -y @notionhq/notion-mcp-server"), "stdio");
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
        tokio::fs::write(integrations_dir.join("mcp_stdio_github.yaml"), "command: npx github\n").await.unwrap();
        tokio::fs::write(integrations_dir.join("mcp_stdio_brave_search.yaml"), "command: npx brave\n").await.unwrap();
        tokio::fs::write(integrations_dir.join("other_integration.yaml"), "some: config\n").await.unwrap();

        let index = bundled_index();
        let index_ids: std::collections::HashSet<String> = index.servers.iter().map(|s| s.id.clone()).collect();

        let mut installed_ids = Vec::new();
        let mut rd = tokio::fs::read_dir(&integrations_dir).await.unwrap();
        while let Ok(Some(entry)) = rd.next_entry().await {
            let fname = entry.file_name();
            let fname_str = fname.to_string_lossy().to_string();
            if !fname_str.ends_with(".yaml") { continue; }
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
        assert!(installed_ids.contains(&"github".to_string()), "must detect github as installed");
        assert!(installed_ids.contains(&"brave-search".to_string()), "must detect brave-search as installed");
        assert!(!installed_ids.contains(&"other".to_string()), "must not detect non-mcp integrations");
    }
}
