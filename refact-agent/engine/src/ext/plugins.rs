use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;

const PLUGIN_SIZE_LIMIT: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarketplaceOwner {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplacePluginEntry {
    pub name: String,
    #[serde(default)]
    pub source: serde_json::Value,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarketplaceJson {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub owner: Option<MarketplaceOwner>,
    #[serde(default)]
    pub plugins: Vec<MarketplacePluginEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceEntry {
    pub name: String,
    pub source: String,
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPluginEntry {
    pub name: String,
    pub marketplace: String,
    #[serde(default)]
    pub version: String,
    pub install_dir: String,
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginsDb {
    #[serde(default)]
    pub marketplaces: Vec<MarketplaceEntry>,
    #[serde(default)]
    pub installed: Vec<InstalledPluginEntry>,
}

pub fn validate_plugin_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("plugin name cannot be empty".to_string());
    }
    if name.starts_with('.') {
        return Err("plugin name cannot start with '.'".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("plugin name contains invalid path characters".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err("plugin name must match [a-zA-Z0-9._-]+".to_string());
    }
    Ok(())
}

fn validate_github_source(source: &str) -> Result<(), String> {
    let valid = source.split('/').count() == 2
        && source.split('/').all(|part| {
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

pub fn plugins_db_path(config_dir: &Path) -> PathBuf {
    config_dir.join("plugins.json")
}

pub fn marketplace_cache_dir(cache_dir: &Path, name: &str) -> PathBuf {
    cache_dir.join("marketplaces").join(name)
}

pub fn plugin_install_dir(config_dir: &Path, name: &str) -> PathBuf {
    config_dir.join("plugins").join("installed").join(name)
}

pub async fn load_plugins_db(config_dir: &Path) -> Result<PluginsDb, String> {
    let path = plugins_db_path(config_dir);
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(db) => Ok(db),
            Err(e) => {
                tracing::warn!("plugins.json is corrupt at {}: {}", path.display(), e);
                Err(format!("plugins.json is corrupt: {}", e))
            }
        },
        Err(_) => Ok(PluginsDb::default()),
    }
}

pub async fn save_plugins_db(config_dir: &Path, db: &PluginsDb) -> Result<(), String> {
    let path = plugins_db_path(config_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create dir {:?}: {}", parent, e))?;
    }
    if path.exists() {
        let bak = path.with_extension("json.bak");
        let _ = tokio::fs::copy(&path, &bak).await;
    }
    let content =
        serde_json::to_string_pretty(db).map_err(|e| format!("serialize plugins db: {}", e))?;
    let tmp_path = path.with_extension("tmp");
    tokio::fs::write(&tmp_path, &content)
        .await
        .map_err(|e| format!("write {:?}: {}", tmp_path, e))?;
    tokio::fs::rename(&tmp_path, &path)
        .await
        .map_err(|e| format!("rename {:?} -> {:?}: {}", tmp_path, path, e))?;
    Ok(())
}

pub fn parse_marketplace_json(content: &str) -> Result<MarketplaceJson, String> {
    serde_json::from_str::<MarketplaceJson>(content)
        .map_err(|e| format!("parse marketplace.json: {}", e))
}

fn marketplace_json_path(dir: &Path) -> Option<PathBuf> {
    let claude_plugin_path = dir.join(".claude-plugin").join("marketplace.json");
    if claude_plugin_path.exists() {
        return Some(claude_plugin_path);
    }
    let root_path = dir.join("marketplace.json");
    if root_path.exists() {
        return Some(root_path);
    }
    None
}

fn marketplace_json_present(dir: &Path) -> bool {
    marketplace_json_path(dir).is_some()
}

pub async fn load_marketplace_json(dir: &Path) -> Result<MarketplaceJson, String> {
    let path = marketplace_json_path(dir).unwrap_or_else(|| dir.join("marketplace.json"));
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("read {:?}: {}", path, e))?;
    parse_marketplace_json(&content)
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
            tracing::warn!("skipping symlink {:?} during plugin copy", src_path);
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
            if *size_acc > PLUGIN_SIZE_LIMIT {
                return Err("Plugin directory exceeds 50MB size limit".to_string());
            }
            tokio::fs::copy(&src_path, &dst_path)
                .await
                .map_err(|e| format!("copy {:?}: {}", src_path, e))?;
        }
    }
    Ok(())
}

fn is_local_source(source: &str) -> bool {
    source.starts_with('/')
        || source.starts_with("./")
        || source.starts_with("../")
        || std::path::Path::new(source).is_absolute()
}

async fn fetch_marketplace_to_cache(
    source: &str,
    cache_dir: &Path,
    tmp_name: &str,
) -> Result<PathBuf, String> {
    let target_dir = marketplace_cache_dir(cache_dir, tmp_name);
    if is_local_source(source) {
        return Ok(PathBuf::from(source));
    }
    let url = format!("https://github.com/{}.git", source);
    tokio::fs::create_dir_all(cache_dir.join("marketplaces"))
        .await
        .map_err(|e| format!("mkdir marketplaces: {}", e))?;
    if target_dir.exists() {
        let repo = git2::Repository::open(&target_dir)
            .map_err(|e| format!("open repo {:?}: {}", target_dir, e))?;
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
        git2::Repository::clone(&url, &target_dir).map_err(|e| format!("clone {}: {}", url, e))?;
    }
    Ok(target_dir)
}

async fn refetch_marketplace_into_cache(
    cache_dir: &Path,
    name: &str,
    source: &str,
) -> Result<(), String> {
    if is_local_source(source) {
        return Err("refusing to re-fetch a local marketplace source into the cache".to_string());
    }
    let tmp_name = format!("tmp_marketplace_{}", uuid::Uuid::new_v4().simple());
    let tmp_dir = fetch_marketplace_to_cache(source, cache_dir, &tmp_name)
        .await
        .map_err(|e| {
            let td = marketplace_cache_dir(cache_dir, &tmp_name);
            if td.exists() {
                let _ = std::fs::remove_dir_all(&td);
            }
            e
        })?;
    if !marketplace_json_present(&tmp_dir) {
        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
        return Err(format!(
            "marketplace source '{}' has no marketplace.json",
            source
        ));
    }
    let final_dir = marketplace_cache_dir(cache_dir, name);
    if final_dir.exists() {
        tokio::fs::remove_dir_all(&final_dir)
            .await
            .map_err(|e| format!("remove stale marketplace cache {:?}: {}", final_dir, e))?;
    }
    tokio::fs::rename(&tmp_dir, &final_dir).await.map_err(|e| {
        format!(
            "rename marketplace dir {:?} -> {:?}: {}",
            tmp_dir, final_dir, e
        )
    })?;
    Ok(())
}

async fn resolve_marketplace_dir(
    cache_dir: &Path,
    name: &str,
    source: &str,
) -> Result<PathBuf, String> {
    if is_local_source(source) {
        return Ok(PathBuf::from(source));
    }
    let dir = marketplace_cache_dir(cache_dir, name);
    if !marketplace_json_present(&dir) {
        refetch_marketplace_into_cache(cache_dir, name, source).await?;
    }
    Ok(dir)
}

async fn add_marketplace_impl(
    config_dir: &Path,
    cache_dir: &Path,
    source: &str,
) -> Result<MarketplaceJson, String> {
    if !is_local_source(source) {
        validate_github_source(source)?;
    }
    let tmp_name = format!("tmp_marketplace_{}", uuid::Uuid::new_v4().simple());
    let marketplace_dir = fetch_marketplace_to_cache(source, cache_dir, &tmp_name)
        .await
        .map_err(|e| {
            let tmp_dir = marketplace_cache_dir(cache_dir, &tmp_name);
            if tmp_dir.exists() {
                let _ = std::fs::remove_dir_all(&tmp_dir);
            }
            e
        })?;
    let mj = load_marketplace_json(&marketplace_dir).await.map_err(|e| {
        let tmp_dir = marketplace_cache_dir(cache_dir, &tmp_name);
        if tmp_dir.exists() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }
        format!("marketplace.json: {}", e)
    })?;
    let name = if mj.name.is_empty() {
        source.trim_matches('/').replace('/', "_")
    } else {
        mj.name.clone()
    };
    validate_plugin_name(&name)
        .map_err(|e| format!("invalid marketplace name '{}': {}", name, e))?;
    for plugin in &mj.plugins {
        validate_plugin_name(&plugin.name)
            .map_err(|e| format!("invalid plugin name '{}': {}", plugin.name, e))?;
    }
    if !is_local_source(source) {
        let final_dir = marketplace_cache_dir(cache_dir, &name);
        if final_dir != marketplace_dir {
            let tmp_dir = marketplace_cache_dir(cache_dir, &tmp_name);
            if tmp_dir.exists() {
                if final_dir.exists() {
                    tokio::fs::remove_dir_all(&final_dir)
                        .await
                        .map_err(|e| format!("remove old cache: {}", e))?;
                }
                tokio::fs::rename(&tmp_dir, &final_dir)
                    .await
                    .map_err(|e| format!("rename marketplace dir: {}", e))?;
            }
        }
    }
    let mut db = load_plugins_db(config_dir).await?;
    db.marketplaces.retain(|m| m.name != name);
    db.marketplaces.push(MarketplaceEntry {
        name: name.clone(),
        source: source.to_string(),
        added_at: Utc::now().to_rfc3339(),
    });
    save_plugins_db(config_dir, &db).await?;
    Ok(MarketplaceJson {
        name,
        ..mj
    })
}

pub async fn add_marketplace(app: AppState, source: &str) -> Result<MarketplaceJson, String> {
    let config_dir = app.paths.config_dir.clone();
    let cache_dir = app.paths.cache_dir.clone();
    add_marketplace_impl(&config_dir, &cache_dir, source).await
}

async fn ensure_default_marketplaces_with_source(
    config_dir: &Path,
    cache_dir: &Path,
    default_sources: &[&str],
) -> Result<(), String> {
    let db = load_plugins_db(config_dir).await?;
    if !db.marketplaces.is_empty() {
        return Ok(());
    }
    for source in default_sources {
        if let Err(e) = add_marketplace_impl(config_dir, cache_dir, source).await {
            tracing::warn!("Failed to seed default marketplace '{}': {}", source, e);
        }
    }
    Ok(())
}

pub async fn ensure_default_marketplaces(app: AppState) -> Result<(), String> {
    let config_dir = app.paths.config_dir.clone();
    let cache_dir = app.paths.cache_dir.clone();
    ensure_default_marketplaces_with_source(
        &config_dir,
        &cache_dir,
        &[
            "anthropics/claude-code",
            "anthropics/claude-plugins-official",
            "anthropics/skills",
            "anthropics/knowledge-work-plugins",
            "microsoft/skills",
            "ComposioHQ/awesome-claude-skills",
            "wshobson/agents",
            "VoltAgent/awesome-claude-code-subagents",
        ],
    )
    .await
}

pub async fn remove_marketplace(app: AppState, name: &str) -> Result<(), String> {
    validate_plugin_name(name)?;
    let config_dir = app.paths.config_dir.clone();
    let mut db = load_plugins_db(&config_dir).await?;
    db.marketplaces.retain(|m| m.name != name);
    save_plugins_db(&config_dir, &db).await
}

pub async fn list_marketplace_plugins(
    app: AppState,
    name: &str,
) -> Result<Vec<MarketplacePluginEntry>, String> {
    validate_plugin_name(name)?;
    let config_dir = app.paths.config_dir.clone();
    let cache_dir = app.paths.cache_dir.clone();
    let db = load_plugins_db(&config_dir).await?;
    let entry = db
        .marketplaces
        .iter()
        .find(|m| m.name == name)
        .ok_or_else(|| format!("marketplace '{}' not found", name))?;
    let marketplace_dir = resolve_marketplace_dir(&cache_dir, name, &entry.source).await?;
    let mj = load_marketplace_json(&marketplace_dir).await?;
    Ok(mj.plugins)
}

pub async fn install_plugin(
    app: AppState,
    plugin_name: &str,
    marketplace_name: &str,
) -> Result<InstalledPluginEntry, String> {
    validate_plugin_name(plugin_name)?;
    validate_plugin_name(marketplace_name)?;
    let config_dir = app.paths.config_dir.clone();
    let cache_dir = app.paths.cache_dir.clone();
    let db = load_plugins_db(&config_dir).await?;
    let market_entry = db
        .marketplaces
        .iter()
        .find(|m| m.name == marketplace_name)
        .ok_or_else(|| format!("marketplace '{}' not found", marketplace_name))?;
    let marketplace_source = market_entry.source.clone();
    let marketplace_dir =
        resolve_marketplace_dir(&cache_dir, marketplace_name, &marketplace_source).await?;
    let mj = load_marketplace_json(&marketplace_dir).await?;
    let plugin_entry = mj
        .plugins
        .iter()
        .find(|p| p.name == plugin_name)
        .ok_or_else(|| {
            format!(
                "plugin '{}' not found in marketplace '{}'",
                plugin_name, marketplace_name
            )
        })?;
    let plugin_source_dir = resolve_plugin_source_dir(&marketplace_dir, &plugin_entry.source)?;
    let install_dir = plugin_install_dir(&config_dir, plugin_name);
    if install_dir.exists() {
        return Err(format!("plugin '{}' is already installed", plugin_name));
    }
    let temp_dir = install_dir.with_extension("installing");
    if temp_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
    let mut size_acc = 0u64;
    if let Err(e) = copy_dir_recursive(&plugin_source_dir, &temp_dir, &mut size_acc).await {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err(e);
    }
    if let Err(e) = tokio::fs::rename(&temp_dir, &install_dir).await {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err(format!("rename install dir: {}", e));
    }
    let version = plugin_entry.version.clone();
    let entry = InstalledPluginEntry {
        name: plugin_name.to_string(),
        marketplace: marketplace_name.to_string(),
        version,
        install_dir: install_dir.to_string_lossy().to_string(),
        installed_at: Utc::now().to_rfc3339(),
    };
    let mut db = load_plugins_db(&config_dir).await?;
    db.installed.retain(|i| i.name != plugin_name);
    db.installed.push(entry.clone());
    save_plugins_db(&config_dir, &db).await?;
    app.integrations
        .ext_cache_generation
        .fetch_add(1, Ordering::Relaxed);
    Ok(entry)
}

fn resolve_plugin_source_dir(
    marketplace_dir: &Path,
    source: &serde_json::Value,
) -> Result<PathBuf, String> {
    match source {
        serde_json::Value::String(s) => {
            let relative = s.trim_start_matches("./");
            let path = Path::new(relative);
            for component in path.components() {
                match component {
                    std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_) => {
                        return Err(format!("unsafe plugin source path: {}", s));
                    }
                    _ => {}
                }
            }
            if path.is_absolute() {
                return Err(format!("unsafe plugin source path: {}", s));
            }
            let joined = marketplace_dir.join(relative);
            match std::fs::symlink_metadata(&joined) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(format!(
                        "plugin source is a symlink (not allowed): {:?}",
                        joined
                    ));
                }
                Err(e) => {
                    return Err(format!("cannot stat plugin source {:?}: {}", joined, e));
                }
                _ => {}
            }
            let marketplace_canon = std::fs::canonicalize(marketplace_dir)
                .map(|path| dunce::simplified(&path).to_path_buf())
                .map_err(|e| format!("canonicalize marketplace {:?}: {}", marketplace_dir, e))?;
            let joined_canon = std::fs::canonicalize(&joined)
                .map(|path| dunce::simplified(&path).to_path_buf())
                .map_err(|e| format!("canonicalize plugin source {:?}: {}", joined, e))?;
            if !joined_canon.starts_with(&marketplace_canon) {
                return Err(format!(
                    "plugin source escapes marketplace directory: {:?}",
                    joined_canon
                ));
            }
            Ok(joined)
        }
        serde_json::Value::Object(obj) => {
            let kind = obj.get("source").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "github" {
                let repo = obj
                    .get("repo")
                    .and_then(|v| v.as_str())
                    .ok_or("missing repo field")?;
                Err(format!("github plugin source not yet supported: {}", repo))
            } else {
                Err(format!("unsupported plugin source type: {}", kind))
            }
        }
        _ => Err("invalid plugin source field".to_string()),
    }
}

