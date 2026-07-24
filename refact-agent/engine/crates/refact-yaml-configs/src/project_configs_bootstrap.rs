use rust_embed::RustEmbed;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use tracing::{info, warn};

const CHECKSUM_FILE: &str = "default-checksums.yaml";
const BOOTSTRAP_KINDS: [&str; 4] = ["modes", "subagents", "toolbox_commands", "code_lens"];

fn is_bootstrap_owned_key(key: &str) -> bool {
    key.split_once('/')
        .map(|(kind, _)| BOOTSTRAP_KINDS.contains(&kind))
        .unwrap_or(false)
}

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

    for kind in &BOOTSTRAP_KINDS {
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
        &mut new_checksums,
    )
    .await;

    for (key, value) in &existing_checksums {
        if !is_bootstrap_owned_key(key) {
            new_checksums
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }

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

fn carry_forward_baseline(
    checksum_key: &str,
    existing_checksums: &HashMap<String, String>,
    new_checksums: &mut HashMap<String, String>,
) {
    if let Some(old_default_checksum) = existing_checksums.get(checksum_key) {
        new_checksums.insert(checksum_key.to_string(), old_default_checksum.clone());
    }
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

    if !path.exists() {
        if fs::write(path, content).await.is_ok() {
            new_checksums.insert(checksum_key.to_string(), new_checksum);
        } else {
            warn!("Failed to write {:?}", path);
        }
        return;
    }

    let existing_content = match fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => {
            carry_forward_baseline(checksum_key, existing_checksums, new_checksums);
            return;
        }
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

    if existing_file_checksum == new_checksum {
        new_checksums.insert(checksum_key.to_string(), new_checksum);
        return;
    }

    // The recorded baseline must always be the checksum of a shipped default, never of
    // user-authored content. A file counts as user-modified when it differs from the
    // default baseline it was created from.
    let matches_baseline = existing_checksums
        .get(checksum_key)
        .map(|baseline| baseline == &existing_file_checksum)
        .unwrap_or(false);

    if !matches_baseline {
        if default_version > existing_version {
            info!(
                "Preserving user-modified config {:?} (file v{}, embedded default v{})",
                path.file_name().unwrap_or_default(),
                existing_version,
                default_version
            );
        }
        carry_forward_baseline(checksum_key, existing_checksums, new_checksums);
        return;
    }

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
            new_checksums.insert(checksum_key.to_string(), existing_file_checksum);
        }
        return;
    }

    if default_version < existing_version {
        carry_forward_baseline(checksum_key, existing_checksums, new_checksums);
        return;
    }

    // Same schema_version, file matches the recorded baseline, yet differs from the embedded
    // default. Correct baselines always equal a shipped default's checksum and shipped content
    // changes come with a schema_version bump, so this signature means the baseline was
    // poisoned with a user-content checksum by pre-fix builds. Preserve the file and drop the
    // unverifiable baseline so the file is treated as user content from now on.
    info!(
        "Preserving config {:?}; dropping unverifiable default baseline",
        path.file_name().unwrap_or_default()
    );
}

