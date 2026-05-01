use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use url::Url;

use crate::ext::plugins::{load_marketplace_json, marketplace_cache_dir};
use crate::ext::config_dirs::collect_md_files_recursive;
use crate::ext::slash_commands::parse_frontmatter_and_body;
use crate::ext::yaml_util::{yaml_str, yaml_str_list};
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::http::routers::v1::at_commands::invalidate_slash_cache;

const MARKETPLACE_SIZE_LIMIT: u64 = 50 * 1024 * 1024;
const SOURCES_FILENAME: &str = "extensions_marketplace_sources.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketplaceKind {
    Skill,
    Command,
    Subagent,
}

impl MarketplaceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MarketplaceKind::Skill => "skill",
            MarketplaceKind::Command => "command",
            MarketplaceKind::Subagent => "subagent",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketplaceSourceKind {
    BuiltinEmbedded,
    BuiltinGithub,
    UserGithub,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketplaceParserMode {
    Manifest,
    Scan,
    Overlay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionsMarketplaceSource {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub enabled: bool,
    pub builtin: bool,
    pub removable: bool,
    pub source_kind: MarketplaceSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    #[serde(default)]
    pub supported_kinds: Vec<MarketplaceKind>,
    pub parser_mode: MarketplaceParserMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionsMarketplaceSourcesDb {
    #[serde(default)]
    pub sources: Vec<ExtensionsMarketplaceSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionsMarketplaceManifest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub owner: Option<ExtensionsMarketplaceOwner>,
    #[serde(default)]
    pub skills: Vec<MarketplaceManifestItem>,
    #[serde(default)]
    pub commands: Vec<MarketplaceManifestItem>,
    #[serde(default)]
    pub subagents: Vec<MarketplaceManifestItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionsMarketplaceOwner {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarketplaceItemParam {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarketplaceManifestItem {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub path: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub params: Vec<MarketplaceItemParam>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub publisher: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub kind: MarketplaceKind,
    pub source_id: String,
    pub source_label: String,
    pub path: String,
    #[serde(default)]
    pub installed_scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_preview: Option<String>,
    #[serde(default)]
    pub params: Vec<MarketplaceItemParam>,
}

#[derive(Debug, Clone)]
pub struct ResolvedMarketplaceItem {
    pub item: MarketplaceItem,
    pub abs_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListedMarketplaceSource {
    pub id: String,
    pub label: String,
    pub description: String,
    pub enabled: bool,
    pub builtin: bool,
    pub removable: bool,
    pub source_kind: MarketplaceSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    #[serde(default)]
    pub supported_kinds: Vec<MarketplaceKind>,
    pub parser_mode: MarketplaceParserMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub item_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallMarketplaceItemRequest {
    pub source_id: String,
    pub item_id: String,
    pub scope: String,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub params: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallMarketplaceItemResponse {
    pub installed: bool,
    pub scope: String,
    pub file_path: String,
    pub item_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveMarketplaceSourceRequest {
    pub url: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigureMarketplaceSourceRequest {
    #[serde(default)]
    pub enabled: Option<bool>,
}

pub fn extensions_marketplace_sources_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SOURCES_FILENAME)
}

#[derive(Debug, Deserialize)]
struct ExtensionsMarketplaceSourcesIndex {
    sources: Vec<ExtensionsMarketplaceSource>,
}

fn builtin_sources() -> Vec<ExtensionsMarketplaceSource> {
    serde_json::from_str::<ExtensionsMarketplaceSourcesIndex>(include_str!(
        "../yaml_configs/extensions_marketplace_sources.json"
    ))
    .expect("bundled extensions_marketplace_sources.json must be valid JSON")
    .sources
}

pub async fn load_sources_db(config_dir: &Path) -> Result<ExtensionsMarketplaceSourcesDb, String> {
    let path = extensions_marketplace_sources_path(config_dir);
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => serde_json::from_str::<ExtensionsMarketplaceSourcesDb>(&content)
            .map_err(|e| format!("extensions marketplace sources are corrupt: {}", e)),
        Err(_) => Ok(ExtensionsMarketplaceSourcesDb::default()),
    }
}

pub async fn save_sources_db(
    config_dir: &Path,
    db: &ExtensionsMarketplaceSourcesDb,
) -> Result<(), String> {
    let path = extensions_marketplace_sources_path(config_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create dir {:?}: {}", parent, e))?;
    }
    let content = serde_json::to_string_pretty(db)
        .map_err(|e| format!("serialize marketplace sources: {}", e))?;
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, &content)
        .await
        .map_err(|e| format!("write {:?}: {}", tmp, e))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("rename {:?} -> {:?}: {}", tmp, path, e))?;
    Ok(())
}

pub async fn load_all_sources(
    config_dir: &Path,
) -> Result<Vec<ExtensionsMarketplaceSource>, String> {
    let db = load_sources_db(config_dir).await?;
    let mut map: HashMap<String, ExtensionsMarketplaceSource> = HashMap::new();
    for src in builtin_sources() {
        map.insert(src.id.clone(), src);
    }
    for src in db.sources {
        map.insert(src.id.clone(), src);
    }
    let mut out: Vec<ExtensionsMarketplaceSource> = map.into_values().collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(out)
}

pub async fn save_user_source(
    config_dir: &Path,
    src: ExtensionsMarketplaceSource,
) -> Result<ExtensionsMarketplaceSource, String> {
    let mut db = load_sources_db(config_dir).await?;
    db.sources.retain(|s| s.id != src.id);
    db.sources.push(src.clone());
    save_sources_db(config_dir, &db).await?;
    Ok(src)
}

pub async fn configure_source(
    config_dir: &Path,
    id: &str,
    enabled: Option<bool>,
) -> Result<(), String> {
    let mut db = load_sources_db(config_dir).await?;
    if let Some(src) = db.sources.iter_mut().find(|s| s.id == id) {
        if let Some(v) = enabled {
            src.enabled = v;
        }
        return save_sources_db(config_dir, &db).await;
    }
    let builtins = builtin_sources();
    let Some(mut builtin) = builtins.into_iter().find(|s| s.id == id) else {
        return Err(format!("source '{}' not found", id));
    };
    if let Some(v) = enabled {
        builtin.enabled = v;
    }
    db.sources.retain(|s| s.id != id);
    db.sources.push(builtin);
    save_sources_db(config_dir, &db).await
}

pub async fn delete_user_source(config_dir: &Path, id: &str) -> Result<(), String> {
    if builtin_sources().iter().any(|s| s.id == id) {
        return Err("cannot delete builtin source".to_string());
    }
    let mut db = load_sources_db(config_dir).await?;
    let before = db.sources.len();
    db.sources.retain(|s| s.id != id);
    if db.sources.len() == before {
        return Err(format!("source '{}' not found", id));
    }
    save_sources_db(config_dir, &db).await
}

pub fn normalize_github_source(input: &str) -> Result<(String, String), String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("GitHub URL is empty".to_string());
    }
    let owner_repo = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let url = Url::parse(trimmed).map_err(|e| format!("invalid GitHub URL: {}", e))?;
        if url.host_str() != Some("github.com") {
            return Err("only github.com repo URLs are supported".to_string());
        }
        let segs: Vec<String> = url
            .path_segments()
            .map(|s| s.filter(|v| !v.is_empty()).map(|v| v.to_string()).collect())
            .unwrap_or_default();
        if segs.len() != 2 {
            return Err(
                "GitHub URL must point to a repo root like https://github.com/owner/repo"
                    .to_string(),
            );
        }
        format!("{}/{}", segs[0], segs[1].trim_end_matches(".git"))
    } else {
        trimmed.trim_end_matches(".git").to_string()
    };
    validate_github_repo(&owner_repo)?;
    Ok((
        owner_repo.clone(),
        format!("https://github.com/{}", owner_repo),
    ))
}

pub fn validate_github_repo(source: &str) -> Result<(), String> {
    let parts: Vec<&str> = source.split('/').collect();
    let valid = parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
        });
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid GitHub source format '{}': must be 'owner/repo'",
            source
        ))
    }
}

