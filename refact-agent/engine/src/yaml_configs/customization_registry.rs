use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::RwLock as ARwLock;

use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_types::*;
use crate::yaml_configs::project_configs_bootstrap::{global_configs_try_create_all, project_configs_ensure_dirs, compute_checksum, get_default_checksum};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    Global,
    Local,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RegistryCache {
    pub project_root: PathBuf,
    pub registry: ProjectRegistry,
    pub last_scan: SystemTime,
}

pub struct RegistryCacheManager {
    pub cache: HashMap<PathBuf, RegistryCache>,
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

    #[allow(dead_code)]
    pub fn remove(&mut self, project_root: &Path) {
        self.cache.remove(project_root);
    }
}

pub async fn load_registry_from_dir(dir: &Path) -> ProjectRegistry {
    let mut registry = ProjectRegistry::default();

    load_modes(&dir.join("modes"), &mut registry, false).await;
    load_subagents(&dir.join("subagents"), &mut registry, false).await;
    load_toolbox_commands(&dir.join("toolbox_commands"), &mut registry).await;
    load_code_lens(&dir.join("code_lens"), &mut registry).await;

    registry
}

pub async fn load_merged_registry(global_dir: &Path, local_dir: Option<&Path>) -> ProjectRegistry {
    let global_registry = load_registry_from_dir(global_dir).await;

    let local_registry = match local_dir {
        Some(dir) => {
            let refact_dir = dir.join(".refact");
            load_local_registry_skip_defaults(&refact_dir).await
        }
        None => ProjectRegistry::default(),
    };

    merge_registries(global_registry, local_registry)
}

async fn load_local_registry_skip_defaults(refact_dir: &Path) -> ProjectRegistry {
    let mut registry = ProjectRegistry::default();

    load_modes_skip_defaults(&refact_dir.join("modes"), &mut registry).await;
    load_subagents_skip_defaults(&refact_dir.join("subagents"), &mut registry).await;
    load_toolbox_commands_skip_defaults(&refact_dir.join("toolbox_commands"), &mut registry).await;
    load_code_lens_skip_defaults(&refact_dir.join("code_lens"), &mut registry).await;

    registry
}

fn merge_registries(global: ProjectRegistry, local: ProjectRegistry) -> ProjectRegistry {
    let mut merged = global;

    for (id, config) in local.modes {
        merged.modes.insert(id, config);
    }

    let mut all_overrides = local.mode_overrides;
    all_overrides.extend(merged.mode_overrides);
    merged.mode_overrides = all_overrides;

    for (id, config) in local.subagents {
        merged.subagents.insert(id, config);
    }

    let mut all_subagent_overrides = local.subagent_overrides;
    all_subagent_overrides.extend(merged.subagent_overrides);
    merged.subagent_overrides = all_subagent_overrides;

    for (id, config) in local.toolbox_commands {
        merged.toolbox_commands.insert(id, config);
    }

    for (id, config) in local.code_lens {
        merged.code_lens.insert(id, config);
    }

    merged.errors.extend(local.errors);

    merged
}

