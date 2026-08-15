use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::time::Duration;
use tracing::error;

use crate::files_correction::{any_glob_matches_path, canonical_path, get_project_dirs};
use crate::global_context::GlobalContext;

pub use refact_core::privacy_types::{FilePrivacyLevel, FilePrivacySettings, PrivacySettings};
pub use refact_privacy::{PolicyLoad, PrivacyPolicy};

const PRIVACY_TOO_OLD: Duration = Duration::from_secs(3);

fn legacy_settings(policy: &PrivacyPolicy, loaded_ts: u64) -> PrivacySettings {
    let only_send_to_servers_i_control = policy
        .zones
        .iter()
        .filter(|zone| zone.name == "only_send_to_servers_i_control")
        .flat_map(|zone| zone.patterns.iter().cloned())
        .collect();
    PrivacySettings {
        privacy_rules: FilePrivacySettings {
            blocked: policy.blocked.clone(),
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

fn get_file_privacy_level(privacy_settings: Arc<PrivacySettings>, path: &Path) -> FilePrivacyLevel {
    if any_glob_matches_path(&privacy_settings.privacy_rules.blocked, path) {
        FilePrivacyLevel::Blocked
    } else if any_glob_matches_path(
        &privacy_settings
            .privacy_rules
            .only_send_to_servers_I_control,
        path,
    ) {
        FilePrivacyLevel::OnlySendToServersIControl
    } else {
        FilePrivacyLevel::AllowToSendAnywhere
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