pub fn source_id_from_repo(owner_repo: &str) -> String {
    owner_repo
        .to_lowercase()
        .replace(['/', '.', '_'], "-")
        .replace("--", "-")
}

fn is_local_source(source: &str) -> bool {
    source.starts_with('/')
        || source.starts_with("./")
        || source.starts_with("../")
        || Path::new(source).is_absolute()
}

pub fn source_cache_dir(cache_dir: &Path, id: &str) -> PathBuf {
    marketplace_cache_dir(cache_dir, id)
}

fn make_body_preview(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(300).collect())
    }
}

fn yaml_params(doc: &serde_yaml::Value) -> Vec<MarketplaceItemParam> {
    let arr = match doc.get("params").and_then(|v| v.as_sequence()) {
        Some(v) => v,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|p| {
            let name = p.get("name").and_then(|v| v.as_str())?.to_string();
            let label = p
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(&name)
                .to_string();
            let default = p
                .get("default")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let required = p.get("required").and_then(|v| v.as_bool()).unwrap_or(false);
            Some(MarketplaceItemParam {
                name,
                label,
                default,
                required,
            })
        })
        .collect()
}

fn substitute_params(content: &str, params: &HashMap<String, String>) -> String {
    let mut result = content.to_string();
    for (key, value) in params {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

async fn substitute_params_in_dir(
    dir: &Path,
    params: &HashMap<String, String>,
) -> Result<(), String> {
    if params.is_empty() {
        return Ok(());
    }
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| format!("readdir {:?}: {}", dir, e))?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            Box::pin(substitute_params_in_dir(&path, params)).await?;
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext == "md" || ext == "yaml" || ext == "yml" {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                let substituted = substitute_params(&content, params);
                if substituted != content {
                    tokio::fs::write(&path, &substituted)
                        .await
                        .map_err(|e| format!("write {:?}: {}", path, e))?;
                }
            }
        }
    }
    Ok(())
}

pub async fn refresh_source_cache(
    config_dir: &Path,
    cache_dir: &Path,
    id: &str,
) -> Result<(), String> {
    let sources = load_all_sources(config_dir).await?;
    let source = sources
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("source '{}' not found", id))?;
    match source.source_kind {
        MarketplaceSourceKind::BuiltinEmbedded => {
            Err("embedded sources cannot be refreshed".to_string())
        }
        MarketplaceSourceKind::BuiltinGithub | MarketplaceSourceKind::UserGithub => {
            if source
                .repo_url
                .as_deref()
                .map(is_local_source)
                .unwrap_or(false)
            {
                return Ok(());
            }
            fetch_source_to_cache(&source, cache_dir, id).await?;
            Ok(())
        }
    }
}

async fn fetch_source_to_cache(
    source: &ExtensionsMarketplaceSource,
    cache_dir: &Path,
    tmp_name: &str,
) -> Result<PathBuf, String> {
    match source.source_kind {
        MarketplaceSourceKind::BuiltinEmbedded => Ok(embedded_source_dir(source.id.as_str())?),
        MarketplaceSourceKind::BuiltinGithub | MarketplaceSourceKind::UserGithub => {
            let repo_url = source.repo_url.as_deref().ok_or("missing repo_url")?;
            let (owner_repo, _) = normalize_github_source(repo_url)?;
            let target = source_cache_dir(cache_dir, tmp_name);
            let url = format!("https://github.com/{}.git", owner_repo);
            tokio::fs::create_dir_all(cache_dir.join("marketplaces"))
                .await
                .map_err(|e| format!("mkdir marketplaces: {}", e))?;
            if target.exists() {
                let repo = git2::Repository::open(&target)
                    .map_err(|e| format!("open repo {:?}: {}", target, e))?;
                let mut remote = repo
                    .find_remote("origin")
                    .map_err(|e| format!("find remote: {}", e))?;
                remote
                    .fetch(
                        &["HEAD:refs/heads/main", "HEAD:refs/heads/master"],
                        None,
                        None,
                    )
                    .or_else(|_| remote.fetch(&["HEAD"], None, None))
                    .map_err(|e| format!("fetch: {}", e))?;
                drop(remote);
                let fetch_head = repo
                    .find_reference("FETCH_HEAD")
                    .or_else(|_| repo.find_reference("refs/heads/main"))
                    .or_else(|_| repo.find_reference("refs/heads/master"))
                    .map_err(|e| format!("resolve ref: {}", e))?;
                let target_obj = fetch_head
                    .peel_to_commit()
                    .map_err(|e| format!("peel: {}", e))?;
                repo.reset(target_obj.as_object(), git2::ResetType::Hard, None)
                    .map_err(|e| format!("reset: {}", e))?;
            } else {
                git2::Repository::clone(&url, &target)
                    .map_err(|e| format!("clone {}: {}", url, e))?;
            }
            Ok(target)
        }
    }
}