async fn load_modes(dir: &Path, registry: &mut ProjectRegistry, skip_defaults: bool) {
    let paths = collect_yaml_paths(dir).await;

    for path in paths {
        if skip_defaults && is_unchanged_default(&path, "modes").await {
            continue;
        }
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

async fn load_modes_skip_defaults(dir: &Path, registry: &mut ProjectRegistry) {
    load_modes(dir, registry, true).await;
}

async fn load_subagents(dir: &Path, registry: &mut ProjectRegistry, skip_defaults: bool) {
    let paths = collect_yaml_paths(dir).await;

    for path in paths {
        if skip_defaults && is_unchanged_default(&path, "subagents").await {
            continue;
        }
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

async fn load_subagents_skip_defaults(dir: &Path, registry: &mut ProjectRegistry) {
    load_subagents(dir, registry, true).await;
}

async fn load_toolbox_commands(dir: &Path, registry: &mut ProjectRegistry) {
    load_toolbox_commands_inner(dir, registry, false).await;
}

async fn load_toolbox_commands_skip_defaults(dir: &Path, registry: &mut ProjectRegistry) {
    load_toolbox_commands_inner(dir, registry, true).await;
}

async fn load_toolbox_commands_inner(dir: &Path, registry: &mut ProjectRegistry, skip_defaults: bool) {
    let paths = collect_yaml_paths(dir).await;

    for path in paths {
        if skip_defaults && is_unchanged_default(&path, "toolbox_commands").await {
            continue;
        }
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
    load_code_lens_inner(dir, registry, false).await;
}

async fn load_code_lens_skip_defaults(dir: &Path, registry: &mut ProjectRegistry) {
    load_code_lens_inner(dir, registry, true).await;
}

async fn load_code_lens_inner(dir: &Path, registry: &mut ProjectRegistry, skip_defaults: bool) {
    let paths = collect_yaml_paths(dir).await;

    for path in paths {
        if skip_defaults && is_unchanged_default(&path, "code_lens").await {
            continue;
        }
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

async fn is_unchanged_default(path: &Path, kind: &str) -> bool {
    let filename = match path.file_name().and_then(|f| f.to_str()) {
        Some(f) => f,
        None => return false,
    };

    let default_checksum = match get_default_checksum(kind, filename) {
        Some(c) => c,
        None => return false,
    };

    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return false,
    };

    compute_checksum(&content) == default_checksum
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
    let config_dir = gcx.read().await.config_dir.clone();
    let dirs = get_project_dirs(gcx.clone()).await;
    let project_root = dirs.first().cloned();

    let cache_key = project_root.clone().unwrap_or_else(|| config_dir.clone());

    {
        let gcx_locked = gcx.read().await;
        let cache_result = gcx_locked.project_registry_cache.read();
        if let Ok(cache) = cache_result {
            if let Some(cached) = cache.get(&cache_key) {
                return Some(cached.registry.clone());
            }
        }
    }

    let _ = global_configs_try_create_all(&config_dir).await;
    if let Some(ref root) = project_root {
        let _ = project_configs_ensure_dirs(root).await;
    }

    let registry = load_merged_registry(&config_dir, project_root.as_deref()).await;

    {
        let gcx_locked = gcx.read().await;
        let cache_result = gcx_locked.project_registry_cache.write();
        if let Ok(mut cache) = cache_result {
            cache.insert(cache_key, registry.clone());
        }
    }

    Some(registry)
}

#[allow(dead_code)]
pub async fn get_global_registry(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> ProjectRegistry {
    let config_dir = gcx.read().await.config_dir.clone();
    let _ = global_configs_try_create_all(&config_dir).await;
    load_registry_from_dir(&config_dir).await
}

pub async fn invalidate_all_registry_caches(gcx: Arc<ARwLock<GlobalContext>>) {
    let cache_arc = gcx.read().await.project_registry_cache.clone();
    if let Ok(mut cache) = cache_arc.write() {
        cache.cache.clear();
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_matches_pattern_exact() {
        assert!(model_matches_pattern("gpt-4o", "gpt-4o"));
        assert!(!model_matches_pattern("gpt-4o", "gpt-4"));
    }

    #[test]
    fn test_model_matches_pattern_wildcard() {
        assert!(model_matches_pattern("gpt-4o", "*"));
        assert!(model_matches_pattern("claude-3", "*"));
    }

    #[test]
    fn test_model_matches_pattern_prefix() {
        assert!(model_matches_pattern("gpt-4o", "gpt-*"));
        assert!(model_matches_pattern("gpt-4-turbo", "gpt-*"));
        assert!(!model_matches_pattern("claude-3", "gpt-*"));
    }

    #[test]
    fn test_glob_matches_exact() {
        assert!(glob_matches("tree", "tree"));
        assert!(!glob_matches("tree", "cat"));
    }

    #[test]
    fn test_glob_matches_wildcard() {
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("*", "tree"));
    }

    #[test]
    fn test_glob_matches_prefix() {
        assert!(glob_matches("search_*", "search_pattern"));
        assert!(glob_matches("search_*", "search_semantic"));
        assert!(!glob_matches("search_*", "tree"));
    }

    #[test]
    fn test_glob_matches_suffix() {
        assert!(glob_matches("*_textdoc", "create_textdoc"));
        assert!(glob_matches("*_textdoc", "update_textdoc"));
        assert!(!glob_matches("*_textdoc", "tree"));
    }

    #[test]
    fn test_match_tool_confirm_action() {
        let rules = vec![
            ToolConfirmRule { match_pattern: "tree".to_string(), action: "auto".to_string() },
            ToolConfirmRule { match_pattern: "search_*".to_string(), action: "auto".to_string() },
            ToolConfirmRule { match_pattern: "*".to_string(), action: "ask".to_string() },
        ];

        assert_eq!(match_tool_confirm_action(&rules, "tree"), Some("auto".to_string()));
        assert_eq!(match_tool_confirm_action(&rules, "search_pattern"), Some("auto".to_string()));
        assert_eq!(match_tool_confirm_action(&rules, "shell"), Some("ask".to_string()));
    }

    #[test]
    fn test_match_tool_confirm_action_empty_rules() {
        let rules: Vec<ToolConfirmRule> = vec![];
        assert_eq!(match_tool_confirm_action(&rules, "tree"), None);
    }

    #[test]
    fn test_map_legacy_mode_to_id() {
        assert_eq!(map_legacy_mode_to_id("AGENT"), "agent");
        assert_eq!(map_legacy_mode_to_id("EXPLORE"), "explore");
        assert_eq!(map_legacy_mode_to_id("NO_TOOLS"), "explore");
        assert_eq!(map_legacy_mode_to_id("CONFIGURE"), "configurator");
        assert_eq!(map_legacy_mode_to_id("PROJECT_SUMMARY"), "project_summary");
        assert_eq!(map_legacy_mode_to_id("TASK_PLANNER"), "task_planner");
        assert_eq!(map_legacy_mode_to_id("TASK_AGENT"), "task_agent");
    }

    #[test]
    fn test_map_legacy_mode_to_id_lowercase_passthrough() {
        assert_eq!(map_legacy_mode_to_id("agent"), "agent");
        assert_eq!(map_legacy_mode_to_id("custom_mode"), "custom_mode");
        assert_eq!(map_legacy_mode_to_id("my-mode-123"), "my-mode-123");
    }

    #[test]
    fn test_map_legacy_mode_to_id_invalid_fallback() {
        assert_eq!(map_legacy_mode_to_id("Invalid Mode"), "agent");
        assert_eq!(map_legacy_mode_to_id("Mode!"), "agent");
    }

    #[test]
    fn test_registry_has_required_subagents() {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let required_subagents = vec![
            "subagent",
            "subagent_with_editing",
            "code_review",
            "code_review_gather_files",
            "strategic_planning",
            "strategic_planning_gather_files",
            "deep_research",
            "commit_message",
            "title_generation",
            "kg_enrich",
            "kg_deprecate",
            "code_edit",
            "compress_trajectory",
            "follow_up",
            "http_subchat",
            "http_subchat_single",
            "memo_extraction",
        ];

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = global_configs_try_create_all(config_dir).await;
            let registry = load_registry_from_dir(config_dir).await;

            for id in &required_subagents {
                assert!(
                    registry.subagents.contains_key(*id),
                    "Missing required subagent: {}. Available: {:?}",
                    id,
                    registry.subagents.keys().collect::<Vec<_>>()
                );
            }
        });
    }

    #[test]
    fn test_registry_subagents_have_valid_subchat_params() {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = global_configs_try_create_all(config_dir).await;
            let registry = load_registry_from_dir(config_dir).await;

            for (id, config) in &registry.subagents {
                assert!(
                    config.subchat.n_ctx.unwrap_or(0) > 0,
                    "Subagent '{}' has invalid n_ctx",
                    id
                );
                assert!(
                    config.subchat.max_new_tokens.unwrap_or(0) > 0,
                    "Subagent '{}' has invalid max_new_tokens",
                    id
                );
                if let Some(ref model_type) = config.subchat.model_type {
                    let valid = model_type.eq_ignore_ascii_case("light")
                        || model_type.eq_ignore_ascii_case("default")
                        || model_type.eq_ignore_ascii_case("thinking");
                    assert!(
                        valid,
                        "Subagent '{}' has invalid model_type: {}",
                        id, model_type
                    );
                }
                if let Some(ref reasoning_effort) = config.subchat.reasoning_effort {
                    let valid = reasoning_effort.eq_ignore_ascii_case("low")
                        || reasoning_effort.eq_ignore_ascii_case("medium")
                        || reasoning_effort.eq_ignore_ascii_case("high");
                    assert!(
                        valid,
                        "Subagent '{}' has invalid reasoning_effort: {}",
                        id, reasoning_effort
                    );
                }
            }
        });
    }
}
