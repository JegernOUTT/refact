use rust_embed::RustEmbed;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use tracing::{info, warn};

const CHECKSUM_FILE: &str = "default-checksums.yaml";

#[derive(RustEmbed)]
#[folder = "src/defaults/"]
struct DefaultConfigs;

#[derive(Deserialize)]
struct SchemaVersionOnly {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
}

fn default_schema_version() -> u32 {
    1
}

pub async fn global_configs_try_create_all(config_dir: &Path) -> Result<(), String> {
    if let Err(e) = fs::create_dir_all(config_dir).await {
        return Err(format!(
            "Failed to create config dir {:?}: {}",
            config_dir, e
        ));
    }

    let dirs = [
        "modes",
        "subagents",
        "toolbox_commands",
        "code_lens",
        "knowledge",
        "trajectories",
        "tasks",
    ];
    for dir in &dirs {
        let dir_path = config_dir.join(dir);
        if let Err(e) = fs::create_dir_all(&dir_path).await {
            warn!("Failed to create directory {:?}: {}", dir_path, e);
        }
    }

    let checksums_path = config_dir.join(CHECKSUM_FILE);
    let existing_checksums = load_checksums(&checksums_path).await;
    let mut new_checksums: HashMap<String, String> = HashMap::new();

    for kind in &["modes", "subagents", "toolbox_commands", "code_lens"] {
        for (filename, content) in get_defaults_for_kind(kind) {
            let target_path = config_dir.join(kind).join(&filename);
            let checksum_key = format!("{}/{}", kind, filename);
            write_default_if_unchanged(
                &target_path,
                &checksum_key,
                &content,
                &existing_checksums,
                &mut new_checksums,
            )
            .await;
        }
    }

    remove_retired_default(
        &config_dir.join("subagents").join("buddy_humor.yaml"),
        "subagents/buddy_humor.yaml",
        &existing_checksums,
    )
    .await;

    save_checksums(&checksums_path, &new_checksums).await;

    info!("Global configs created/updated in {:?}", config_dir);
    Ok(())
}

pub async fn project_configs_ensure_dirs(project_root: &Path) -> Result<(), String> {
    let refact_dir = project_root.join(".refact");

    if !project_root.exists() {
        return Err("Project root does not exist".to_string());
    }

    let dirs = [
        "modes",
        "subagents",
        "toolbox_commands",
        "code_lens",
        "knowledge",
        "trajectories",
        "tasks",
    ];
    for dir in &dirs {
        let dir_path = refact_dir.join(dir);
        if let Err(e) = fs::create_dir_all(&dir_path).await {
            warn!("Failed to create directory {:?}: {}", dir_path, e);
        }
    }

    Ok(())
}