async fn ensure_source_ready(
    cache_dir: &Path,
    source: &ExtensionsMarketplaceSource,
) -> Result<PathBuf, String> {
    match source.source_kind {
        MarketplaceSourceKind::BuiltinEmbedded => embedded_source_dir(source.id.as_str()),
        MarketplaceSourceKind::BuiltinGithub | MarketplaceSourceKind::UserGithub => {
            let Some(repo_url) = source.repo_url.as_deref() else {
                return Err("missing repo_url".to_string());
            };
            if is_local_source(repo_url) {
                return Ok(PathBuf::from(repo_url));
            }
            let final_dir = source_cache_dir(cache_dir, &source.id);
            if final_dir.exists() {
                return Ok(final_dir);
            }
            let tmp_name = format!("tmp_marketplace_{}", uuid::Uuid::new_v4().simple());
            let tmp_dir = fetch_source_to_cache(source, cache_dir, &tmp_name).await?;
            if matches!(
                source.source_kind,
                MarketplaceSourceKind::BuiltinGithub | MarketplaceSourceKind::UserGithub
            ) {
                let from = source_cache_dir(cache_dir, &tmp_name);
                if from.exists() && from != final_dir {
                    if final_dir.exists() {
                        tokio::fs::remove_dir_all(&final_dir)
                            .await
                            .map_err(|e| format!("remove old cache: {}", e))?;
                    }
                    tokio::fs::rename(&from, &final_dir)
                        .await
                        .map_err(|e| format!("rename marketplace dir: {}", e))?;
                }
                Ok(final_dir)
            } else {
                Ok(tmp_dir)
            }
        }
    }
}

fn embedded_source_dir(id: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("yaml_configs")
        .join("marketplace");
    let dir = match id {
        "refact-starter-skills" => root.join("skills"),
        "refact-starter-commands" => root.join("commands"),
        "refact-starter-subagents" => root.join("subagents"),
        _ => return Err(format!("unknown embedded source '{}'", id)),
    };
    Ok(dir)
}

fn parse_manifest(content: &str) -> Result<ExtensionsMarketplaceManifest, String> {
    serde_json::from_str::<ExtensionsMarketplaceManifest>(content)
        .map_err(|e| format!("parse marketplace manifest: {}", e))
}

async fn load_manifest(dir: &Path) -> Result<ExtensionsMarketplaceManifest, String> {
    let explicit = dir.join(".refact-marketplace").join("marketplace.json");
    if explicit.exists() {
        let content = tokio::fs::read_to_string(&explicit)
            .await
            .map_err(|e| format!("read {:?}: {}", explicit, e))?;
        return parse_manifest(&content);
    }
    let root = dir.join("marketplace.json");
    if root.exists() {
        let content = tokio::fs::read_to_string(&root)
            .await
            .map_err(|e| format!("read {:?}: {}", root, e))?;
        if let Ok(manifest) = parse_manifest(&content) {
            return Ok(manifest);
        }
        let plugin_market = load_marketplace_json(dir).await?;
        return Ok(ExtensionsMarketplaceManifest {
            name: plugin_market.name,
            owner: plugin_market
                .owner
                .map(|o| ExtensionsMarketplaceOwner { name: o.name }),
            ..Default::default()
        });
    }
    Err("marketplace.json not found".to_string())
}

fn relative_path_is_safe(path: &Path) -> Result<(), String> {
    for component in path.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("unsafe marketplace path: {}", path.display()));
            }
            _ => {}
        }
    }
    if path.is_absolute() {
        return Err(format!("unsafe marketplace path: {}", path.display()));
    }
    Ok(())
}

fn resolve_repo_path(repo_dir: &Path, relative: &str) -> Result<PathBuf, String> {
    let trimmed = relative.trim_start_matches("./");
    let rel = Path::new(trimmed);
    relative_path_is_safe(rel)?;
    let joined = repo_dir.join(rel);
    match std::fs::symlink_metadata(&joined) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!(
                "marketplace source is a symlink (not allowed): {:?}",
                joined
            ));
        }
        Err(e) => {
            return Err(format!(
                "cannot stat marketplace source {:?}: {}",
                joined, e
            ))
        }
        _ => {}
    }
    let repo_canon = std::fs::canonicalize(repo_dir)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|e| format!("canonicalize repo {:?}: {}", repo_dir, e))?;
    let joined_canon = std::fs::canonicalize(&joined)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|e| format!("canonicalize marketplace source {:?}: {}", joined, e))?;
    if !joined_canon.starts_with(&repo_canon) {
        return Err(format!(
            "marketplace source escapes repo directory: {:?}",
            joined_canon
        ));
    }
    Ok(joined)
}

async fn copy_dir_recursive(src: &Path, dst: &Path, size_acc: &mut u64) -> Result<(), String> {
    tokio::fs::create_dir_all(dst)
        .await
        .map_err(|e| format!("mkdir {:?}: {}", dst, e))?;
    let mut entries = tokio::fs::read_dir(src)
        .await
        .map_err(|e| format!("readdir {:?}: {}", src, e))?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = match entry.file_type().await {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            tracing::warn!("skipping symlink {:?} during marketplace copy", src_path);
            continue;
        }
        if file_type.is_dir() {
            Box::pin(copy_dir_recursive(&src_path, &dst_path, size_acc)).await?;
        } else if file_type.is_file() {
            let file_len = tokio::fs::metadata(&src_path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            *size_acc += file_len;
            if *size_acc > MARKETPLACE_SIZE_LIMIT {
                return Err("Marketplace item exceeds 50MB size limit".to_string());
            }
            tokio::fs::copy(&src_path, &dst_path)
                .await
                .map_err(|e| format!("copy {:?}: {}", src_path, e))?;
        }
    }
    Ok(())
}

