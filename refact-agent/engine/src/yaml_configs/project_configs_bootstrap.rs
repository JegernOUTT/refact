use rust_embed::RustEmbed;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use tracing::{info, warn};

const CHECKSUM_FILE: &str = "default-checksums.yaml";

#[derive(RustEmbed)]
#[folder = "src/yaml_configs/defaults/"]
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
    if !config_dir.exists() {
        return Err("Config dir does not exist".to_string());
    }

    let dirs = ["modes", "subagents", "toolbox_commands", "code_lens", "knowledge", "trajectories", "tasks"];
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
            write_default_if_unchanged(&target_path, &content, &existing_checksums, &mut new_checksums).await;
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

    let dirs = ["modes", "subagents", "toolbox_commands", "code_lens", "knowledge", "trajectories", "tasks"];
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

async fn write_default_if_unchanged(
    path: &Path,
    content: &str,
    existing_checksums: &HashMap<String, String>,
    new_checksums: &mut HashMap<String, String>,
) {
    let path_str = path.to_string_lossy().to_string();
    let new_checksum = compute_checksum(content);
    let default_version = extract_schema_version(content);

    new_checksums.insert(path_str.clone(), new_checksum.clone());

    if path.exists() {
        let existing_content = match fs::read_to_string(path).await {
            Ok(c) => c,
            Err(_) => return,
        };

        let existing_file_checksum = compute_checksum(&existing_content);
        let existing_version = extract_schema_version(&existing_content);

        // Version precedence: always upgrade if default is newer
        if default_version > existing_version {
            info!(
                "Upgrading config {:?} from v{} to v{}",
                path.file_name().unwrap_or_default(),
                existing_version,
                default_version
            );
            if let Err(e) = fs::write(path, content).await {
                warn!("Failed to write {:?}: {}", path, e);
            }
            return;
        }

        // If versions are equal, use checksums to detect user modifications
        if default_version == existing_version {
            let is_user_modified = match existing_checksums.get(&path_str) {
                Some(old_default_checksum) => &existing_file_checksum != old_default_checksum,
                None => true,
            };

            if is_user_modified {
                return; // User modified, preserve their changes
            }

            // Checksum matches, update to new default of same version
            if let Err(e) = fs::write(path, content).await {
                warn!("Failed to write {:?}: {}", path, e);
            }
            return;
        }

        // default_version < existing_version: keep existing (user has newer version)
        return;
    }

    // File doesn't exist, write it
    if let Err(e) = fs::write(path, content).await {
        warn!("Failed to write {:?}: {}", path, e);
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
