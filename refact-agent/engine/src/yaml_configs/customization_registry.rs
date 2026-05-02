use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::RwLock as ARwLock;

use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_types::*;
use crate::yaml_configs::project_configs_bootstrap::{
    global_configs_try_create_all, project_configs_ensure_dirs, compute_checksum,
    get_default_checksum,
};

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
        Self {
            cache: HashMap::new(),
        }
    }

    pub fn get(&self, project_root: &Path) -> Option<&RegistryCache> {
        self.cache.get(project_root)
    }

    pub fn insert(&mut self, project_root: PathBuf, registry: ProjectRegistry) {
        self.cache.insert(
            project_root.clone(),
            RegistryCache {
                project_root,
                registry,
                last_scan: SystemTime::now(),
            },
        );
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

async fn load_toolbox_commands_inner(
    dir: &Path,
    registry: &mut ProjectRegistry,
    skip_defaults: bool,
) {
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
        if path
            .extension()
            .map(|e| e == "yaml" || e == "yml")
            .unwrap_or(false)
        {
            let effectively_empty = tokio::fs::read_to_string(&path)
                .await
                .map(|c| c.trim().is_empty())
                .unwrap_or(false);
            if !effectively_empty {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

async fn load_yaml_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;
    serde_yaml::from_str(&content).map_err(|e| format!("Failed to parse YAML: {}", e))
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

    let matching_override = registry
        .mode_overrides
        .iter()
        .filter(|o| o.base.as_deref() == Some(mode_id))
        .filter_map(|o| {
            o.match_models
                .as_ref()
                .and_then(|patterns| best_matching_pattern_specificity(model_id, patterns))
                .map(|specificity| (specificity, o))
        })
        .fold(None, best_override_by_specificity)
        .map(|(_, o)| o);

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

    let matching_override = registry
        .subagent_overrides
        .iter()
        .filter(|o| o.base.as_deref() == Some(subagent_id))
        .filter_map(|o| {
            o.match_models
                .as_ref()
                .and_then(|patterns| best_matching_pattern_specificity(model_id, patterns))
                .map(|specificity| (specificity, o))
        })
        .fold(None, best_override_by_specificity)
        .map(|(_, o)| o);

    match matching_override {
        Some(override_config) => Some(base.apply_override(override_config)),
        None => Some(base.clone()),
    }
}

fn model_matches_pattern(model_id: &str, pattern: &str) -> bool {
    let canonical = crate::caps::model_caps::canonicalize_model_name(model_id);
    let candidates = [
        canonical.original.as_str(),
        canonical.provider_stripped.as_str(),
        canonical.base_model.as_str(),
        canonical.last_segment.as_str(),
        canonical.last_segment_base.as_str(),
    ];

    candidates
        .iter()
        .any(|c| model_matches_pattern_single(c, pattern))
        || {
            let pattern_norm = normalize_model_match_str(pattern);
            candidates
                .iter()
                .any(|c| model_matches_pattern_single(&normalize_model_match_str(c), &pattern_norm))
        }
}

fn normalize_model_match_str(s: &str) -> String {
    s.to_lowercase().replace('.', "-")
}

fn model_matches_pattern_single(model_id: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return model_id == pattern;
    }

    let segments: Vec<_> = pattern
        .split('*')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        return true;
    }

    let mut remaining = model_id;
    for (index, segment) in segments.iter().enumerate() {
        if index == 0 && !pattern.starts_with('*') {
            if !remaining.starts_with(segment) {
                return false;
            }
            remaining = &remaining[segment.len()..];
        } else if index == segments.len() - 1 && !pattern.ends_with('*') {
            return remaining.ends_with(segment);
        } else if let Some(segment_pos) = remaining.find(segment) {
            remaining = &remaining[segment_pos + segment.len()..];
        } else {
            return false;
        }
    }

    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModelPatternSpecificity {
    exact: bool,
    wildcard_rank: u8,
    literal_chars: usize,
}

impl ModelPatternSpecificity {
    fn for_pattern(pattern: &str) -> Self {
        let exact = !pattern.contains('*');
        Self {
            exact,
            wildcard_rank: if exact {
                0
            } else {
                wildcard_specificity_rank(pattern)
            },
            literal_chars: pattern.chars().filter(|c| *c != '*').count(),
        }
    }

    fn is_more_specific_than(self, other: Self) -> bool {
        if self.exact && other.exact {
            false
        } else if self.exact != other.exact {
            self.exact
        } else if self.wildcard_rank != other.wildcard_rank {
            self.wildcard_rank > other.wildcard_rank
        } else {
            self.literal_chars > other.literal_chars
        }
    }
}

fn wildcard_specificity_rank(pattern: &str) -> u8 {
    if is_specific_wildcard(pattern) {
        3
    } else if is_contains_wildcard(pattern) {
        2
    } else {
        1
    }
}

fn is_specific_wildcard(pattern: &str) -> bool {
    let literal = pattern.replace('*', "");
    literal
        .chars()
        .any(|c| c.is_ascii_digit() || c == '.' || c == '/' || c == '_')
}

fn is_contains_wildcard(pattern: &str) -> bool {
    pattern.starts_with('*') && pattern.ends_with('*') && pattern.chars().any(|c| c != '*')
}

fn best_matching_pattern_specificity(
    model_id: &str,
    patterns: &[String],
) -> Option<ModelPatternSpecificity> {
    patterns
        .iter()
        .filter(|pattern| model_matches_pattern(model_id, pattern))
        .map(|pattern| ModelPatternSpecificity::for_pattern(pattern))
        .fold(None, |best, specificity| match best {
            Some(best) if !specificity.is_more_specific_than(best) => Some(best),
            _ => Some(specificity),
        })
}

fn best_override_by_specificity<'a, T>(
    best: Option<(ModelPatternSpecificity, &'a T)>,
    candidate: (ModelPatternSpecificity, &'a T),
) -> Option<(ModelPatternSpecificity, &'a T)> {
    match best {
        Some(best) if !candidate.0.is_more_specific_than(best.0) => Some(best),
        _ => Some(candidate),
    }
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

pub async fn get_project_registry(gcx: Arc<ARwLock<GlobalContext>>) -> Option<ProjectRegistry> {
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
pub async fn get_global_registry(gcx: Arc<ARwLock<GlobalContext>>) -> ProjectRegistry {
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
        "TASK_PLANNER" => "task_planner",
        "TASK_AGENT" => "task_agent",
        _ => {
            if mode_str
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
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
    use std::collections::HashMap;

    fn runtime_required_subagent_ids() -> Vec<&'static str> {
        vec![
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
            "mode_transition",
            "memo_extraction",
        ]
    }

    fn read_default_mode_file(filename: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("yaml_configs")
                .join("defaults")
                .join("modes")
                .join(filename),
        )
        .unwrap_or_default()
    }

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn load_default_registry_for_tests() -> ProjectRegistry {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().to_path_buf();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            global_configs_try_create_all(&config_dir).await.unwrap();
            load_registry_from_dir(&config_dir).await
        })
    }

    fn base_agent_mode() -> ModeConfig {
        ModeConfig {
            schema_version: 1,
            id: "agent".to_string(),
            title: "Agent".to_string(),
            description: String::new(),
            specific: false,
            prompt: "base".to_string(),
            tools: vec!["tree".to_string(), "cat".to_string(), "shell".to_string()],
            allow_integrations: true,
            allow_mcp: true,
            allow_subagents: true,
            model_defaults: ModeModelDefaults::default(),
            tool_confirm: ToolConfirmConfig {
                rules: vec![
                    ToolConfirmRule {
                        match_pattern: "tree".to_string(),
                        action: "auto".to_string(),
                    },
                    ToolConfirmRule {
                        match_pattern: "shell".to_string(),
                        action: "ask".to_string(),
                    },
                ],
            },
            thread_defaults: ModeThreadDefaults {
                include_project_info: Some(true),
                checkpoints_enabled: Some(true),
                auto_approve_editing_tools: Some(true),
                auto_approve_dangerous_commands: Some(false),
            },
            ui: ModeUi::default(),
            base: None,
            match_models: None,
            override_config: None,
        }
    }

    fn mode_override(id: &str, patterns: &[&str], prompt: &str) -> ModeConfig {
        ModeConfig {
            schema_version: 1,
            id: id.to_string(),
            title: String::new(),
            description: String::new(),
            specific: true,
            prompt: String::new(),
            tools: Vec::new(),
            allow_integrations: false,
            allow_mcp: false,
            allow_subagents: false,
            model_defaults: ModeModelDefaults::default(),
            tool_confirm: ToolConfirmConfig::default(),
            thread_defaults: ModeThreadDefaults::default(),
            ui: ModeUi::default(),
            base: Some("agent".to_string()),
            match_models: Some(patterns.iter().map(|pattern| pattern.to_string()).collect()),
            override_config: Some(ModeOverride {
                prompt: Some(prompt.to_string()),
                ..Default::default()
            }),
        }
    }

    fn registry_with_mode_overrides(overrides: Vec<ModeConfig>) -> ProjectRegistry {
        let mut modes = HashMap::new();
        modes.insert("agent".to_string(), base_agent_mode());
        ProjectRegistry {
            modes,
            mode_overrides: overrides,
            ..Default::default()
        }
    }

    fn base_subagent() -> SubagentConfig {
        SubagentConfig {
            schema_version: 1,
            id: "coder".to_string(),
            title: "Base".to_string(),
            description: String::new(),
            specific: false,
            expose_as_tool: false,
            has_code: false,
            tool: None,
            subchat: SubchatConfig::default(),
            messages: SubagentMessages::default(),
            prompts: SubagentPrompts::default(),
            gather_files: GatherFilesConfig::default(),
            tools: Vec::new(),
            base: None,
            match_models: None,
            extra: HashMap::new(),
        }
    }

    fn subagent_override(id: &str, patterns: &[&str], title: &str) -> SubagentConfig {
        SubagentConfig {
            schema_version: 1,
            id: id.to_string(),
            title: title.to_string(),
            description: String::new(),
            specific: true,
            expose_as_tool: false,
            has_code: false,
            tool: None,
            subchat: SubchatConfig::default(),
            messages: SubagentMessages::default(),
            prompts: SubagentPrompts::default(),
            gather_files: GatherFilesConfig::default(),
            tools: Vec::new(),
            base: Some("coder".to_string()),
            match_models: Some(patterns.iter().map(|pattern| pattern.to_string()).collect()),
            extra: HashMap::new(),
        }
    }

    fn assert_mode_inherits_agent_surface(agent: &ModeConfig, resolved: &ModeConfig) {
        assert_eq!(resolved.tools, agent.tools);
        assert_eq!(resolved.allow_integrations, agent.allow_integrations);
        assert_eq!(resolved.allow_mcp, agent.allow_mcp);
        assert_eq!(resolved.allow_subagents, agent.allow_subagents);
        assert_eq!(
            resolved.tool_confirm.rules.len(),
            agent.tool_confirm.rules.len()
        );
        for (resolved_rule, agent_rule) in resolved
            .tool_confirm
            .rules
            .iter()
            .zip(agent.tool_confirm.rules.iter())
        {
            assert_eq!(resolved_rule.match_pattern, agent_rule.match_pattern);
            assert_eq!(resolved_rule.action, agent_rule.action);
        }
        assert_eq!(
            resolved.thread_defaults.include_project_info,
            agent.thread_defaults.include_project_info
        );
        assert_eq!(
            resolved.thread_defaults.checkpoints_enabled,
            agent.thread_defaults.checkpoints_enabled
        );
        assert_eq!(
            resolved.thread_defaults.auto_approve_editing_tools,
            agent.thread_defaults.auto_approve_editing_tools
        );
        assert_eq!(
            resolved.thread_defaults.auto_approve_dangerous_commands,
            agent.thread_defaults.auto_approve_dangerous_commands
        );
    }

    struct ProviderAgentPromptCase {
        filename: &'static str,
        model_id: &'static str,
        markers: &'static [&'static str],
    }

    fn provider_agent_prompt_cases() -> Vec<ProviderAgentPromptCase> {
        vec![
            ProviderAgentPromptCase {
                filename: "gpt55_agent.yaml",
                model_id: "gpt-5.5",
                markers: &[
                    "GPT-5.5",
                    "outcome-first",
                    "OpenAI reasoning/tool continuity",
                ],
            },
            ProviderAgentPromptCase {
                filename: "claude_opus47_agent.yaml",
                model_id: "claude-opus-4-7",
                markers: &[
                    "Claude Opus 4.7",
                    "adaptive thinking/effort",
                    "thinking blocks/signatures byte-for-byte",
                ],
            },
            ProviderAgentPromptCase {
                filename: "gemini3_agent.yaml",
                model_id: "gemini-3.1-pro-preview",
                markers: &["Gemini 3", "functionCall.id", "thought summaries"],
            },
            ProviderAgentPromptCase {
                filename: "kimi_k26_agent.yaml",
                model_id: "kimi-k2.6",
                markers: &["Kimi K2.6", "reasoning_content", "strict tool schemas"],
            },
            ProviderAgentPromptCase {
                filename: "glm51_agent.yaml",
                model_id: "glm-5.1",
                markers: &[
                    "GLM-5.1",
                    "`reasoning_content`",
                    "plan → execute → validate",
                ],
            },
            ProviderAgentPromptCase {
                filename: "minimax_m27_agent.yaml",
                model_id: "MiniMax-M2.7",
                markers: &[
                    "MiniMax M2.7",
                    "complete assistant content arrays",
                    "Anthropic-style `tool_use`/`tool_result` continuity",
                ],
            },
            ProviderAgentPromptCase {
                filename: "qwen36_agent.yaml",
                model_id: "qwen3.6-flash",
                markers: &[
                    "Qwen3.6 coding models",
                    "exact JSON tool arguments",
                    "ReAct stopword assumptions",
                ],
            },
        ]
    }

    fn assert_provider_prompt_contains_base_agent_contract(prompt: &str) {
        for marker in [
            "[mode3] You are Refact Agent",
            "## Core Philosophy: Orchestrate",
            "subagent()",
            "strategic_planning()",
            "code_review()",
            "## Memory & Past Conversations",
            "knowledge(search_key)",
            "create_knowledge(content)",
            "## Workflow",
            "Understand the Task",
            "Implement without Delegation",
            "Validate via Delegation",
            "tasks_set",
            "task_done()",
            "ask_questions()",
            "%CD_INSTRUCTIONS%",
            "%SHELL_INSTRUCTIONS%",
            "%COMPRESS_HANDOFF_INSTRUCTIONS%",
            "%RICH_CONTENT_INSTRUCTIONS%",
            "%SYSTEM_INFO%",
            "%ENVIRONMENT_INFO%",
            "%WORKSPACE_INFO%",
            "%SKILLS_INSTRUCTIONS%",
            "%PROJECT_CONFIGS%",
            "%GIT_INFO%",
            "%PROJECT_TREE%",
        ] {
            assert!(
                prompt.contains(marker),
                "provider prompt missing base Agent marker '{}': {}",
                marker,
                prompt
            );
        }
    }

    fn yaml_key(key: &str) -> serde_yaml::Value {
        serde_yaml::Value::String(key.to_string())
    }

    #[tokio::test]
    async fn imported_competitor_subagent_loads_through_registry() {
        use crate::ext::competitor_import::types::ImportStatus;

        let workspace = tempfile::tempdir().unwrap();
        write_file(
            &workspace.path().join(".claude/agents/registry-reviewer.md"),
            "---\nname: Registry Reviewer\ndescription: Reviews imported registry behavior\ntools:\n  - Read\n  - Grep\n  - Edit\ndenied-tools:\n  - Edit\nmaxTurns: 7\nmodel: sonnet\n---\nUse registry context to review {{task}}.",
        );

        let summary = crate::ext::competitor_import::run_project_import_with_paths(&[workspace
            .path()
            .to_path_buf()])
        .await;
        let registry = load_registry_from_dir(&workspace.path().join(".refact")).await;

        assert_eq!(summary.status_counts.get(&ImportStatus::Created), Some(&1));
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        let subagent = registry
            .subagents
            .get("registry-reviewer")
            .expect("imported subagent should load through registry");
        assert_eq!(subagent.schema_version, 2);
        assert_eq!(subagent.title, "Registry Reviewer");
        assert_eq!(subagent.description, "Reviews imported registry behavior");
        assert_eq!(subagent.subchat.max_steps, Some(7));
        assert_eq!(subagent.subchat.model.as_deref(), Some("sonnet"));
        assert_eq!(
            subagent.messages.user_template.as_deref(),
            Some("{{task}}\n")
        );
        assert_eq!(subagent.tools, vec!["cat", "search_pattern"]);
        let tool = subagent.tool.as_ref().unwrap();
        assert!(tool.agentic);
        assert_eq!(tool.required, vec!["task"]);
    }

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
        assert!(model_matches_pattern("openai/gpt-4o", "gpt-4*"));
        assert!(model_matches_pattern("openrouter/openai/gpt-4o", "gpt-4*"));
        assert!(model_matches_pattern("claude-3.7-sonnet", "claude-3-7*"));
        assert!(model_matches_pattern(
            "anthropic/claude-3.7-sonnet",
            "claude-3-7*"
        ));

        assert!(model_matches_pattern("gpt-4o", "gpt-*"));
        assert!(model_matches_pattern("gpt-4-turbo", "gpt-*"));
        assert!(!model_matches_pattern("claude-3", "gpt-*"));
    }

    #[test]
    fn test_model_matches_pattern_contains_wildcard() {
        assert!(model_matches_pattern("llama-7b", "*7b*"));
        assert!(model_matches_pattern("qwen2.5-7b-instruct", "*7b*"));
        assert!(model_matches_pattern("phi-mini", "*mini*"));
        assert!(!model_matches_pattern("abc-def", "*def*abc*"));
    }

    #[test]
    fn test_mode_override_specificity_contains_wildcard_beats_broad_family() {
        let registry = registry_with_mode_overrides(vec![
            mode_override("oss_agent", &["llama*", "qwen*", "phi*"], "generic"),
            mode_override("oss_weak_agent", &["*7b*", "*mini*"], "weak"),
        ]);

        let llama = resolve_mode_for_model(&registry, "agent", Some("llama-7b"))
            .expect("agent mode should resolve");
        let qwen = resolve_mode_for_model(&registry, "agent", Some("qwen2.5-7b-instruct"))
            .expect("agent mode should resolve");
        let phi = resolve_mode_for_model(&registry, "agent", Some("phi-mini"))
            .expect("agent mode should resolve");
        let generic = resolve_mode_for_model(&registry, "agent", Some("qwen2.5-coder"))
            .expect("agent mode should resolve");

        assert_eq!(llama.prompt, "weak");
        assert_eq!(qwen.prompt, "weak");
        assert_eq!(phi.prompt, "weak");
        assert_eq!(generic.prompt, "generic");
    }

    #[test]
    fn test_mode_override_specificity_exact_beats_broad_wildcard() {
        let registry = registry_with_mode_overrides(vec![
            mode_override("openai_agent", &["gpt-5*"], "broad"),
            mode_override("gpt55_agent", &["gpt-5.5"], "exact"),
        ]);

        let resolved = resolve_mode_for_model(&registry, "agent", Some("openai/gpt-5.5"))
            .expect("agent mode should resolve");

        assert_eq!(resolved.prompt, "exact");
    }

    #[test]
    fn test_mode_override_specificity_qwen_provider_pattern_beats_broad() {
        let registry = registry_with_mode_overrides(vec![
            mode_override("oss_agent", &["qwen*"], "broad"),
            mode_override(
                "qwen36_agent",
                &["qwen3.6-flash", "Qwen/Qwen3.6-*"],
                "specific",
            ),
        ]);

        let exact_resolved = resolve_mode_for_model(&registry, "agent", Some("qwen3.6-flash"))
            .expect("agent mode should resolve");
        let provider_resolved =
            resolve_mode_for_model(&registry, "agent", Some("alibaba/Qwen/Qwen3.6-flash"))
                .expect("agent mode should resolve");

        assert!(model_matches_pattern("qwen3.6-flash", "qwen*"));
        assert!(model_matches_pattern("qwen3.6-flash", "qwen3.6-flash"));
        assert!(model_matches_pattern("alibaba/Qwen/Qwen3.6-flash", "qwen*"));
        assert!(model_matches_pattern(
            "alibaba/Qwen/Qwen3.6-flash",
            "Qwen/Qwen3.6-*"
        ));
        assert_eq!(exact_resolved.prompt, "specific");
        assert_eq!(provider_resolved.prompt, "specific");
    }

    #[test]
    fn test_mode_override_specificity_preserves_stable_order_on_tie() {
        let registry = registry_with_mode_overrides(vec![
            mode_override("first", &["gpt-5*"], "first"),
            mode_override("second", &["gpt-5*"], "second"),
        ]);

        let resolved = resolve_mode_for_model(&registry, "agent", Some("gpt-5.5"))
            .expect("agent mode should resolve");

        assert_eq!(resolved.prompt, "first");
    }

    #[test]
    fn test_mode_override_specificity_preserves_local_precedence_on_tie() {
        let global =
            registry_with_mode_overrides(vec![mode_override("global", &["gpt-5*"], "global")]);
        let local = ProjectRegistry {
            mode_overrides: vec![mode_override("local", &["gpt-5*"], "local")],
            ..Default::default()
        };
        let merged = merge_registries(global, local);

        let resolved = resolve_mode_for_model(&merged, "agent", Some("gpt-5.5"))
            .expect("agent mode should resolve");

        assert_eq!(resolved.prompt, "local");
    }

    #[test]
    fn test_subagent_override_specificity_uses_best_match() {
        let mut subagents = HashMap::new();
        subagents.insert("coder".to_string(), base_subagent());
        let registry = ProjectRegistry {
            subagents,
            subagent_overrides: vec![
                subagent_override("oss_coder", &["qwen*"], "Broad"),
                subagent_override(
                    "qwen36_coder",
                    &["qwen3.6-flash", "Qwen/Qwen3.6-*"],
                    "Specific",
                ),
            ],
            ..Default::default()
        };

        let exact_resolved = resolve_subagent_for_model(&registry, "coder", Some("qwen3.6-flash"))
            .expect("subagent should resolve");
        let provider_resolved =
            resolve_subagent_for_model(&registry, "coder", Some("alibaba/Qwen/Qwen3.6-flash"))
                .expect("subagent should resolve");

        assert_eq!(exact_resolved.title, "Specific");
        assert_eq!(provider_resolved.title, "Specific");
    }

    #[test]
    fn test_prompt_and_model_defaults_only_mode_override_inherits_agent_surface() {
        let mut overlay = mode_override("provider_agent", &["provider-model*"], "provider");
        overlay
            .override_config
            .as_mut()
            .expect("override config should exist")
            .model_defaults = Some(ModeModelDefaults {
            default: Some(ModelTypeConfig {
                temperature: Some(0.2),
                ..Default::default()
            }),
            ..Default::default()
        });
        let registry = registry_with_mode_overrides(vec![overlay]);
        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();

        let resolved = resolve_mode_for_model(&registry, "agent", Some("provider-model-pro"))
            .expect("agent mode should resolve");

        assert_eq!(resolved.prompt, "provider");
        assert_eq!(
            resolved
                .model_defaults
                .default
                .as_ref()
                .and_then(|defaults| defaults.temperature),
            Some(0.2)
        );
        assert_mode_inherits_agent_surface(&agent, &resolved);
    }

    #[test]
    fn test_provider_agent_overlay_prompts_include_full_agent_contract() {
        let registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);

        for case in provider_agent_prompt_cases() {
            let resolved = resolve_mode_for_model(&registry, "agent", Some(case.model_id))
                .expect("agent mode should resolve");

            assert_provider_prompt_contains_base_agent_contract(&resolved.prompt);
            for marker in case.markers {
                assert!(
                    resolved.prompt.contains(marker),
                    "{} resolved prompt missing provider marker '{}': {}",
                    case.model_id,
                    marker,
                    resolved.prompt
                );
            }
        }
    }

    #[test]
    fn test_provider_agent_overlays_do_not_replace_tool_or_capability_surface() {
        let forbidden_top_level = [
            "tools",
            "tool_confirm",
            "allow_integrations",
            "allow_mcp",
            "allow_subagents",
            "thread_defaults",
        ];
        let forbidden_override = [
            "tools_replace",
            "tools_add",
            "tools_remove",
            "tool_confirm",
            "allow_integrations",
            "allow_mcp",
            "allow_subagents",
            "thread_defaults",
        ];

        for case in provider_agent_prompt_cases() {
            let content = read_default_mode_file(case.filename);
            let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
                .unwrap_or_else(|err| panic!("{} should parse as YAML: {}", case.filename, err));
            let root = yaml
                .as_mapping()
                .unwrap_or_else(|| panic!("{} root should be a mapping", case.filename));

            for key in forbidden_top_level {
                assert!(
                    !root.contains_key(&yaml_key(key)),
                    "{} must not define top-level {}",
                    case.filename,
                    key
                );
            }

            let override_mapping = root
                .get(&yaml_key("override"))
                .and_then(|value| value.as_mapping())
                .unwrap_or_else(|| panic!("{} should define override mapping", case.filename));
            for key in forbidden_override {
                assert!(
                    !override_mapping.contains_key(&yaml_key(key)),
                    "{} must not define override {}",
                    case.filename,
                    key
                );
            }
        }
    }

    #[test]
    fn test_default_oss_agent_overlay_inherits_agent_surface() {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = global_configs_try_create_all(config_dir).await;
            let registry = load_registry_from_dir(config_dir).await;
            assert!(registry.errors.is_empty(), "{:?}", registry.errors);

            let agent = registry
                .modes
                .get("agent")
                .expect("base agent should exist")
                .clone();
            let resolved = resolve_mode_for_model(&registry, "agent", Some("qwen2.5-coder"))
                .expect("agent mode should resolve");

            assert_ne!(resolved.prompt, agent.prompt);
            assert!(resolved.prompt.contains("open-source or local model"));
            assert!(!resolved
                .prompt
                .contains("weaker open-source or local model"));
            assert!(!resolved.prompt.contains("Qwen3.6 coding models"));
            assert_mode_inherits_agent_surface(&agent, &resolved);
        });
    }

    #[test]
    fn test_default_oss_weak_agent_overlay_beats_broad_oss_overlay() {
        let registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();

        for model_id in ["llama-7b", "qwen2.5-7b-instruct", "phi-mini"] {
            let resolved = resolve_mode_for_model(&registry, "agent", Some(model_id))
                .expect("agent mode should resolve");

            assert!(
                resolved
                    .prompt
                    .contains("weaker open-source or local model"),
                "{} should resolve to weak OSS prompt: {}",
                model_id,
                resolved.prompt
            );
            assert!(!resolved.prompt.contains("Qwen3.6 coding models"));
            assert_mode_inherits_agent_surface(&agent, &resolved);
        }

        let generic = resolve_mode_for_model(&registry, "agent", Some("qwen2.5-coder"))
            .expect("agent mode should resolve");

        assert!(generic.prompt.contains("open-source or local model"));
        assert!(!generic.prompt.contains("weaker open-source or local model"));
        assert!(!generic.prompt.contains("Qwen3.6 coding models"));
        assert_mode_inherits_agent_surface(&agent, &generic);
    }

    #[test]
    fn test_default_gpt55_agent_overlay_resolves_specific_and_inherits_agent_surface() {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = global_configs_try_create_all(config_dir).await;
            let registry = load_registry_from_dir(config_dir).await;
            assert!(registry.errors.is_empty(), "{:?}", registry.errors);

            let agent = registry
                .modes
                .get("agent")
                .expect("base agent should exist")
                .clone();

            for model_id in ["gpt-5.5", "openai/gpt-5.5-2026-04-23"] {
                let resolved = resolve_mode_for_model(&registry, "agent", Some(model_id))
                    .expect("agent mode should resolve");

                assert!(resolved.prompt.contains("GPT-5.5"));
                assert!(resolved.prompt.contains("outcome-first"));
                assert_eq!(
                    resolved
                        .model_defaults
                        .default
                        .as_ref()
                        .and_then(|defaults| defaults.reasoning_effort.as_deref()),
                    Some("medium")
                );
                assert_eq!(
                    resolved
                        .model_defaults
                        .thinking
                        .as_ref()
                        .and_then(|defaults| defaults.reasoning_effort.as_deref()),
                    Some("high")
                );
                assert_mode_inherits_agent_surface(&agent, &resolved);
            }
        });
    }

    #[test]
    fn test_default_gpt55_agent_overlay_beats_generic_openai_agent() {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = global_configs_try_create_all(config_dir).await;
            let registry = load_registry_from_dir(config_dir).await;
            assert!(registry.errors.is_empty(), "{:?}", registry.errors);

            let generic = resolve_mode_for_model(&registry, "agent", Some("openai/gpt-5-latest"))
                .expect("agent mode should resolve");
            let specific =
                resolve_mode_for_model(&registry, "agent", Some("openai/gpt-5.5-latest"))
                    .expect("agent mode should resolve");

            assert!(generic.prompt.contains("OpenAI models"));
            assert!(specific.prompt.contains("GPT-5.5"));
            assert!(specific.prompt.contains("outcome-first"));
            assert!(!specific.prompt.contains("OpenAI models"));
        });
    }

    #[test]
    fn test_default_claude_opus47_agent_overlay_resolves_specific_and_inherits_agent_surface() {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = global_configs_try_create_all(config_dir).await;
            let registry = load_registry_from_dir(config_dir).await;
            assert!(registry.errors.is_empty(), "{:?}", registry.errors);

            let agent = registry
                .modes
                .get("agent")
                .expect("base agent should exist")
                .clone();
            let overlay = registry
                .mode_overrides
                .iter()
                .find(|mode| mode.id == "claude_opus47_agent")
                .expect("claude opus overlay should load");
            let overlay_defaults = overlay
                .override_config
                .as_ref()
                .and_then(|override_config| override_config.model_defaults.as_ref())
                .expect("claude opus overlay should define model defaults");

            assert_eq!(
                overlay_defaults
                    .default
                    .as_ref()
                    .and_then(|defaults| defaults.thinking_budget),
                None
            );
            assert_eq!(
                overlay_defaults
                    .thinking
                    .as_ref()
                    .and_then(|defaults| defaults.thinking_budget),
                None
            );

            for model_id in ["claude-opus-4-7", "anthropic/claude-opus-4-7-latest"] {
                let resolved = resolve_mode_for_model(&registry, "agent", Some(model_id))
                    .expect("agent mode should resolve");

                assert!(resolved.prompt.contains("Opus 4.7"));
                assert!(resolved.prompt.contains("adaptive thinking"));
                assert_eq!(
                    resolved
                        .model_defaults
                        .default
                        .as_ref()
                        .and_then(|defaults| defaults.reasoning_effort.as_deref()),
                    Some("high")
                );
                assert_eq!(
                    resolved
                        .model_defaults
                        .thinking
                        .as_ref()
                        .and_then(|defaults| defaults.reasoning_effort.as_deref()),
                    Some("xhigh")
                );
                assert_mode_inherits_agent_surface(&agent, &resolved);
            }
        });
    }

    #[test]
    fn test_default_gemini3_agent_overlay_inherits_agent_surface() {
        let registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);

        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();

        for model_id in ["gemini-3.1-pro-preview", "google/gemini-3-flash"] {
            let resolved = resolve_mode_for_model(&registry, "agent", Some(model_id))
                .expect("agent mode should resolve");

            assert!(resolved.prompt.contains("Gemini 3"));
            assert!(resolved.prompt.contains("functionCall.id"));
            assert!(resolved.prompt.contains("high thinking"));
            assert!(resolved.prompt.contains("medium thinking"));
            assert_eq!(
                resolved
                    .model_defaults
                    .default
                    .as_ref()
                    .and_then(|defaults| defaults.reasoning_effort.as_deref()),
                Some("high")
            );
            assert_mode_inherits_agent_surface(&agent, &resolved);
        }
    }

    #[test]
    fn test_default_kimi_k26_agent_overlay_inherits_agent_surface() {
        let registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);

        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();

        for model_id in ["kimi-k2.6", "moonshot/kimi-k2.6"] {
            let resolved = resolve_mode_for_model(&registry, "agent", Some(model_id))
                .expect("agent mode should resolve");

            assert!(resolved.prompt.contains("Kimi K2.6"));
            assert!(resolved.prompt.contains("reasoning_content"));
            assert!(resolved.prompt.contains("strict tool schemas"));
            assert_eq!(
                resolved
                    .model_defaults
                    .default
                    .as_ref()
                    .and_then(|defaults| defaults.temperature),
                Some(1.0)
            );
            assert_mode_inherits_agent_surface(&agent, &resolved);
        }
    }

    #[test]
    fn test_kimi_k26_agent_overlay_beats_broad_oss_overlay() {
        let mut registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();
        registry
            .mode_overrides
            .push(mode_override("future_oss_kimi", &["kimi*"], "broad oss"));

        let resolved = resolve_mode_for_model(&registry, "agent", Some("kimi-k2.6"))
            .expect("agent mode should resolve");

        assert!(resolved.prompt.contains("Kimi K2.6"));
        assert_ne!(resolved.prompt, "broad oss");
        assert_mode_inherits_agent_surface(&agent, &resolved);
    }

    #[test]
    fn test_default_glm51_agent_overlay_inherits_agent_surface() {
        let registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        assert!(registry
            .mode_overrides
            .iter()
            .any(|mode| mode.id == "glm51_agent"));

        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();

        for model_id in ["glm-5.1", "GLM-5.1", "zhipu/glm-5.1"] {
            let resolved = resolve_mode_for_model(&registry, "agent", Some(model_id))
                .expect("agent mode should resolve");

            assert!(resolved.prompt.contains("GLM-5.1"));
            assert!(resolved.prompt.contains("`reasoning_content`"));
            assert_eq!(
                resolved
                    .model_defaults
                    .default
                    .as_ref()
                    .and_then(|defaults| defaults.temperature),
                Some(1.0)
            );
            assert_mode_inherits_agent_surface(&agent, &resolved);
        }
    }

    #[test]
    fn test_default_minimax_m27_agent_overlay_inherits_agent_surface() {
        let registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        assert!(registry
            .mode_overrides
            .iter()
            .any(|mode| mode.id == "minimax_m27_agent"));

        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();

        for model_id in [
            "MiniMax-M2.7",
            "minimax-m2.7",
            "minimax/MiniMax-M2.7-highspeed",
        ] {
            let resolved = resolve_mode_for_model(&registry, "agent", Some(model_id))
                .expect("agent mode should resolve");

            assert!(resolved.prompt.contains("MiniMax M2.7"));
            assert!(resolved
                .prompt
                .contains("complete assistant content arrays"));
            assert!(resolved.prompt.contains("Anthropic-style `tool_use`"));
            assert_eq!(
                resolved
                    .model_defaults
                    .default
                    .as_ref()
                    .and_then(|defaults| defaults.temperature),
                Some(1.0)
            );
            assert_eq!(
                resolved
                    .model_defaults
                    .default
                    .as_ref()
                    .and_then(|defaults| defaults.top_p),
                Some(0.95)
            );
            assert_mode_inherits_agent_surface(&agent, &resolved);
        }
    }

    #[test]
    fn test_default_qwen36_agent_overlay_matches_specific_models_and_inherits_agent_surface() {
        let registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();

        for model_id in [
            "qwen3.6-flash",
            "qwen-3.6-35b",
            "Qwen/Qwen3.6-35B-A3B",
            "modelstudio/qwen3.6-flash",
        ] {
            let resolved = resolve_mode_for_model(&registry, "agent", Some(model_id))
                .expect("agent mode should resolve");

            assert!(
                resolved.prompt.contains("Qwen3.6 coding models"),
                "{} should resolve to Qwen3.6 prompt: {}",
                model_id,
                resolved.prompt
            );
            assert!(resolved
                .prompt
                .contains("Preserve thinking and reasoning content"));
            assert!(resolved.prompt.contains("exact JSON tool arguments"));
            assert!(resolved.prompt.contains("ReAct stopword assumptions"));
            assert_eq!(
                resolved
                    .model_defaults
                    .default
                    .as_ref()
                    .and_then(|defaults| defaults.temperature),
                Some(1.0)
            );
            assert_mode_inherits_agent_surface(&agent, &resolved);
        }
    }

    #[test]
    fn test_default_qwen36_agent_overlay_beats_broad_qwen_and_preserves_generic_qwen_fallback() {
        let registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();

        assert!(model_matches_pattern("qwen3.6-flash", "qwen*"));
        let qwen36 = resolve_mode_for_model(&registry, "agent", Some("qwen3.6-flash"))
            .expect("agent mode should resolve");
        assert!(qwen36.prompt.contains("Qwen3.6 coding models"));
        assert!(!qwen36.prompt.contains("open-source or local model"));
        assert_mode_inherits_agent_surface(&agent, &qwen36);

        let qwen25 = resolve_mode_for_model(&registry, "agent", Some("qwen2.5-coder"))
            .expect("agent mode should resolve");
        assert!(qwen25.prompt.contains("open-source or local model"));
        assert!(!qwen25.prompt.contains("Qwen3.6 coding models"));
        assert_mode_inherits_agent_surface(&agent, &qwen25);
    }

    #[test]
    fn test_default_provider_agent_overlays_parse_together_and_inherit_agent_surface() {
        let registry = load_default_registry_for_tests();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        let agent = registry
            .modes
            .get("agent")
            .expect("base agent should exist")
            .clone();

        for overlay in registry
            .mode_overrides
            .iter()
            .filter(|overlay| overlay.base.as_deref() == Some("agent"))
            .filter(|overlay| overlay.id != "openai_agent")
        {
            let resolved = agent.apply_override(
                overlay
                    .override_config
                    .as_ref()
                    .expect("provider overlay should define override"),
            );

            assert_mode_inherits_agent_surface(&agent, &resolved);
        }
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
            ToolConfirmRule {
                match_pattern: "tree".to_string(),
                action: "auto".to_string(),
            },
            ToolConfirmRule {
                match_pattern: "search_*".to_string(),
                action: "auto".to_string(),
            },
            ToolConfirmRule {
                match_pattern: "*".to_string(),
                action: "ask".to_string(),
            },
        ];

        assert_eq!(
            match_tool_confirm_action(&rules, "tree"),
            Some("auto".to_string())
        );
        assert_eq!(
            match_tool_confirm_action(&rules, "search_pattern"),
            Some("auto".to_string())
        );
        assert_eq!(
            match_tool_confirm_action(&rules, "shell"),
            Some("ask".to_string())
        );
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
    fn test_registry_has_runtime_required_subagents() {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = global_configs_try_create_all(config_dir).await;
            let registry = load_registry_from_dir(config_dir).await;

            for id in runtime_required_subagent_ids() {
                assert!(
                    registry.subagents.contains_key(id),
                    "Missing required subagent: {}. Available: {:?}",
                    id,
                    registry.subagents.keys().collect::<Vec<_>>()
                );
            }
        });
    }

    #[test]
    fn test_registry_subagents_have_valid_optional_subchat_params() {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = global_configs_try_create_all(config_dir).await;
            let registry = load_registry_from_dir(config_dir).await;

            for (id, config) in &registry.subagents {
                if let Some(n_ctx) = config.subchat.n_ctx {
                    assert!(n_ctx > 0, "Subagent '{}' has invalid n_ctx", id);
                }
                if let Some(max_new_tokens) = config.subchat.max_new_tokens {
                    assert!(
                        max_new_tokens > 0,
                        "Subagent '{}' has invalid max_new_tokens",
                        id
                    );
                }
                if let Some(ref model_type) = config.subchat.model_type {
                    let valid = model_type.eq_ignore_ascii_case("light")
                        || model_type.eq_ignore_ascii_case("default")
                        || model_type.eq_ignore_ascii_case("thinking")
                        || model_type.eq_ignore_ascii_case("buddy");
                    assert!(
                        valid,
                        "Subagent '{}' has invalid model_type: {}",
                        id, model_type
                    );
                }
                if let Some(ref reasoning_effort) = config.subchat.reasoning_effort {
                    let valid = reasoning_effort.eq_ignore_ascii_case("low")
                        || reasoning_effort.eq_ignore_ascii_case("medium")
                        || reasoning_effort.eq_ignore_ascii_case("high")
                        || reasoning_effort.eq_ignore_ascii_case("xhigh")
                        || reasoning_effort.eq_ignore_ascii_case("max");
                    assert!(
                        valid,
                        "Subagent '{}' has invalid reasoning_effort: {}",
                        id, reasoning_effort
                    );
                }
            }
        });
    }

    #[test]
    fn test_setup_mode_stays_read_only_by_default() {
        let content = read_default_mode_file("setup.yaml");

        assert!(content.contains("allow_integrations: false"));
        assert!(content.contains("allow_mcp: false"));
        assert!(content.contains("allow_subagents: false"));
        assert!(!content.contains("\n  - rm\n"));
        assert!(!content.contains("\n  - mv\n"));
    }

    #[test]
    fn test_setup_mcp_prompt_prefers_safe_examples() {
        let content = read_default_mode_file("setup_mcp.yaml");

        assert!(
            content.contains("HTTP over deprecated SSE")
                || content.contains("prefer HTTP over deprecated SSE")
        );
        assert!(!content.contains("@latest"));
        assert!(content.contains("@modelcontextprotocol/server-github@<version>"));
    }

    #[test]
    fn test_setup_skills_prompt_documents_supported_skill_fields() {
        let content = read_default_mode_file("setup_skills.yaml");

        for key in [
            "argument-hint",
            "allowed-tools",
            "user-invocable",
            "disable-model-invocation",
            "@include <relative-file>",
        ] {
            assert!(
                content.contains(key),
                "setup_skills.yaml should mention '{}': {}",
                key,
                content
            );
        }
    }

    #[test]
    fn test_registry_has_no_mode_config_errors() {
        use crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all;

        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = global_configs_try_create_all(config_dir).await;
            let registry = load_registry_from_dir(config_dir).await;

            let mode_errors: Vec<_> = registry
                .errors
                .iter()
                .filter(|e| {
                    Path::new(&e.file_path)
                        .components()
                        .any(|c| c == std::path::Component::Normal("modes".as_ref()))
                })
                .map(|e| format!("{}: {}", e.file_path, e.error))
                .collect();

            assert!(
                mode_errors.is_empty(),
                "Default mode configs should parse without errors. Found: {:?}",
                mode_errors
            );
        });
    }
}