async fn scan_skill_items(
    repo_dir: &Path,
    source: &ExtensionsMarketplaceSource,
) -> Result<Vec<MarketplaceItem>, String> {
    let mut roots = vec![repo_dir.to_path_buf()];
    for rel in ["skills", ".claude/skills"] {
        let dir = repo_dir.join(rel);
        if dir.exists() {
            roots.push(dir);
        }
    }
    roots.extend(
        plugin_source_dirs(repo_dir)
            .await
            .into_iter()
            .map(|dir| dir.join("skills")),
    );
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    // Support single-skill repos that place SKILL.md directly at the repo root.
    let root_skill_md = repo_dir.join("SKILL.md");
    if root_skill_md.exists() && seen.insert(String::new()) {
        if let Ok(content) = tokio::fs::read_to_string(&root_skill_md).await {
            let (fm, body) = parse_frontmatter_and_body(&content);
            let name = yaml_str(&fm, "name");
            if !name.is_empty() {
                out.push(MarketplaceItem {
                    id: name.clone(),
                    name,
                    description: yaml_str(&fm, "description"),
                    tags: yaml_str_list(&fm, "tags"),
                    publisher: source.label.clone(),
                    homepage: source.repo_url.clone(),
                    kind: MarketplaceKind::Skill,
                    source_id: source.id.clone(),
                    source_label: source.label.clone(),
                    path: String::new(),
                    installed_scopes: Vec::new(),
                    body_preview: make_body_preview(&body),
                    params: yaml_params(&fm),
                });
            }
        }
    }
    for root in roots {
        let mut entries = match tokio::fs::read_dir(&root).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }
            let rel = path
                .strip_prefix(repo_dir)
                .map_err(|e| format!("strip prefix: {}", e))?
                .to_string_lossy()
                .to_string();
            if !seen.insert(rel.clone()) {
                continue;
            }
            let content = match tokio::fs::read_to_string(&skill_md).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let (fm, body) = parse_frontmatter_and_body(&content);
            let name = yaml_str(&fm, "name");
            if name.is_empty() {
                continue;
            }
            out.push(MarketplaceItem {
                id: name.clone(),
                name,
                description: yaml_str(&fm, "description"),
                tags: yaml_str_list(&fm, "tags"),
                publisher: source.label.clone(),
                homepage: source.repo_url.clone(),
                kind: MarketplaceKind::Skill,
                source_id: source.id.clone(),
                source_label: source.label.clone(),
                path: rel,
                installed_scopes: Vec::new(),
                body_preview: make_body_preview(&body),
                params: yaml_params(&fm),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

async fn scan_command_items(
    repo_dir: &Path,
    source: &ExtensionsMarketplaceSource,
) -> Result<Vec<MarketplaceItem>, String> {
    let mut dirs = Vec::new();
    for sub in ["commands", ".refact/commands", ".claude/commands"] {
        let dir = repo_dir.join(sub);
        if dir.exists() {
            dirs.push(dir);
        }
    }
    dirs.extend(
        plugin_source_dirs(repo_dir)
            .await
            .into_iter()
            .map(|dir| dir.join("commands")),
    );
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(mut entries) = tokio::fs::read_dir(repo_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let content = match tokio::fs::read_to_string(&path).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let (fm, body) = parse_frontmatter_and_body(&content);
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if stem.is_empty() {
                continue;
            }
            let rel = path
                .strip_prefix(repo_dir)
                .map_err(|e| format!("strip prefix: {}", e))?
                .to_string_lossy()
                .to_string();
            if !seen.insert(rel.clone()) {
                continue;
            }
            out.push(MarketplaceItem {
                id: stem.clone(),
                name: stem,
                description: yaml_str(&fm, "description"),
                tags: yaml_str_list(&fm, "tags"),
                publisher: source.label.clone(),
                homepage: source.repo_url.clone(),
                kind: MarketplaceKind::Command,
                source_id: source.id.clone(),
                source_label: source.label.clone(),
                path: rel,
                installed_scopes: Vec::new(),
                body_preview: make_body_preview(&body),
                params: yaml_params(&fm),
            });
        }
    }
    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        let files = collect_md_files_recursive(&dir).await;
        for path in files {
            let content = match tokio::fs::read_to_string(&path).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let (fm, body) = parse_frontmatter_and_body(&content);
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if stem.is_empty() {
                continue;
            }
            let rel = path
                .strip_prefix(repo_dir)
                .map_err(|e| format!("strip prefix: {}", e))?
                .to_string_lossy()
                .to_string();
            if !seen.insert(rel.clone()) {
                continue;
            }
            out.push(MarketplaceItem {
                id: stem.clone(),
                name: stem,
                description: yaml_str(&fm, "description"),
                tags: yaml_str_list(&fm, "tags"),
                publisher: source.label.clone(),
                homepage: source.repo_url.clone(),
                kind: MarketplaceKind::Command,
                source_id: source.id.clone(),
                source_label: source.label.clone(),
                path: rel,
                installed_scopes: Vec::new(),
                body_preview: make_body_preview(&body),
                params: yaml_params(&fm),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

async fn scan_subagent_items(
    repo_dir: &Path,
    source: &ExtensionsMarketplaceSource,
) -> Result<Vec<MarketplaceItem>, String> {
    let mut out = Vec::new();
    let mut seen_paths = HashSet::new();

    for sub in ["subagents", ".refact/subagents"] {
        let dir = repo_dir.join(sub);
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let content = match tokio::fs::read_to_string(&path).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let cfg = match serde_yaml::from_str::<
                crate::yaml_configs::customization_types::SubagentConfig,
            >(&content)
            {
                Ok(v) => v,
                Err(_) => continue,
            };
            if cfg.id.is_empty() {
                continue;
            }
            let rel = path
                .strip_prefix(repo_dir)
                .map_err(|e| format!("strip prefix: {}", e))?
                .to_string_lossy()
                .to_string();
            if !seen_paths.insert(rel.clone()) {
                continue;
            }
            out.push(MarketplaceItem {
                id: cfg.id.clone(),
                name: if cfg.title.is_empty() {
                    cfg.id.clone()
                } else {
                    cfg.title
                },
                description: cfg.description,
                tags: Vec::new(),
                publisher: source.label.clone(),
                homepage: source.repo_url.clone(),
                kind: MarketplaceKind::Subagent,
                source_id: source.id.clone(),
                source_label: source.label.clone(),
                path: rel,
                installed_scopes: Vec::new(),
                body_preview: None,
                params: Vec::new(),
            });
        }
    }

    let plugin_dirs = plugin_source_dirs(repo_dir).await;
    let mut markdown_roots = vec![
        repo_dir.join("agents"),
        repo_dir.join(".claude").join("agents"),
    ];
    for dir in &plugin_dirs {
        markdown_roots.push(dir.clone());
        markdown_roots.push(dir.join("agents"));
        markdown_roots.push(dir.join(".claude").join("agents"));
    }
    for dir in markdown_roots {
        if !dir.exists() {
            continue;
        }
        let files = collect_md_files_recursive(&dir).await;
        for path in files {
            let rel = path
                .strip_prefix(repo_dir)
                .map_err(|e| format!("strip prefix: {}", e))?
                .to_string_lossy()
                .to_string();
            if !seen_paths.insert(rel.clone()) {
                continue;
            }
            let content = match tokio::fs::read_to_string(&path).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let (fm, body) = parse_frontmatter_and_body(&content);
            let name = yaml_str(&fm, "name");
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let id = if name.is_empty() {
                stem.clone()
            } else {
                sanitize_subagent_id(&name)
            };
            if id.is_empty() {
                continue;
            }
            let title = if name.is_empty() { stem.clone() } else { name };
            out.push(MarketplaceItem {
                id,
                name: title,
                description: yaml_str(&fm, "description"),
                tags: yaml_str_list(&fm, "tags"),
                publisher: source.label.clone(),
                homepage: source.repo_url.clone(),
                kind: MarketplaceKind::Subagent,
                source_id: source.id.clone(),
                source_label: source.label.clone(),
                path: rel,
                installed_scopes: Vec::new(),
                body_preview: make_body_preview(&body),
                params: yaml_params(&fm),
            });
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

fn sanitize_subagent_id(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in input.chars() {
        let mapped = if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-' {
            Some(ch)
        } else if ch.is_ascii_uppercase() {
            Some(ch.to_ascii_lowercase())
        } else if ch.is_ascii_whitespace() || ch == '/' || ch == '.' {
            Some('-')
        } else {
            None
        };
        let Some(ch) = mapped else {
            continue;
        };
        if ch == '-' {
            if out.is_empty() || prev_dash {
                continue;
            }
            prev_dash = true;
            out.push(ch);
            continue;
        }
        prev_dash = false;
        out.push(ch);
    }
    out.trim_matches('-').to_string()
}

fn map_external_tools_to_refact(input: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for tool in input {
        let mapped = match tool.to_ascii_lowercase().as_str() {
            "read" => Some("cat"),
            "glob" => Some("search_pattern"),
            "grep" => Some("search_pattern"),
            "bash" => Some("shell"),
            "write" | "edit" | "multiedit" => Some("apply_patch"),
            "todowrite" => Some("tasks_set"),
            _ => None,
        };
        if let Some(name) = mapped {
            if seen.insert(name.to_string()) {
                out.push(name.to_string());
            }
        }
    }
    out
}

async fn plugin_source_dirs(repo_dir: &Path) -> Vec<PathBuf> {
    let market = match load_marketplace_json(repo_dir).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for plugin in market.plugins {
        let Some(src) = plugin.source.as_str() else {
            continue;
        };
        let Ok(path) = resolve_repo_path(repo_dir, src) else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

fn convert_markdown_subagent_to_yaml(item_id: &str, content: &str) -> Result<String, String> {
    let (fm, body) = parse_frontmatter_and_body(content);
    let title = yaml_str(&fm, "name");
    let description = yaml_str(&fm, "description");
    let tools = map_external_tools_to_refact(yaml_str_list(&fm, "tools"));
    let expose_as_tool = !tools.is_empty();
    let mut yaml = String::new();
    yaml.push_str("schema_version: 1\n");
    yaml.push_str(&format!("id: {}\n", item_id));
    yaml.push_str(&format!(
        "title: {}\n",
        serde_json::to_string(&if title.is_empty() {
            item_id.to_string()
        } else {
            title
        })
        .map_err(|e| e.to_string())?
    ));
    yaml.push_str(&format!(
        "description: {}\n",
        serde_json::to_string(&description).map_err(|e| e.to_string())?
    ));
    yaml.push_str("specific: false\n");
    yaml.push_str(&format!(
        "expose_as_tool: {}\n",
        if expose_as_tool { "true" } else { "false" }
    ));
    yaml.push_str("has_code: false\n");
    if expose_as_tool {
        yaml.push_str("tool:\n");
        yaml.push_str(&format!(
            "  description: {}\n",
            serde_json::to_string(&if description.is_empty() {
                format!("Run {}", item_id)
            } else {
                description.clone()
            })
            .map_err(|e| e.to_string())?
        ));
        yaml.push_str("  agentic: true\n");
        yaml.push_str("  allow_parallel: true\n");
        yaml.push_str("  parameters:\n");
        yaml.push_str("    - name: task\n");
        yaml.push_str("      type: string\n");
        yaml.push_str("      description: Task description for the subagent\n");
        yaml.push_str("  required:\n");
        yaml.push_str("    - task\n");
    }
    yaml.push_str("subchat:\n");
    yaml.push_str("  context_mode: bare\n");
    yaml.push_str("  stateful: false\n");
    yaml.push_str("  model_type: default\n");
    yaml.push_str("  max_steps: 10\n");
    yaml.push_str("messages:\n");
    yaml.push_str(&format!(
        "  system_prompt: |\n{}",
        indent_multiline(body.trim(), 4)
    ));
    yaml.push('\n');
    if expose_as_tool {
        yaml.push_str("  user_template: |\n");
        yaml.push_str("    {{task}}\n");
    }
    if !tools.is_empty() {
        yaml.push_str("tools:\n");
        for tool in tools {
            yaml.push_str(&format!("  - {}\n", tool));
        }
    }
    Ok(yaml)
}

fn indent_multiline(input: &str, spaces: usize) -> String {
    let indent = " ".repeat(spaces);
    if input.is_empty() {
        return format!("{}", indent);
    }
    input
        .lines()
        .map(|line| format!("{}{}", indent, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn manifest_items_to_marketplace_items(
    source: &ExtensionsMarketplaceSource,
    kind: MarketplaceKind,
    entries: Vec<MarketplaceManifestItem>,
) -> Vec<MarketplaceItem> {
    entries
        .into_iter()
        .map(|entry| MarketplaceItem {
            id: entry.id.clone(),
            name: if entry.name.is_empty() {
                entry.id.clone()
            } else {
                entry.name
            },
            description: entry.description,
            tags: entry.tags,
            publisher: if entry.publisher.is_empty() {
                source.label.clone()
            } else {
                entry.publisher
            },
            homepage: entry.homepage.or_else(|| source.repo_url.clone()),
            kind,
            source_id: source.id.clone(),
            source_label: source.label.clone(),
            path: entry.path,
            installed_scopes: Vec::new(),
            body_preview: None,
            params: entry.params,
        })
        .collect()
}

async fn load_items_for_source(
    repo_dir: &Path,
    source: &ExtensionsMarketplaceSource,
    kind: MarketplaceKind,
) -> Result<Vec<MarketplaceItem>, String> {
    match source.parser_mode {
        MarketplaceParserMode::Manifest => match load_manifest(repo_dir).await {
            Ok(manifest) => Ok(match kind {
                MarketplaceKind::Skill => {
                    let items = manifest_items_to_marketplace_items(source, kind, manifest.skills);
                    if items.is_empty() {
                        scan_skill_items(repo_dir, source).await?
                    } else {
                        items
                    }
                }
                MarketplaceKind::Command => {
                    let items =
                        manifest_items_to_marketplace_items(source, kind, manifest.commands);
                    if items.is_empty() {
                        scan_command_items(repo_dir, source).await?
                    } else {
                        items
                    }
                }
                MarketplaceKind::Subagent => {
                    let items =
                        manifest_items_to_marketplace_items(source, kind, manifest.subagents);
                    if items.is_empty() {
                        scan_subagent_items(repo_dir, source).await?
                    } else {
                        items
                    }
                }
            }),
            Err(err) => match kind {
                MarketplaceKind::Skill => scan_skill_items(repo_dir, source).await.or(Err(err)),
                MarketplaceKind::Command => scan_command_items(repo_dir, source).await.or(Err(err)),
                MarketplaceKind::Subagent => {
                    scan_subagent_items(repo_dir, source).await.or(Err(err))
                }
            },
        },
        MarketplaceParserMode::Scan | MarketplaceParserMode::Overlay => match kind {
            MarketplaceKind::Skill => scan_skill_items(repo_dir, source).await,
            MarketplaceKind::Command => scan_command_items(repo_dir, source).await,
            MarketplaceKind::Subagent => scan_subagent_items(repo_dir, source).await,
        },
    }
}

fn source_to_listed(
    source: &ExtensionsMarketplaceSource,
    item_count: u32,
    error: Option<String>,
) -> ListedMarketplaceSource {
    ListedMarketplaceSource {
        id: source.id.clone(),
        label: source.label.clone(),
        description: source.description.clone(),
        enabled: source.enabled,
        builtin: source.builtin,
        removable: source.removable,
        source_kind: source.source_kind.clone(),
        repo_url: source.repo_url.clone(),
        supported_kinds: source.supported_kinds.clone(),
        parser_mode: source.parser_mode.clone(),
        last_sync_at: source.last_sync_at.clone(),
        error,
        item_count,
    }
}

pub async fn list_marketplace_items(
    gcx: Arc<ARwLock<GlobalContext>>,
    kind: MarketplaceKind,
) -> Result<(Vec<MarketplaceItem>, Vec<ListedMarketplaceSource>), String> {
    let (config_dir, cache_dir) = {
        let g = gcx.read().await;
        (g.config_dir.clone(), g.cache_dir.clone())
    };
    let sources = load_all_sources(&config_dir).await?;
    let installed = installed_scopes_by_kind(gcx.clone(), kind).await?;

    let relevant: Vec<ExtensionsMarketplaceSource> = sources
        .into_iter()
        .filter(|s| s.supported_kinds.contains(&kind))
        .collect();

    // Load all enabled sources concurrently instead of sequentially.
    let load_results =
        futures::future::join_all(relevant.iter().filter(|s| s.enabled).map(|source| {
            let cache_dir = cache_dir.clone();
            let source = source.clone();
            async move {
                let result = async {
                    let repo_dir = ensure_source_ready(&cache_dir, &source).await?;
                    load_items_for_source(&repo_dir, &source, kind).await
                }
                .await;
                (source, result)
            }
        }))
        .await;

    let mut items: Vec<MarketplaceItem> = Vec::new();
    let mut listed_sources: Vec<ListedMarketplaceSource> = Vec::new();

    for source in relevant.iter().filter(|s| !s.enabled) {
        listed_sources.push(source_to_listed(source, 0, source.error.clone()));
    }

    for (source, result) in load_results {
        match result {
            Ok(source_items) => {
                let count = source_items.len() as u32;
                listed_sources.push(source_to_listed(&source, count, None));
                items.extend(source_items);
            }
            Err(e) => {
                listed_sources.push(source_to_listed(&source, 0, Some(e)));
            }
        }
    }

    for item in &mut items {
        if let Some(scopes) = installed.get(&item.id) {
            item.installed_scopes = scopes.clone();
        }
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((items, listed_sources))
}

pub async fn installed_scopes_by_kind(
    gcx: Arc<ARwLock<GlobalContext>>,
    kind: MarketplaceKind,
) -> Result<HashMap<String, Vec<String>>, String> {
    use crate::ext::config_dirs::get_ext_dirs;
    let ext_dirs = get_ext_dirs(gcx.clone()).await;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    match kind {
        MarketplaceKind::Skill => {
            for item in crate::ext::skills::load_skill_indices(&ext_dirs).await {
                let scope = match item.source {
                    crate::ext::config_dirs::CommandSource::GlobalClaude
                    | crate::ext::config_dirs::CommandSource::GlobalRefact => "global",
                    crate::ext::config_dirs::CommandSource::ProjectClaude(_)
                    | crate::ext::config_dirs::CommandSource::ProjectRefact(_) => "local",
                    crate::ext::config_dirs::CommandSource::InstalledPlugin(_) => "plugin",
                };
                map.entry(item.name).or_default().push(scope.to_string());
            }
        }
        MarketplaceKind::Command => {
            for item in crate::ext::slash_commands::load_slash_commands(&ext_dirs).await {
                let scope = match item.source {
                    crate::ext::config_dirs::CommandSource::GlobalClaude
                    | crate::ext::config_dirs::CommandSource::GlobalRefact => "global",
                    crate::ext::config_dirs::CommandSource::ProjectClaude(_)
                    | crate::ext::config_dirs::CommandSource::ProjectRefact(_) => "local",
                    crate::ext::config_dirs::CommandSource::InstalledPlugin(_) => "plugin",
                };
                map.entry(item.name).or_default().push(scope.to_string());
            }
        }
        MarketplaceKind::Subagent => {
            let config_dir = gcx.read().await.config_dir.clone();
            let locals = get_project_dirs(gcx.clone()).await;
            if let Some(registry) =
                crate::yaml_configs::customization_registry::get_project_registry(gcx.clone()).await
            {
                for (id, _cfg) in registry.subagents {
                    let global_path = config_dir.join("subagents").join(format!("{}.yaml", id));
                    if global_path.exists() {
                        map.entry(id.clone())
                            .or_default()
                            .push("global".to_string());
                    }
                    if locals.iter().any(|root| {
                        root.join(".refact")
                            .join("subagents")
                            .join(format!("{}.yaml", id))
                            .exists()
                    }) {
                        map.entry(id).or_default().push("local".to_string());
                    }
                }
            }
        }
    }
    for scopes in map.values_mut() {
        scopes.sort();
        scopes.dedup();
    }
    Ok(map)
}

pub async fn resolve_marketplace_item(
    gcx: Arc<ARwLock<GlobalContext>>,
    kind: MarketplaceKind,
    source_id: &str,
    item_id: &str,
) -> Result<ResolvedMarketplaceItem, String> {
    let (config_dir, cache_dir) = {
        let g = gcx.read().await;
        (g.config_dir.clone(), g.cache_dir.clone())
    };
    let sources = load_all_sources(&config_dir).await?;
    let source = sources
        .into_iter()
        .find(|s| s.id == source_id)
        .ok_or_else(|| format!("source '{}' not found", source_id))?;
    let repo_dir = ensure_source_ready(&cache_dir, &source).await?;
    let items = load_items_for_source(&repo_dir, &source, kind).await?;
    let item = items.into_iter().find(|i| i.id == item_id).ok_or_else(|| {
        format!(
            "{} '{}' not found in source '{}'",
            kind.as_str(),
            item_id,
            source_id
        )
    })?;
    let abs_path = resolve_repo_path(&repo_dir, &item.path)?;
    Ok(ResolvedMarketplaceItem { item, abs_path })
}

async fn resolve_scope_dir(
    gcx: Arc<ARwLock<GlobalContext>>,
    scope: &str,
) -> Result<(PathBuf, String), String> {
    let config_dir = gcx.read().await.config_dir.clone();
    let project_root = get_project_dirs(gcx.clone()).await.into_iter().next();
    match scope {
        "global" => Ok((config_dir, "global".to_string())),
        "local" => match project_root {
            Some(root) => Ok((root.join(".refact"), "local".to_string())),
            None => Err("no project root for local scope".to_string()),
        },
        other => Err(format!(
            "invalid scope: '{}'; expected 'global' or 'local'",
            other
        )),
    }
}

pub async fn install_marketplace_item(
    gcx: Arc<ARwLock<GlobalContext>>,
    kind: MarketplaceKind,
    req: InstallMarketplaceItemRequest,
) -> Result<InstallMarketplaceItemResponse, String> {
    let ResolvedMarketplaceItem { item, abs_path } =
        resolve_marketplace_item(gcx.clone(), kind, &req.source_id, &req.item_id).await?;
    let (base_dir, scope_name) = resolve_scope_dir(gcx.clone(), &req.scope).await?;

    let (target, file_path) = match kind {
        MarketplaceKind::Skill => {
            let dir = base_dir.join("skills").join(&item.name);
            let file = dir.join("SKILL.md");
            (dir, file)
        }
        MarketplaceKind::Command => {
            let file = base_dir.join("commands").join(format!("{}.md", item.name));
            (file.clone(), file)
        }
        MarketplaceKind::Subagent => {
            let file = base_dir.join("subagents").join(format!("{}.yaml", item.id));
            (file.clone(), file)
        }
    };

    // Validate params against the declared schema before any destructive action.
    {
        let declared: HashMap<&str, &MarketplaceItemParam> =
            item.params.iter().map(|p| (p.name.as_str(), p)).collect();
        for key in req.params.keys() {
            if !declared.contains_key(key.as_str()) {
                return Err(format!(
                    "unknown parameter '{}'; declared params: [{}]",
                    key,
                    item.params
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        for param in &item.params {
            if param.required {
                let val = req
                    .params
                    .get(&param.name)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                if val.is_empty() {
                    return Err(format!(
                        "required parameter '{}' is missing or empty",
                        param.name
                    ));
                }
            }
        }
    }

    // Conflict check without any deletion yet.
    if target.exists() && !req.overwrite {
        return Err(format!("destination already exists: {}", target.display()));
    }

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir {:?}: {}", parent, e))?;
    }

    match kind {
        MarketplaceKind::Skill => {
            // Stage into a temp dir first so the old install is only removed after
            // the new one is fully prepared.
            let temp = target.with_extension("installing");
            if temp.exists() {
                let _ = tokio::fs::remove_dir_all(&temp).await;
            }
            let mut size = 0u64;
            if item.path.is_empty() {
                // Root-level SKILL.md repo: create the skill dir and copy just the one file.
                tokio::fs::create_dir_all(&temp)
                    .await
                    .map_err(|e| format!("mkdir {:?}: {}", temp, e))?;
                size = tokio::fs::copy(abs_path.join("SKILL.md"), temp.join("SKILL.md"))
                    .await
                    .map_err(|e| format!("copy SKILL.md from root: {}", e))?;
            } else {
                copy_dir_recursive(&abs_path, &temp, &mut size).await?;
            }
            if size > MARKETPLACE_SIZE_LIMIT {
                return Err("Marketplace item exceeds 50MB size limit".to_string());
            }
            // Apply substitutions to the staged dir, not the live target.
            substitute_params_in_dir(&temp, &req.params).await?;
            // Everything staged: now atomically swap.
            if target.exists() {
                tokio::fs::remove_dir_all(&target)
                    .await
                    .map_err(|e| format!("remove existing target {:?}: {}", target, e))?;
            }
            tokio::fs::rename(&temp, &target)
                .await
                .map_err(|e| format!("rename install dir: {}", e))?;
        }
        MarketplaceKind::Command => {
            let temp = target.with_extension("installing");
            let meta = tokio::fs::metadata(&abs_path)
                .await
                .map_err(|e| format!("metadata {:?}: {}", abs_path, e))?;
            if meta.len() > MARKETPLACE_SIZE_LIMIT {
                return Err("Marketplace item exceeds 50MB size limit".to_string());
            }
            tokio::fs::copy(&abs_path, &temp)
                .await
                .map_err(|e| format!("copy {:?}: {}", abs_path, e))?;
            // Apply substitutions to temp, not target.
            if !req.params.is_empty() {
                if let Ok(content) = tokio::fs::read_to_string(&temp).await {
                    let substituted = substitute_params(&content, &req.params);
                    if substituted != content {
                        tokio::fs::write(&temp, &substituted)
                            .await
                            .map_err(|e| format!("write {:?}: {}", temp, e))?;
                    }
                }
            }
            // rename() replaces the target atomically on Unix/Windows.
            tokio::fs::rename(&temp, &target)
                .await
                .map_err(|e| format!("rename install file: {}", e))?;
        }
        MarketplaceKind::Subagent => {
            let temp = target.with_extension("installing");
            let content = tokio::fs::read_to_string(&abs_path)
                .await
                .map_err(|e| format!("read {:?}: {}", abs_path, e))?;
            let converted = if abs_path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                content
            } else {
                convert_markdown_subagent_to_yaml(&item.id, &content)?
            };
            let final_content = if req.params.is_empty() {
                converted
            } else {
                let substituted = substitute_params(&converted, &req.params);
                // Validate that param substitution did not corrupt the YAML.
                serde_yaml::from_str::<serde_yaml::Value>(&substituted)
                    .map_err(|e| format!("parameter substitution produced invalid YAML: {}", e))?;
                substituted
            };
            let byte_len = final_content.as_bytes().len() as u64;
            if byte_len > MARKETPLACE_SIZE_LIMIT {
                return Err("Marketplace item exceeds 50MB size limit".to_string());
            }
            tokio::fs::write(&temp, final_content)
                .await
                .map_err(|e| format!("write {:?}: {}", temp, e))?;
            tokio::fs::rename(&temp, &target)
                .await
                .map_err(|e| format!("rename install file: {}", e))?;
            crate::yaml_configs::customization_registry::invalidate_all_registry_caches(
                gcx.clone(),
            )
            .await;
        }
    }

    gcx.read()
        .await
        .ext_cache_generation
        .fetch_add(1, Ordering::Relaxed);
    invalidate_slash_cache().await;

    Ok(InstallMarketplaceItemResponse {
        installed: true,
        scope: scope_name,
        file_path: file_path.display().to_string(),
        item_id: item.id,
        name: item.name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_github_source_full_url() {
        let (owner_repo, url) =
            normalize_github_source("https://github.com/anthropics/skills").unwrap();
        assert_eq!(owner_repo, "anthropics/skills");
        assert_eq!(url, "https://github.com/anthropics/skills");
    }

    #[test]
    fn test_normalize_github_source_short() {
        let (owner_repo, url) = normalize_github_source("wshobson/agents").unwrap();
        assert_eq!(owner_repo, "wshobson/agents");
        assert_eq!(url, "https://github.com/wshobson/agents");
    }

    #[test]
    fn test_normalize_github_source_rejects_non_root() {
        let result = normalize_github_source("https://github.com/anthropics/skills/tree/main");
        assert!(result.is_err());
    }

    #[test]
    fn test_source_id_from_repo() {
        assert_eq!(
            source_id_from_repo("anthropics/skills"),
            "anthropics-skills"
        );
    }

    #[test]
    fn test_relative_path_is_safe() {
        assert!(relative_path_is_safe(Path::new("skills/foo")).is_ok());
        assert!(relative_path_is_safe(Path::new("../skills/foo")).is_err());
        assert!(relative_path_is_safe(Path::new("/etc/passwd")).is_err());
    }

    #[tokio::test]
    async fn test_scan_skill_items() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("reviewer");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: reviewer\ndescription: Review code\ntags:\n  - code\n---\nBody",
        )
        .await
        .unwrap();
        let src = ExtensionsMarketplaceSource {
            id: "x".to_string(),
            label: "X".to_string(),
            description: String::new(),
            enabled: true,
            builtin: false,
            removable: true,
            source_kind: MarketplaceSourceKind::UserGithub,
            repo_url: Some("https://github.com/x/y".to_string()),
            supported_kinds: vec![MarketplaceKind::Skill],
            parser_mode: MarketplaceParserMode::Scan,
            last_sync_at: None,
            error: None,
        };
        let items = scan_skill_items(tmp.path(), &src).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "reviewer");
        assert_eq!(items[0].description, "Review code");
    }

    #[tokio::test]
    async fn test_scan_command_items() {
        let tmp = tempfile::tempdir().unwrap();
        let cmd_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&cmd_dir).await.unwrap();
        tokio::fs::write(
            cmd_dir.join("review.md"),
            "---\ndescription: Review\ntags:\n  - code\n---\nRun review",
        )
        .await
        .unwrap();
        let src = ExtensionsMarketplaceSource {
            id: "x".to_string(),
            label: "X".to_string(),
            description: String::new(),
            enabled: true,
            builtin: false,
            removable: true,
            source_kind: MarketplaceSourceKind::UserGithub,
            repo_url: Some("https://github.com/x/y".to_string()),
            supported_kinds: vec![MarketplaceKind::Command],
            parser_mode: MarketplaceParserMode::Scan,
            last_sync_at: None,
            error: None,
        };
        let items = scan_command_items(tmp.path(), &src).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "review");
        assert_eq!(items[0].description, "Review");
    }

    #[test]
    fn test_resolve_repo_path_rejects_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let err = resolve_repo_path(tmp.path(), "../etc/passwd").unwrap_err();
        assert!(err.contains("unsafe"));
    }
}