async fn load_checksums(path: &Path) -> HashMap<String, String> {
    if !path.exists() {
        return HashMap::new();
    }
    match fs::read_to_string(path).await {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

async fn save_checksums(path: &Path, checksums: &HashMap<String, String>) {
    if let Ok(content) = serde_yaml::to_string(checksums) {
        let _ = fs::write(path, content).await;
    }
}

pub fn compute_checksum(content: &str) -> String {
    format!("{:x}", md5::compute(content.as_bytes()))
}

fn extract_schema_version(content: &str) -> u32 {
    serde_yaml::from_str::<SchemaVersionOnly>(content)
        .map(|v| v.schema_version)
        .unwrap_or(1)
}

fn is_effectively_empty(content: &str) -> bool {
    content.trim().is_empty()
}

async fn write_default_if_unchanged(
    path: &Path,
    checksum_key: &str,
    content: &str,
    existing_checksums: &HashMap<String, String>,
    new_checksums: &mut HashMap<String, String>,
) {
    let new_checksum = compute_checksum(content);
    let default_version = extract_schema_version(content);

    if path.exists() {
        let existing_content = match fs::read_to_string(path).await {
            Ok(c) => c,
            Err(_) => return,
        };

        if is_effectively_empty(&existing_content) {
            info!(
                "Healing empty config file {:?} with embedded default",
                path.file_name().unwrap_or_default()
            );
            if fs::write(path, content).await.is_ok() {
                new_checksums.insert(checksum_key.to_string(), new_checksum);
            }
            return;
        }

        let existing_file_checksum = compute_checksum(&existing_content);
        let existing_version = extract_schema_version(&existing_content);

        if default_version > existing_version {
            info!(
                "Upgrading config {:?} from v{} to v{}",
                path.file_name().unwrap_or_default(),
                existing_version,
                default_version
            );
            if fs::write(path, content).await.is_ok() {
                new_checksums.insert(checksum_key.to_string(), new_checksum);
            } else {
                warn!("Failed to upgrade {:?}", path);
                if let Some(old) = existing_checksums.get(checksum_key) {
                    new_checksums.insert(checksum_key.to_string(), old.clone());
                }
            }
            return;
        }

        if default_version == existing_version {
            let is_user_modified = match existing_checksums.get(checksum_key) {
                Some(old_default_checksum) => &existing_file_checksum != old_default_checksum,
                None => true,
            };

            if is_user_modified {
                new_checksums.insert(checksum_key.to_string(), existing_file_checksum);
                return;
            }

            if fs::write(path, content).await.is_ok() {
                new_checksums.insert(checksum_key.to_string(), new_checksum);
            } else {
                warn!("Failed to write {:?}", path);
                new_checksums.insert(checksum_key.to_string(), existing_file_checksum);
            }
            return;
        }

        new_checksums.insert(checksum_key.to_string(), existing_file_checksum);
        return;
    }

    if fs::write(path, content).await.is_ok() {
        new_checksums.insert(checksum_key.to_string(), new_checksum);
    } else {
        warn!("Failed to write {:?}", path);
    }
}

async fn remove_retired_default(
    path: &Path,
    checksum_key: &str,
    existing_checksums: &HashMap<String, String>,
) {
    if !path.exists() {
        return;
    }
    let Some(old_default_checksum) = existing_checksums.get(checksum_key) else {
        return;
    };
    let Ok(existing_content) = fs::read_to_string(path).await else {
        return;
    };
    if compute_checksum(&existing_content) == *old_default_checksum {
        let _ = fs::remove_file(path).await;
    }
}

fn get_defaults_for_kind(kind: &str) -> Vec<(String, String)> {
    let prefix = format!("{}/", kind);
    DefaultConfigs::iter()
        .filter(|path| {
            path.starts_with(&prefix)
                && (path.ends_with(".yaml") || path.ends_with(".yml"))
                && !path.ends_with(".example")
                && !path.contains(".yaml.example")
        })
        .filter_map(|path| {
            let filename = path.strip_prefix(&prefix)?.to_string();
            if filename.contains('/') {
                return None;
            }
            let content = DefaultConfigs::get(&path)?;
            let content_str = std::str::from_utf8(content.data.as_ref()).ok()?;
            Some((filename, content_str.to_string()))
        })
        .collect()
}

pub fn get_default_checksum(kind: &str, filename: &str) -> Option<String> {
    let path = format!("{}/{}", kind, filename);
    let file = DefaultConfigs::get(&path)?;
    let content = std::str::from_utf8(file.data.as_ref()).ok()?;
    Some(compute_checksum(content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    #[tokio::test]
    async fn test_bootstrap_creates_root_dir() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("fresh_config");
        assert!(!config_dir.exists());
        let result = global_configs_try_create_all(&config_dir).await;
        assert!(
            result.is_ok(),
            "bootstrap should create root dir: {:?}",
            result
        );
        assert!(config_dir.exists());
        assert!(config_dir.join("modes").exists());
        assert!(config_dir.join("subagents").exists());
        assert!(config_dir.join("toolbox_commands").exists());
        assert!(config_dir.join("code_lens").exists());
    }

    #[tokio::test]
    async fn test_bootstrap_heals_empty_files() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();

        global_configs_try_create_all(config_dir).await.unwrap();

        let bugs_path = config_dir.join("toolbox_commands").join("bugs.yaml");
        assert!(bugs_path.exists(), "bugs.yaml should exist after bootstrap");
        fs::write(&bugs_path, "").await.unwrap();
        assert_eq!(fs::metadata(&bugs_path).await.unwrap().len(), 0);

        let explain_path = config_dir.join("toolbox_commands").join("explain.yaml");
        fs::write(&explain_path, "   \n  \t\n").await.unwrap();

        global_configs_try_create_all(config_dir).await.unwrap();

        let bugs_content = fs::read_to_string(&bugs_path).await.unwrap();
        assert!(
            !bugs_content.trim().is_empty(),
            "bugs.yaml should be healed"
        );
        assert!(
            bugs_content.contains("schema_version"),
            "healed file should have schema_version"
        );

        let explain_content = fs::read_to_string(&explain_path).await.unwrap();
        assert!(
            !explain_content.trim().is_empty(),
            "explain.yaml should be healed"
        );
    }

    #[tokio::test]
    async fn test_bootstrap_checksums_use_relative_keys() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let checksums_path = config_dir.join(CHECKSUM_FILE);
        let checksums_content = fs::read_to_string(&checksums_path).await.unwrap();
        let checksums: HashMap<String, String> = serde_yaml::from_str(&checksums_content).unwrap();

        for key in checksums.keys() {
            assert!(
                !key.starts_with('/') && !key.contains(":\\"),
                "checksum key should be relative, got: {}",
                key
            );
            let parts: Vec<&str> = key.splitn(2, '/').collect();
            assert_eq!(parts.len(), 2, "key should have exactly one slash: {}", key);
            assert!(
                ["modes", "subagents", "toolbox_commands", "code_lens"].contains(&parts[0]),
                "key kind should be valid: {}",
                key
            );
        }
    }

    #[tokio::test]
    async fn test_bootstrap_checksum_not_advanced_on_write_failure() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let checksums_path = config_dir.join(CHECKSUM_FILE);
        let checksums_content = fs::read_to_string(&checksums_path).await.unwrap();
        let checksums: HashMap<String, String> = serde_yaml::from_str(&checksums_content).unwrap();

        for key in checksums.keys() {
            let file_path = config_dir.join(key);
            assert!(
                file_path.exists(),
                "checksum entry {} exists but file {:?} does not",
                key,
                file_path
            );
        }
    }

    #[tokio::test]
    async fn test_bootstrap_removes_unmodified_retired_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        let path = config_dir.join("subagents").join("buddy_humor.yaml");
        fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        let content = "schema_version: 2\nid: buddy_humor\n";
        fs::write(&path, content).await.unwrap();
        let checksums_path = config_dir.join(CHECKSUM_FILE);
        let checksums = HashMap::from([(
            "subagents/buddy_humor.yaml".to_string(),
            compute_checksum(content),
        )]);
        save_checksums(&checksums_path, &checksums).await;

        global_configs_try_create_all(config_dir).await.unwrap();

        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_bootstrap_preserves_user_modified_files() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let agent_path = config_dir.join("modes").join("agent.yaml");
        let original = fs::read_to_string(&agent_path).await.unwrap();
        let modified = format!("{}\n# user comment\n", original);
        fs::write(&agent_path, &modified).await.unwrap();

        global_configs_try_create_all(config_dir).await.unwrap();

        let after = fs::read_to_string(&agent_path).await.unwrap();
        assert!(
            after.contains("# user comment"),
            "user modification should be preserved"
        );
    }

    #[tokio::test]
    async fn test_bootstrap_upgrades_stale_buddy_mode_default() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let stale_buddy_mode = r#"schema_version: 15