async fn remove_retired_default(
    path: &Path,
    checksum_key: &str,
    existing_checksums: &HashMap<String, String>,
    new_checksums: &mut HashMap<String, String>,
) {
    if !path.exists() {
        return;
    }
    let Some(old_default_checksum) = existing_checksums.get(checksum_key) else {
        return;
    };
    let Ok(existing_content) = fs::read_to_string(path).await else {
        new_checksums.insert(checksum_key.to_string(), old_default_checksum.clone());
        return;
    };
    if compute_checksum(&existing_content) != *old_default_checksum {
        return;
    }
    if fs::remove_file(path).await.is_err() {
        warn!("Failed to remove retired default {:?}", path);
        new_checksums.insert(checksum_key.to_string(), old_default_checksum.clone());
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
    #[tokio::test]
    async fn test_bootstrap_preserves_user_modified_files_across_repeated_runs() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let agent_path = config_dir.join("modes").join("agent.yaml");
        let original = fs::read_to_string(&agent_path).await.unwrap();
        let modified = format!("{}\n# user removed a tool\n", original);
        fs::write(&agent_path, &modified).await.unwrap();
        let default_checksum = compute_checksum(&original);

        for run in 1..=3 {
            global_configs_try_create_all(config_dir).await.unwrap();

            let after = fs::read_to_string(&agent_path).await.unwrap();
            assert_eq!(
                after, modified,
                "user modification must survive bootstrap run {}",
                run
            );

            let checksums_content = fs::read_to_string(&config_dir.join(CHECKSUM_FILE))
                .await
                .unwrap();
            let saved: HashMap<String, String> = serde_yaml::from_str(&checksums_content).unwrap();
            assert_eq!(
                saved.get("modes/agent.yaml"),
                Some(&default_checksum),
                "baseline must stay the shipped default checksum on run {}",
                run
            );
        }
    }

    #[tokio::test]
    async fn test_bootstrap_preserves_user_modified_file_on_schema_upgrade() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        let stale_default = "schema_version: 15\nid: buddy\ntools:\n  - say\n";
        let user_modified = "schema_version: 15\nid: buddy\ntools:\n  - say\n# user tweak\n";
        let buddy_path = config_dir.join("modes").join("buddy.yaml");
        fs::create_dir_all(buddy_path.parent().unwrap())
            .await
            .unwrap();
        fs::write(&buddy_path, user_modified).await.unwrap();
        let checksums_path = config_dir.join(CHECKSUM_FILE);
        let checksums = HashMap::from([(
            "modes/buddy.yaml".to_string(),
            compute_checksum(stale_default),
        )]);
        save_checksums(&checksums_path, &checksums).await;

        for run in 1..=2 {
            global_configs_try_create_all(config_dir).await.unwrap();

            let after = fs::read_to_string(&buddy_path).await.unwrap();
            assert_eq!(
                after, user_modified,
                "user-modified stale config must not be auto-upgraded on run {}",
                run
            );

            let checksums_content = fs::read_to_string(&checksums_path).await.unwrap();
            let saved: HashMap<String, String> = serde_yaml::from_str(&checksums_content).unwrap();
            assert_eq!(
                saved.get("modes/buddy.yaml"),
                Some(&compute_checksum(stale_default)),
                "baseline must stay the old default checksum on run {}",
                run
            );
        }
    }

    #[tokio::test]
    async fn test_bootstrap_preserves_user_modified_file_without_baseline() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let agent_path = config_dir.join("modes").join("agent.yaml");
        let original = fs::read_to_string(&agent_path).await.unwrap();
        let modified = format!("{}\n# user comment\n", original);
        fs::write(&agent_path, &modified).await.unwrap();
        fs::remove_file(config_dir.join(CHECKSUM_FILE))
            .await
            .unwrap();

        for run in 1..=2 {
            global_configs_try_create_all(config_dir).await.unwrap();
            let after = fs::read_to_string(&agent_path).await.unwrap();
            assert_eq!(
                after, modified,
                "user modification without baseline must survive run {}",
                run
            );
        }
    }

    #[tokio::test]
    async fn test_bootstrap_restores_baseline_for_pristine_file_without_checksums() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let agent_path = config_dir.join("modes").join("agent.yaml");
        let original = fs::read_to_string(&agent_path).await.unwrap();
        fs::remove_file(config_dir.join(CHECKSUM_FILE))
            .await
            .unwrap();

        global_configs_try_create_all(config_dir).await.unwrap();

        let after = fs::read_to_string(&agent_path).await.unwrap();
        assert_eq!(after, original, "pristine default must stay intact");

        let checksums_content = fs::read_to_string(&config_dir.join(CHECKSUM_FILE))
            .await
            .unwrap();
        let saved: HashMap<String, String> = serde_yaml::from_str(&checksums_content).unwrap();
        assert_eq!(
            saved.get("modes/agent.yaml"),
            Some(&compute_checksum(&original)),
            "pristine default must be re-recognized as baseline"
        );
    }
    #[tokio::test]
    async fn test_bootstrap_migrates_poisoned_baseline_from_pre_fix_builds() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let agent_path = config_dir.join("modes").join("agent.yaml");
        let original = fs::read_to_string(&agent_path).await.unwrap();
        let modified = format!("{}\n# user removed a tool\n", original);
        fs::write(&agent_path, &modified).await.unwrap();

        // Pre-fix builds recorded the user's file checksum as the "default baseline".
        let checksums_path = config_dir.join(CHECKSUM_FILE);
        let mut checksums: HashMap<String, String> =
            serde_yaml::from_str(&fs::read_to_string(&checksums_path).await.unwrap()).unwrap();
        checksums.insert("modes/agent.yaml".to_string(), compute_checksum(&modified));
        save_checksums(&checksums_path, &checksums).await;

        for run in 1..=2 {
            global_configs_try_create_all(config_dir).await.unwrap();
            let after = fs::read_to_string(&agent_path).await.unwrap();
            assert_eq!(
                after, modified,
                "user file with poisoned baseline must survive run {}",
                run
            );
        }

        let saved: HashMap<String, String> =
            serde_yaml::from_str(&fs::read_to_string(&checksums_path).await.unwrap()).unwrap();
        assert_eq!(
            saved.get("modes/agent.yaml"),
            None,
            "poisoned baseline must be dropped"
        );
    }

    #[tokio::test]
    async fn test_bootstrap_preserves_foreign_checksum_entries() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let checksums_path = config_dir.join(CHECKSUM_FILE);
        let mut checksums: HashMap<String, String> =
            serde_yaml::from_str(&fs::read_to_string(&checksums_path).await.unwrap()).unwrap();
        checksums.insert(
            "privacy.yaml".to_string(),
            "sha256-from-create-configs".to_string(),
        );
        checksums.insert(
            "integrations.d/github.yaml".to_string(),
            "owned-by-another-subsystem".to_string(),
        );
        save_checksums(&checksums_path, &checksums).await;

        global_configs_try_create_all(config_dir).await.unwrap();

        let saved: HashMap<String, String> =
            serde_yaml::from_str(&fs::read_to_string(&checksums_path).await.unwrap()).unwrap();
        assert_eq!(
            saved.get("privacy.yaml").map(|s| s.as_str()),
            Some("sha256-from-create-configs"),
            "bare-filename entries owned by create_configs must survive bootstrap"
        );
        assert_eq!(
            saved.get("integrations.d/github.yaml").map(|s| s.as_str()),
            Some("owned-by-another-subsystem"),
            "entries under foreign kinds must survive bootstrap"
        );
        assert!(
            saved.get("modes/agent.yaml").is_some(),
            "bootstrap-owned entries must still be recorded"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_bootstrap_keeps_retired_baseline_when_removal_fails() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        global_configs_try_create_all(config_dir).await.unwrap();

        let retired_content = "schema_version: 2\nid: buddy_humor\n";
        let retired_path = config_dir.join("subagents").join("buddy_humor.yaml");
        fs::write(&retired_path, retired_content).await.unwrap();
        let checksums_path = config_dir.join(CHECKSUM_FILE);
        let mut checksums: HashMap<String, String> =
            serde_yaml::from_str(&fs::read_to_string(&checksums_path).await.unwrap()).unwrap();
        checksums.insert(
            "subagents/buddy_humor.yaml".to_string(),
            compute_checksum(retired_content),
        );
        save_checksums(&checksums_path, &checksums).await;

        let subagents_dir = config_dir.join("subagents");
        let writable = std::fs::metadata(&subagents_dir).unwrap().permissions();
        let mut read_only = writable.clone();
        read_only.set_mode(0o555);
        std::fs::set_permissions(&subagents_dir, read_only).unwrap();

        global_configs_try_create_all(config_dir).await.unwrap();

        std::fs::set_permissions(&subagents_dir, writable).unwrap();

        assert!(
            retired_path.exists(),
            "retired file should still exist after failed removal"
        );
        let saved: HashMap<String, String> =
            serde_yaml::from_str(&fs::read_to_string(&checksums_path).await.unwrap()).unwrap();
        assert_eq!(
            saved.get("subagents/buddy_humor.yaml"),
            Some(&compute_checksum(retired_content)),
            "baseline must be kept so retirement can retry"
        );

        global_configs_try_create_all(config_dir).await.unwrap();
        assert!(
            !retired_path.exists(),
            "retirement must succeed once the directory is writable again"
        );
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
