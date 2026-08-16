use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde_yaml::{Mapping, Value};
use tokio::io::AsyncWriteExt;
use tokio::time::Duration;
use tracing::error;
use uuid::Uuid;

use crate::files_correction::{canonical_path, get_project_dirs};
use crate::global_context::GlobalContext;

pub mod destinations;
pub mod heuristic;
pub mod records;

pub use refact_core::privacy_types::{FilePrivacyLevel, FilePrivacySettings, PrivacySettings};
pub use refact_privacy::{PolicyLoad, PrivacyPolicy};

const PRIVACY_TOO_OLD: Duration = Duration::from_secs(3);

fn legacy_settings(policy: &PrivacyPolicy, loaded_ts: u64) -> PrivacySettings {
    let mut blocked = policy.blocked.clone();
    let mut only_send_to_servers_i_control = Vec::new();
    for zone in &policy.zones {
        if zone.send_to.is_empty() {
            blocked.extend(zone.patterns.iter().cloned());
        } else if zone.send_to.as_slice() != ["*"] {
            only_send_to_servers_i_control.extend(zone.patterns.iter().cloned());
        }
    }
    PrivacySettings {
        privacy_rules: FilePrivacySettings {
            blocked,
            only_send_to_servers_I_control: only_send_to_servers_i_control,
        },
        loaded_ts,
    }
}

fn project_privacy_paths(project_dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    project_dirs
        .into_iter()
        .map(|project_dir| project_dir.join(".refact").join("privacy.yaml"))
        .collect()
}

fn global_privacy_path(gcx: &GlobalContext) -> PathBuf {
    if gcx.cmdline.privacy_yaml.is_empty() {
        gcx.config_dir.join("privacy.yaml")
    } else {
        canonical_path(gcx.cmdline.privacy_yaml.clone())
    }
}

pub async fn load_privacy_if_needed(gcx: Arc<GlobalContext>) -> Arc<PrivacySettings> {
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (config_dir, privacy_yaml, previous_settings, previous_load) = {
        let previous_settings = gcx.privacy_settings.read().unwrap().clone();
        let should_reload = previous_settings.loaded_ts + PRIVACY_TOO_OLD.as_secs() <= current_time;
        if !should_reload {
            return previous_settings;
        }
        (
            gcx.config_dir.clone(),
            gcx.cmdline.privacy_yaml.clone(),
            previous_settings,
            gcx.privacy_policy_load.read().unwrap().clone(),
        )
    };

    let global_path = if privacy_yaml.is_empty() {
        config_dir.join("privacy.yaml")
    } else {
        canonical_path(privacy_yaml)
    };
    let project_paths = project_privacy_paths(get_project_dirs(gcx.clone()).await);
    let loaded =
        refact_privacy::load_policy(&global_path, &project_paths, Some(&previous_load)).await;

    if let Some(error_message) = &loaded.error {
        error!("{error_message}");
        *gcx.privacy_policy_load.write().unwrap() = loaded;
        return previous_settings;
    }

    let new_privacy_settings = Arc::new(legacy_settings(&loaded.policy, current_time));
    *gcx.privacy_policy_load.write().unwrap() = loaded;
    *gcx.privacy_settings.write().unwrap() = new_privacy_settings.clone();
    new_privacy_settings
}

