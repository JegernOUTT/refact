use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::RwLock as ARwLock;

use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_types::*;
use crate::yaml_configs::project_configs_bootstrap::project_configs_try_create_all;

#[derive(Debug, Clone)]
pub struct RegistryCache {
    pub project_root: PathBuf,
    pub registry: ProjectRegistry,
    pub last_scan: SystemTime,
}

pub struct RegistryCacheManager {
    cache: HashMap<PathBuf, RegistryCache>,
}

impl RegistryCacheManager {
    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }

    pub fn get(&self, project_root: &Path) -> Option<&RegistryCache> {
        self.cache.get(project_root)
    }

    pub fn insert(&mut self, project_root: PathBuf, registry: ProjectRegistry) {
        self.cache.insert(project_root.clone(), RegistryCache {
            project_root,
            registry,
            last_scan: SystemTime::now(),
        });
    }

    pub fn remove(&mut self, project_root: &Path) {
        self.cache.remove(project_root);
    }
}

pub async fn load_project_registry(project_root: &Path) -> ProjectRegistry {
    let mut registry = ProjectRegistry::default();
    let refact_dir = project_root.join(".refact");

    load_modes(&refact_dir.join("modes"), &mut registry).await;
    load_subagents(&refact_dir.join("subagents"), &mut registry).await;
    load_toolbox_commands(&refact_dir.join("toolbox_commands"), &mut registry).await;
    load_code_lens(&refact_dir.join("code_lens"), &mut registry).await;

    registry
}

async fn load_modes(dir: &Path, registry: &mut ProjectRegistry) {
    let paths = collect_yaml_paths(dir).await;

    for path in paths {
        match load_yaml_file::<ModeConfig>(&path).await {
            Ok(config) => {
                if config.base.is_some() && config.match_models.is_some() {
                    registry.mode_overrides.push(config);
                } else if registry.modes.contains_key(&config.id) {
                    registry.errors.push(RegistryError {
                        file_path: path.display().to_string(),
                        error: format!("duplicate mode id '{}'", config.id),
                    });
                } else {
                    registry.modes.insert(config.id.clone(), config);
                }
            }
            Err(e) => {
                registry.errors.push(RegistryError {
                    file_path: path.display().to_string(),
                    error: e,
                });
            }
        }
    }
}

async fn load_subagents(dir: &Path, registry: &mut ProjectRegistry) {
    let paths = collect_yaml_paths(dir).await;

    for path in paths {
        match load_yaml_file::<SubagentConfig>(&path).await {
            Ok(config) => {
                if config.base.is_some() && config.match_models.is_some() {
                    registry.subagent_overrides.push(config);
                } else if registry.subagents.contains_key(&config.id) {
                    registry.errors.push(RegistryError {
                        file_path: path.display().to_string(),
                        error: format!("duplicate subagent id '{}'", config.id),
                    });
                } else {
                    registry.subagents.insert(config.id.clone(), config);
                }
            }
            Err(e) => {
                registry.errors.push(RegistryError {
                    file_path: path.display().to_string(),
                    error: e,
                });
            }
        }
    }
}

async fn load_toolbox_commands(dir: &Path, registry: &mut ProjectRegistry) {
    let paths = collect_yaml_paths(dir).await;

    for path in paths {
        match load_yaml_file::<ToolboxCommandConfig>(&path).await {
            Ok(config) => {
                if registry.toolbox_commands.contains_key(&config.id) {
                    registry.errors.push(RegistryError {
                        file_path: path.display().to_string(),
                        error: format!("duplicate toolbox_command id '{}'", config.id),
                    });
                } else {
                    registry.toolbox_commands.insert(config.id.clone(), config);
                }
            }
            Err(e) => {
                registry.errors.push(RegistryError {
                    file_path: path.display().to_string(),
                    error: e,
                });
            }
        }
    }
}

async fn load_code_lens(dir: &Path, registry: &mut ProjectRegistry) {
    let paths = collect_yaml_paths(dir).await;

    for path in paths {
        match load_yaml_file::<CodeLensConfig>(&path).await {
            Ok(config) => {
                if registry.code_lens.contains_key(&config.id) {
                    registry.errors.push(RegistryError {
                        file_path: path.display().to_string(),
                        error: format!("duplicate code_lens id '{}'", config.id),
                    });
                } else {
                    registry.code_lens.insert(config.id.clone(), config);
                }
            }
            Err(e) => {
                registry.errors.push(RegistryError {
                    file_path: path.display().to_string(),
                    error: e,
                });
            }
        }
    }
}