id: buddy
tools:
  - buddy_say
  - buddy_render_controls
"#;
        let buddy_path = config_dir.join("modes").join("buddy.yaml");
        fs::write(&buddy_path, stale_buddy_mode).await.unwrap();
        let checksums_path = config_dir.join(CHECKSUM_FILE);
        let checksums = HashMap::from([(
            "modes/buddy.yaml".to_string(),
            compute_checksum(stale_buddy_mode),
        )]);
        save_checksums(&checksums_path, &checksums).await;

        global_configs_try_create_all(config_dir).await.unwrap();

        let upgraded = fs::read_to_string(&buddy_path).await.unwrap();
        assert!(upgraded.contains("schema_version: 17"));
        assert!(upgraded.contains("  - buddy_speak"));
        assert!(upgraded.contains("  - buddy_runtime_event"));
        assert!(upgraded.contains("  - buddy_log_activity"));
    }

    #[test]
    fn default_modes_with_update_plan_guidance_require_schema_19() {
        for (filename, content) in get_defaults_for_kind("modes") {
            if !content.contains("update_plan") {
                continue;
            }
            let config: crate::customization_types::ModeConfig = serde_yaml::from_str(&content)
                .unwrap_or_else(|err| panic!("{filename} should parse: {err}"));
            assert!(
                config.schema_version >= 19,
                "{filename} contains update_plan guidance but has schema_version {}",
                config.schema_version
            );
        }
    }

    #[test]
    fn default_modes_with_update_goal_guidance_require_schema_20() {
        for (filename, content) in get_defaults_for_kind("modes") {
            if !content.contains("update_goal") {
                continue;
            }
            let config: crate::customization_types::ModeConfig = serde_yaml::from_str(&content)
                .unwrap_or_else(|err| panic!("{filename} should parse: {err}"));
            assert!(
                config.schema_version >= 20,
                "{filename} contains update_goal guidance but has schema_version {}",
                config.schema_version
            );
        }
    }

    #[test]
    fn read_only_modes_with_cat_include_codegraph_tools_and_guidance() {
        const CODEGRAPH_TOOLS: [&str; 9] = [
            "codegraph_overview",
            "code_health",
            "git_risk",
            "code_why",
            "code_duplication",
            "dead_code",
            "security_scan",
            "pr_blast",
            "code_map",
        ];

        for (filename, content) in get_defaults_for_kind("modes") {
            let config: crate::customization_types::ModeConfig = serde_yaml::from_str(&content)
                .unwrap_or_else(|err| panic!("{filename} should parse: {err}"));
            let is_read_only = config.ui.tags.iter().any(|tag| tag == "read-only");
            if !is_read_only || !config.tools.iter().any(|tool| tool == "cat") {
                continue;
            }

            for tool in CODEGRAPH_TOOLS {
                assert!(
                    config.tools.iter().any(|configured| configured == tool),
                    "{filename} is read-only and has cat, so it must also include {tool}"
                );
            }
            assert!(
                config.prompt.contains("%CODEGRAPH_INSTRUCTIONS%"),
                "{filename} is read-only and has cat, so its prompt must include CodeGraph guidance"
            );
        }
    }

    #[test]
    fn openai_agent_overlay_inherits_base_codegraph_tools() {
        const CODEGRAPH_TOOLS: [&str; 9] = [
            "codegraph_overview",
            "code_health",
            "git_risk",
            "code_why",
            "code_duplication",
            "dead_code",
            "security_scan",
            "pr_blast",
            "code_map",
        ];
        let defaults = get_defaults_for_kind("modes");
        let parse = |wanted: &str| {
            let (_, content) = defaults
                .iter()
                .find(|(filename, _)| filename == wanted)
                .unwrap_or_else(|| panic!("missing embedded mode {wanted}"));
            serde_yaml::from_str::<crate::customization_types::ModeConfig>(content)
                .unwrap_or_else(|err| panic!("{wanted} should parse: {err}"))
        };
        let agent = parse("agent.yaml");
        let openai = parse("openai_agent.yaml");
        let resolved = agent.apply_override(
            openai
                .override_config
                .as_ref()
                .expect("openai_agent should define an override"),
        );

        for tool in CODEGRAPH_TOOLS {
            assert!(
                resolved.tools.iter().any(|configured| configured == tool),
                "openai_agent should inherit {tool} from agent"
            );
        }
    }
}