pub async fn save_privacy_policy(
    gcx: Arc<GlobalContext>,
    policy: PrivacyPolicy,
) -> Result<(), String> {
    policy
        .compile()
        .map_err(|error| format!("invalid privacy policy: {error}"))?;
    let path = global_privacy_path(&gcx);
    let existing = match tokio::fs::read_to_string(&path).await {
        Ok(content) => serde_yaml::from_str::<Value>(&content)
            .map_err(|error| format!("failed to parse {}: {error}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Value::Mapping(Mapping::new())
        }
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    let mut document = existing
        .as_mapping()
        .cloned()
        .ok_or_else(|| format!("{} must contain a YAML mapping", path.display()))?;
    document.insert(
        Value::String("privacy_rules".to_string()),
        serde_yaml::to_value(&policy)
            .map_err(|error| format!("failed to serialize privacy policy: {error}"))?,
    );
    let content = serde_yaml::to_string(&Value::Mapping(document))
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    write_privacy_file(&path, &content).await?;

    let previous_load = gcx.privacy_policy_load.read().unwrap().clone();
    let project_paths = project_privacy_paths(get_project_dirs(gcx.clone()).await);
    let loaded = refact_privacy::load_policy(&path, &project_paths, Some(&previous_load)).await;
    if loaded.error.is_some() {
        *gcx.privacy_policy_load.write().unwrap() = loaded;
        return Ok(());
    }
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_secs();
    let settings = Arc::new(legacy_settings(&loaded.policy, current_time));
    *gcx.privacy_policy_load.write().unwrap() = loaded;
    *gcx.privacy_settings.write().unwrap() = settings;
    Ok(())
}

async fn write_privacy_file(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("privacy.yaml");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await
            .map_err(|error| format!("failed to create {}: {error}", temporary.display()))?;
        file.write_all(content.as_bytes())
            .await
            .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
        file.sync_all()
            .await
            .map_err(|error| format!("failed to sync {}: {error}", temporary.display()))?;
        drop(file);
        #[cfg(target_os = "windows")]
        if path.exists() {
            tokio::fs::remove_file(path)
                .await
                .map_err(|error| format!("failed to replace {}: {error}", path.display()))?;
        }
        tokio::fs::rename(&temporary, path)
            .await
            .map_err(|error| format!("failed to replace {}: {error}", path.display()))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

fn legacy_patterns(patterns: &[String]) -> Vec<String> {
    let mut expanded = Vec::with_capacity(patterns.len() * 2);
    for pattern in patterns {
        expanded.push(pattern.clone());
        if !pattern
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | ']'))
        {
            expanded.push(format!("*{pattern}"));
        }
    }
    expanded
}

fn legacy_policy(privacy_settings: &PrivacySettings) -> PrivacyPolicy {
    let mut policy = refact_privacy::migrate_legacy(refact_privacy::LegacyPrivacyPolicy {
        blocked: legacy_patterns(&privacy_settings.privacy_rules.blocked),
        only_send_to_servers_i_control: legacy_patterns(
            &privacy_settings
                .privacy_rules
                .only_send_to_servers_I_control,
        ),
    });
    if let Some(zone) = policy
        .zones
        .iter_mut()
        .find(|zone| zone.name == "only_send_to_servers_i_control")
    {
        zone.send_to.push("legacy_controlled_server".to_string());
    }
    policy
}

fn get_file_privacy_level(privacy_settings: Arc<PrivacySettings>, path: &Path) -> FilePrivacyLevel {
    let policy = match legacy_policy(&privacy_settings).compile() {
        Ok(policy) => policy,
        Err(_) => return FilePrivacyLevel::Blocked,
    };
    let zone = policy.zone_for_path(path);
    if zone.name == "blocked" {
        FilePrivacyLevel::Blocked
    } else if zone.send_to.len() == 1 && zone.send_to[0] == "*" {
        FilePrivacyLevel::AllowToSendAnywhere
    } else if !zone.send_to.is_empty() {
        FilePrivacyLevel::OnlySendToServersIControl
    } else {
        FilePrivacyLevel::Blocked
    }
}

pub fn check_file_privacy(
    privacy_settings: Arc<PrivacySettings>,
    path: &Path,
    min_allowed_privacy_level: &FilePrivacyLevel,
) -> Result<(), String> {
    let file_privacy_level = get_file_privacy_level(privacy_settings.clone(), path);
    if file_privacy_level < *min_allowed_privacy_level {
        return Err(format!("privacy level {:?}", file_privacy_level));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, sync::Arc};

    #[test]
    fn test_privacy_patterns() {
        // Arrange
        let privacy_settings = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: vec![
                    "*.pem".to_string(),
                    "*/semi_private_dir/*.md".to_string(),
                ],
                blocked: vec![
                    "*.pem".to_string(),
                    "*/secret_dir/*".to_string(),
                    "secret_passwords.txt".to_string(),
                ],
            },
            loaded_ts: 0,
        });

        let current_dir = std::env::current_dir().unwrap();

        let cases: Vec<(PathBuf, FilePrivacyLevel)> = vec![
            (current_dir.join("secret.pem"), FilePrivacyLevel::Blocked), // matches both
            (
                current_dir.join("somedir/secret.pem"),
                FilePrivacyLevel::Blocked,
            ), // matches both
            (
                current_dir.join("secret.pub"),
                FilePrivacyLevel::AllowToSendAnywhere,
            ),
            (
                current_dir.join("secret_passwords.txt"),
                FilePrivacyLevel::Blocked,
            ),
            (
                current_dir.join("3/2/1/secret_passwords.txt"),
                FilePrivacyLevel::Blocked,
            ),
            (
                current_dir.join("secret_passwords.jpeg"),
                FilePrivacyLevel::AllowToSendAnywhere,
            ),
            (
                current_dir.join("secret_dir/anything.jpg"),
                FilePrivacyLevel::Blocked,
            ),
            (
                current_dir.join("semi_private_dir/wow1.md"),
                FilePrivacyLevel::OnlySendToServersIControl,
            ),
            (
                current_dir.join("semi_private_dir/wow1.jpeg"),
                FilePrivacyLevel::AllowToSendAnywhere,
            ),
            (
                current_dir.join("1/2/3/semi_private_dir/wow1.md"),
                FilePrivacyLevel::OnlySendToServersIControl,
            ),
            (
                current_dir.join("1/2/3/semi_private_dir/4/5/6/wow1.md"),
                FilePrivacyLevel::OnlySendToServersIControl,
            ),
            (
                current_dir.join("wow1.md"),
                FilePrivacyLevel::AllowToSendAnywhere,
            ),
        ];

        for (path, expected_privacy_level) in cases {
            let actual_privacy_level = get_file_privacy_level(privacy_settings.clone(), &path);
            assert_eq!(
                actual_privacy_level,
                expected_privacy_level,
                "Testing get_file_privacy_level with path {} and expected privacy level {:?}, got {:?}",
                path.display(),
                expected_privacy_level,
                actual_privacy_level,
            );
        }
    }

    #[test]
    fn test_privacy_minimum() {
        let privacy_settings = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: vec![
                    "*.cat.txt".to_string(),
                    "*.md".to_string(),
                    "*/.venv/*".to_string(),
                    "**/tests_dir/**/*".to_string(),
                ],
                blocked: vec!["*/make.png".to_string(), "*.txt".to_string()],
            },
            loaded_ts: 0,
        });

        let current_dir = std::env::current_dir().unwrap();

        let cases: Vec<(PathBuf, FilePrivacyLevel, bool)> = vec![
            (
                current_dir.join("test.zip"),
                FilePrivacyLevel::AllowToSendAnywhere,
                true,
            ),
            (
                current_dir.join("test.md"),
                FilePrivacyLevel::AllowToSendAnywhere,
                false,
            ),
            (
                current_dir.join("test.md"),
                FilePrivacyLevel::OnlySendToServersIControl,
                true,
            ),
            (
                current_dir.join("test.cat.txt"),
                FilePrivacyLevel::OnlySendToServersIControl,
                false,
            ),
        ];

        for (path, expected_privacy_level, expected_result) in &cases {
            let result = check_file_privacy(privacy_settings.clone(), path, expected_privacy_level);
            if *expected_result {
                assert!(
                    result.is_ok(),
                    "Testing check_file_privacy with path {} and expected privacy level {:?}, got {:?} and it should have been ok",
                    path.display(),
                    expected_privacy_level,
                    result.unwrap_err(),
                );
            } else {
                assert!(
                    result.is_err(),
                    "Testing check_file_privacy with path {} and expected privacy level {:?}, got {:?} and it should have been err",
                    path.display(),
                    expected_privacy_level,
                    result.unwrap(),
                );
            }
        }
    }
}