async fn collect_yaml_paths(dir: &Path) -> Vec<PathBuf> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    let mut paths = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            paths.push(path);
        }
    }
    paths.sort();
    paths
}

async fn load_yaml_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;
    serde_yaml::from_str(&content)
        .map_err(|e| format!("Failed to parse YAML: {}", e))
}

pub fn resolve_mode_for_model(
    registry: &ProjectRegistry,
    mode_id: &str,
    model_id: Option<&str>,
) -> Option<ModeConfig> {
    let base = registry.modes.get(mode_id)?;

    let model_id = match model_id {
        Some(m) => m,
        None => return Some(base.clone()),
    };

    let matching_override = registry.mode_overrides.iter()
        .filter(|o| o.base.as_deref() == Some(mode_id))
        .find(|o| {
            o.match_models.as_ref()
                .map(|patterns| patterns.iter().any(|p| model_matches_pattern(model_id, p)))
                .unwrap_or(false)
        });

    match matching_override {
        Some(override_config) => {
            if let Some(ref ov) = override_config.override_config {
                Some(base.apply_override(ov))
            } else {
                Some(base.clone())
            }
        }
        None => Some(base.clone()),
    }
}

pub fn resolve_subagent_for_model(
    registry: &ProjectRegistry,
    subagent_id: &str,
    model_id: Option<&str>,
) -> Option<SubagentConfig> {
    let base = registry.subagents.get(subagent_id)?;

    let model_id = match model_id {
        Some(m) => m,
        None => return Some(base.clone()),
    };

    let matching_override = registry.subagent_overrides.iter()
        .filter(|o| o.base.as_deref() == Some(subagent_id))
        .find(|o| {
            o.match_models.as_ref()
                .map(|patterns| patterns.iter().any(|p| model_matches_pattern(model_id, p)))
                .unwrap_or(false)
        });

    match matching_override {
        Some(override_config) => Some(base.apply_override(override_config)),
        None => Some(base.clone()),
    }
}

fn model_matches_pattern(model_id: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.ends_with("*") {
        let prefix = &pattern[..pattern.len() - 1];
        return model_id.starts_with(prefix);
    }
    model_id == pattern
}

pub fn match_tool_confirm_action(rules: &[ToolConfirmRule], tool_name: &str) -> Option<String> {
    for rule in rules {
        if glob_matches(&rule.match_pattern, tool_name) {
            return Some(rule.action.clone());
        }
    }
    None
}

fn glob_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.ends_with("*") {
        let prefix = &pattern[..pattern.len() - 1];
        return name.starts_with(prefix);
    }
    if pattern.starts_with("*") {
        let suffix = &pattern[1..];
        return name.ends_with(suffix);
    }
    pattern == name
}

pub async fn get_project_registry(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Option<ProjectRegistry> {
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first()?.clone();

    {
        let gcx_locked = gcx.read().await;
        let cache_result = gcx_locked.project_registry_cache.read();
        if let Ok(cache) = cache_result {
            if let Some(cached) = cache.get(&project_root) {
                return Some(cached.registry.clone());
            }
        }
    }

    let _ = project_configs_try_create_all(&project_root).await;
    let registry = load_project_registry(&project_root).await;

    {
        let gcx_locked = gcx.read().await;
        let cache_result = gcx_locked.project_registry_cache.write();
        if let Ok(mut cache) = cache_result {
            cache.insert(project_root, registry.clone());
        }
    }

    Some(registry)
}

pub async fn get_mode_config(
    gcx: Arc<ARwLock<GlobalContext>>,
    mode_id: &str,
    model_id: Option<&str>,
) -> Option<ModeConfig> {
    let registry = get_project_registry(gcx).await?;
    resolve_mode_for_model(&registry, mode_id, model_id)
}

pub async fn get_subagent_config(
    gcx: Arc<ARwLock<GlobalContext>>,
    subagent_id: &str,
    model_id: Option<&str>,
) -> Option<SubagentConfig> {
    let registry = get_project_registry(gcx).await?;
    resolve_subagent_for_model(&registry, subagent_id, model_id)
}

pub fn map_legacy_mode_to_id(mode_str: &str) -> &str {
    match mode_str.to_uppercase().as_str() {
        "NO_TOOLS" => "explore",
        "EXPLORE" => "explore",
        "AGENT" => "agent",
        "CONFIGURE" => "configurator",
        "PROJECT_SUMMARY" => "project_summary",
        "TASK_PLANNER" => "task_planner",
        "TASK_AGENT" => "task_agent",
        _ => {
            if mode_str.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
                mode_str
            } else {
                "agent"
            }
        }
    }
}