pub async fn uninstall_plugin(app: AppState, plugin_name: &str) -> Result<(), String> {
    validate_plugin_name(plugin_name)?;
    let config_dir = app.paths.config_dir.clone();
    let mut db = load_plugins_db(&config_dir).await?;
    let was_installed = db.installed.iter().any(|i| i.name == plugin_name);
    db.installed.retain(|i| i.name != plugin_name);
    save_plugins_db(&config_dir, &db).await?;
    let install_dir = plugin_install_dir(&config_dir, plugin_name);
    if install_dir.exists() {
        tokio::fs::remove_dir_all(&install_dir)
            .await
            .map_err(|e| format!("remove install dir {:?}: {}", install_dir, e))?;
    }
    if was_installed {
        app.integrations
            .ext_cache_generation
            .fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::config_dirs::CommandSource;

    #[test]
    fn test_validate_plugin_name_accepts_valid() {
        assert!(validate_plugin_name("my-plugin").is_ok());
        assert!(validate_plugin_name("plugin_v2").is_ok());
        assert!(validate_plugin_name("plugin.1.0").is_ok());
        assert!(validate_plugin_name("abc").is_ok());
        assert!(validate_plugin_name("Plugin-123").is_ok());
    }

    #[test]
    fn test_validate_plugin_name_rejects_traversal() {
        assert!(validate_plugin_name("../../etc").is_err());
        assert!(validate_plugin_name("/absolute").is_err());
        assert!(validate_plugin_name("a/b").is_err());
        assert!(validate_plugin_name("..").is_err());
        assert!(validate_plugin_name(".hidden").is_err());
        assert!(validate_plugin_name("").is_err());
        assert!(validate_plugin_name("path\\traversal").is_err());
        assert!(validate_plugin_name("name with spaces").is_err());
    }

    #[tokio::test]
    async fn test_copy_dir_recursive_skips_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        tokio::fs::create_dir_all(&src).await.unwrap();

        tokio::fs::write(src.join("regular.txt"), "content")
            .await
            .unwrap();

        #[cfg(unix)]
        {
            let target = tmp.path().join("target_file.txt");
            tokio::fs::write(&target, "secret").await.unwrap();
            std::os::unix::fs::symlink(&target, src.join("symlink.txt")).unwrap();

            let mut size_acc = 0u64;
            copy_dir_recursive(&src, &dst, &mut size_acc).await.unwrap();

            assert!(dst.join("regular.txt").exists());
            assert!(
                !dst.join("symlink.txt").exists(),
                "symlink should not be copied"
            );
        }
        #[cfg(not(unix))]
        {
            let mut size_acc = 0u64;
            copy_dir_recursive(&src, &dst, &mut size_acc).await.unwrap();
            assert!(dst.join("regular.txt").exists());
        }
    }

    #[tokio::test]
    async fn test_ext_dirs_includes_installed_plugin_dirs() {
        use crate::ext::config_dirs::ExtDirs;
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_path_buf();
        let install_root = config_dir.join("plugins").join("installed");
        tokio::fs::create_dir_all(&install_root).await.unwrap();
        tokio::fs::create_dir_all(install_root.join("plugin-x"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(install_root.join("plugin-y"))
            .await
            .unwrap();

        let mut installed_dirs = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(&install_root).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.is_dir() {
                    installed_dirs.push(path);
                }
            }
        }
        installed_dirs.sort();

        let ext_dirs = ExtDirs {
            global_dirs: vec![config_dir.clone()],
            installed_dirs: installed_dirs.clone(),
            project_dirs: vec![],
        };

        assert_eq!(ext_dirs.installed_dirs.len(), 2);
        let all = ext_dirs.all_dirs_in_order();
        assert!(all.contains(&&install_root.join("plugin-x")));
        assert!(all.contains(&&install_root.join("plugin-y")));

        let global_dirs = vec![config_dir.clone()];
        let src = crate::ext::config_dirs::source_for_dir(
            &install_root.join("plugin-x"),
            &global_dirs,
            &installed_dirs,
        );
        assert!(
            matches!(src, crate::ext::config_dirs::CommandSource::InstalledPlugin(n) if n == "plugin-x")
        );
    }

    #[tokio::test]
    async fn test_plugins_db_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().to_path_buf();
        let db = PluginsDb {
            marketplaces: vec![MarketplaceEntry {
                name: "market".to_string(),
                source: "/some/path".to_string(),
                added_at: "2024-01-01".to_string(),
            }],
            installed: vec![InstalledPluginEntry {
                name: "plugin".to_string(),
                marketplace: "market".to_string(),
                version: "1.0.0".to_string(),
                install_dir: "/installed/plugin".to_string(),
                installed_at: "2024-01-02".to_string(),
            }],
        };
        save_plugins_db(&config_dir, &db).await.unwrap();
        let loaded = load_plugins_db(&config_dir).await.unwrap();
        assert_eq!(loaded.marketplaces.len(), 1);
        assert_eq!(loaded.marketplaces[0].name, "market");
        assert_eq!(loaded.installed.len(), 1);
        assert_eq!(loaded.installed[0].name, "plugin");
    }

    #[tokio::test]
    async fn test_load_plugins_db_missing_file() {
        let result = load_plugins_db(Path::new("/nonexistent/path")).await;
        assert!(result.is_ok());
        let db = result.unwrap();
        assert!(db.marketplaces.is_empty());
        assert!(db.installed.is_empty());
    }

    #[tokio::test]
    async fn test_ensure_default_marketplaces_partial_failure_continues() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let cache_dir = tmp.path().join("cache");
        tokio::fs::create_dir_all(&config_dir).await.unwrap();
        tokio::fs::create_dir_all(&cache_dir).await.unwrap();

        let good_market = tmp.path().join("good-market");
        tokio::fs::create_dir_all(&good_market).await.unwrap();
        tokio::fs::write(
            good_market.join("marketplace.json"),
            r#"{"name": "good-market", "plugins": []}"#,
        )
        .await
        .unwrap();

        let good_source = good_market.to_string_lossy().to_string();
        let result = ensure_default_marketplaces_with_source(
            &config_dir,
            &cache_dir,
            &["/nonexistent/bad-market", good_source.as_str()],
        )
        .await;
        assert!(result.is_ok(), "partial failure should be non-fatal");

        let db = load_plugins_db(&config_dir).await.unwrap();
        assert_eq!(
            db.marketplaces.len(),
            1,
            "good marketplace should be seeded despite earlier failure"
        );
        assert_eq!(db.marketplaces[0].name, "good-market");
    }

    #[tokio::test]
    async fn test_add_marketplace_twice_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let cache_dir = tmp.path().join("cache");
        tokio::fs::create_dir_all(&config_dir).await.unwrap();

        let marketplace_dir = tmp.path().join("marketplace");
        tokio::fs::create_dir_all(&marketplace_dir).await.unwrap();
        tokio::fs::write(
            marketplace_dir.join("marketplace.json"),
            r#"{"name": "test-market", "plugins": []}"#,
        )
        .await
        .unwrap();

        let source = marketplace_dir.to_string_lossy().to_string();

        let result1 = add_marketplace_impl(&config_dir, &cache_dir, &source).await;
        assert!(result1.is_ok(), "first add failed: {:?}", result1);

        let result2 = add_marketplace_impl(&config_dir, &cache_dir, &source).await;
        assert!(result2.is_ok(), "second add failed: {:?}", result2);

        let db = load_plugins_db(&config_dir).await.unwrap();
        assert_eq!(
            db.marketplaces.len(),
            1,
            "should have exactly one entry after two adds"
        );
        assert_eq!(db.marketplaces[0].name, "test-market");
        assert_eq!(db.marketplaces[0].source, source);
    }

    #[tokio::test]
    async fn test_load_plugins_db_corrupt_json_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("plugins.json"), "NOT VALID JSON {{{{")
            .await
            .unwrap();
        let result = load_plugins_db(tmp.path()).await;
        assert!(
            result.is_err(),
            "corrupt JSON must return error, not empty default"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("corrupt"),
            "error should mention 'corrupt': {}",
            msg
        );
    }

    #[test]
    fn test_resolve_plugin_source_rejects_escape_via_components() {
        let tmp = tempfile::tempdir().unwrap();
        let result =
            resolve_plugin_source_dir(tmp.path(), &serde_json::json!("../../../etc/passwd"));
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("unsafe"),
            "error should mention 'unsafe': {}",
            msg
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_resolve_plugin_source_rejects_symlink_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let marketplace_dir = tmp.path().join("marketplace");
        tokio::fs::create_dir_all(&marketplace_dir).await.unwrap();
        let real_dir = tmp.path().join("real-target");
        tokio::fs::create_dir_all(&real_dir).await.unwrap();
        let symlink_path = marketplace_dir.join("symlinked-plugin");
        std::os::unix::fs::symlink(&real_dir, &symlink_path).unwrap();

        let result =
            resolve_plugin_source_dir(&marketplace_dir, &serde_json::json!("./symlinked-plugin"));
        assert!(result.is_err(), "symlink source must be rejected");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("symlink"),
            "error should mention 'symlink': {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_resolve_plugin_source_accepts_regular_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let marketplace_dir = tmp.path().join("marketplace");
        let plugin_dir = marketplace_dir.join("plugins").join("my-plugin");
        tokio::fs::create_dir_all(&plugin_dir).await.unwrap();

        let result =
            resolve_plugin_source_dir(&marketplace_dir, &serde_json::json!("./plugins/my-plugin"));
        assert!(result.is_ok(), "regular dir should succeed: {:?}", result);
    }
}
