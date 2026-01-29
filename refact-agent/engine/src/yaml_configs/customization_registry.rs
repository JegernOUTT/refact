use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::yaml_configs::customization_types::*;

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
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            continue;
        }

        match load_yaml_file::<ModeConfig>(&path) {
            Ok(config) => {
                if config.base.is_some() && config.match_models.is_some() {
                    registry.mode_overrides.push(config);
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
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            continue;
        }

        match load_yaml_file::<SubagentConfig>(&path) {
            Ok(config) => {
                if config.base.is_some() && config.match_models.is_some() {
                    registry.subagent_overrides.push(config);
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
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            continue;
        }

        match load_yaml_file::<ToolboxCommandConfig>(&path) {
            Ok(config) => {
                registry.toolbox_commands.insert(config.id.clone(), config);
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
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            continue;
        }

        match load_yaml_file::<CodeLensConfig>(&path) {
            Ok(config) => {
                registry.code_lens.insert(config.id.clone(), config);
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

fn load_yaml_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let content = std::fs::read_to_string(path)
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
        Some(override_config) => Some(override_config.clone()),
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
