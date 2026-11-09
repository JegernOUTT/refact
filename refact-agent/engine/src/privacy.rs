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

pub mod classification_cache;
pub mod destinations;
pub mod heuristic;
pub mod records;

pub use refact_core::privacy_types::{FilePrivacyLevel, FilePrivacySettings, PrivacySettings};
pub use refact_privacy::{PolicyLoad, PrivacyPolicy};

const PRIVACY_TOO_OLD: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct PrivacyMatchCounts {
    pub counts: std::collections::BTreeMap<String, usize>,
    pub fingerprint: u64,
    pub computed_at: Option<std::time::Instant>,
    pub computing: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivacyObservationRuntimeState {
    pub runtime_available: bool,
    pub last_error: Option<String>,
}

pub fn record_observation_status(gcx: &GlobalContext, status: &refact_exec::ObservationStatus) {
    let state = match status {
        refact_exec::ObservationStatus::Unavailable(reason) => PrivacyObservationRuntimeState {
            runtime_available: false,
            last_error: Some(reason.clone()),
        },
        refact_exec::ObservationStatus::Observed(_)
        | refact_exec::ObservationStatus::Pending(_)
        | refact_exec::ObservationStatus::Incomplete(_) => PrivacyObservationRuntimeState {
            runtime_available: true,
            last_error: None,
        },
    };
    *gcx.privacy_observation_runtime.write().unwrap() = state;
}

pub async fn warn_observation_degraded_once(gcx: Arc<GlobalContext>, reason: &str) {
    crate::buddy::observers::privacy_degraded::warn_once(gcx, reason).await;
}

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
pub async fn global_only_policy(gcx: Arc<GlobalContext>) -> Option<refact_privacy::PrivacyPolicy> {
    let path = global_privacy_path(&gcx);
    let content = tokio::fs::read_to_string(&path).await.ok()?;
    refact_privacy::parse_policy_yaml(&content).ok()
}

pub async fn load_privacy_if_needed(gcx: Arc<GlobalContext>) -> Arc<PrivacySettings> {
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (config_dir, privacy_yaml, previous_load) = {
        let previous_settings = gcx.privacy_settings.read().unwrap().clone();
        let should_reload = previous_settings.loaded_ts + PRIVACY_TOO_OLD.as_secs() <= current_time;
        if !should_reload {
            return previous_settings;
        }
        (
            gcx.config_dir.clone(),
            gcx.cmdline.privacy_yaml.clone(),
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
    }
    install_policy_load(&gcx, loaded, current_time)
}

fn install_policy_load(
    gcx: &GlobalContext,
    mut loaded: PolicyLoad,
    current_time: u64,
) -> Arc<PrivacySettings> {
    let mut current = gcx.privacy_policy_load.write().unwrap();
    if loaded.error.is_some() {
        loaded.policy = current.policy.clone();
        *current = loaded;
        return gcx.privacy_settings.read().unwrap().clone();
    }
    let policy_changed = current.policy != loaded.policy;
    if !policy_changed {
        loaded.policy = current.policy.clone();
    }
    let new_privacy_settings = Arc::new(legacy_settings(&loaded.policy, current_time));
    *current = loaded;
    *gcx.privacy_settings.write().unwrap() = new_privacy_settings.clone();
    if policy_changed {
        gcx.tool_catalog_generations.advance_privacy();
    }
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
    let mut document = match tokio::fs::read_to_string(&path).await {
        Ok(content) => match serde_yaml::from_str::<Value>(&content) {
            Ok(Value::Mapping(document)) => document,
            Ok(_) | Err(_) => {
                backup_corrupt_privacy_file(&path, &content).await?;
                Mapping::new()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Mapping::new(),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
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
        install_policy_load(&gcx, loaded, 0);
        return Ok(());
    }
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_secs();
    install_policy_load(&gcx, loaded, current_time);
    Ok(())
}

async fn backup_corrupt_privacy_file(path: &Path, content: &str) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("privacy.yaml");
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_millis();
    let backup = parent.join(format!(
        "{file_name}.corrupt-{timestamp}-{}",
        Uuid::new_v4()
    ));
    write_privacy_file(&backup, content).await?;
    Ok(backup)
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

#[cfg(test)]
thread_local! {
    pub(crate) static LEGACY_PRIVACY_COMPILE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) struct LegacyPrivacyMatcher {
    policy: Result<refact_privacy::CompiledPolicy, String>,
}

impl LegacyPrivacyMatcher {
    pub(crate) fn new(privacy_settings: &PrivacySettings) -> Self {
        #[cfg(test)]
        LEGACY_PRIVACY_COMPILE_COUNT.with(|count| count.set(count.get() + 1));
        Self {
            policy: legacy_policy(privacy_settings)
                .compile()
                .map_err(|error| error.to_string()),
        }
    }

    fn level(&self, path: &Path) -> FilePrivacyLevel {
        match &self.policy {
            Ok(policy) => file_privacy_level_with_policy(policy, path),
            Err(_) => FilePrivacyLevel::Blocked,
        }
    }

    pub(crate) fn check(
        &self,
        path: &Path,
        min_allowed_privacy_level: &FilePrivacyLevel,
    ) -> Result<(), String> {
        let file_privacy_level = self.level(path);
        if file_privacy_level < *min_allowed_privacy_level {
            return Err(format!("privacy level {:?}", file_privacy_level));
        }
        Ok(())
    }

    pub(crate) fn check_for_read_preparation(&self, path: &Path) -> Result<(), String> {
        let file_privacy_level = self.level(path);
        if file_privacy_level == FilePrivacyLevel::Blocked {
            return Err(format!("privacy level {:?}", file_privacy_level));
        }
        Ok(())
    }
}

pub fn get_file_privacy_level(
    privacy_settings: Arc<PrivacySettings>,
    path: &Path,
) -> FilePrivacyLevel {
    LegacyPrivacyMatcher::new(&privacy_settings).level(path)
}

fn file_privacy_level_with_policy(
    policy: &refact_privacy::CompiledPolicy,
    path: &Path,
) -> FilePrivacyLevel {
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
    LegacyPrivacyMatcher::new(&privacy_settings).check(path, min_allowed_privacy_level)
}

pub(crate) fn check_file_privacy_for_read_preparation(
    privacy_settings: Arc<PrivacySettings>,
    path: &Path,
) -> Result<(), String> {
    LegacyPrivacyMatcher::new(&privacy_settings).check_for_read_preparation(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, sync::Arc};

    #[tokio::test]
    async fn equal_refresh_and_save_preserve_installed_policy_identity() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        save_privacy_policy(gcx.clone(), PrivacyPolicy::default())
            .await
            .unwrap();
        let installed = gcx.privacy_policy_load.read().unwrap().policy.clone();
        let generation = gcx
            .tool_catalog_generations
            .privacy
            .load(std::sync::atomic::Ordering::SeqCst);
        *gcx.privacy_settings.write().unwrap() = Arc::new(legacy_settings(&installed, 0));
        load_privacy_if_needed(gcx.clone()).await;
        assert!(Arc::ptr_eq(
            &installed,
            &gcx.privacy_policy_load.read().unwrap().policy
        ));
        save_privacy_policy(gcx.clone(), installed.as_ref().clone())
            .await
            .unwrap();
        assert!(Arc::ptr_eq(
            &installed,
            &gcx.privacy_policy_load.read().unwrap().policy
        ));
        assert_eq!(
            gcx.tool_catalog_generations
                .privacy
                .load(std::sync::atomic::Ordering::SeqCst),
            generation
        );
    }

    #[tokio::test]
    async fn policy_install_compares_current_state_and_preserves_failed_load_diagnostics() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let first = gcx.privacy_policy_load.read().unwrap().clone();
        let generation = gcx
            .tool_catalog_generations
            .privacy
            .load(std::sync::atomic::Ordering::SeqCst);
        let changed = PolicyLoad {
            policy: Arc::new(PrivacyPolicy {
                blocked: vec!["private.txt".into()],
                ..Default::default()
            }),
            error: None,
            source_paths: vec!["current.yaml".into()],
        };
        install_policy_load(&gcx, changed.clone(), 10);
        let same_content = PolicyLoad {
            policy: Arc::new(changed.policy.as_ref().clone()),
            source_paths: vec!["refreshed.yaml".into()],
            ..changed.clone()
        };
        let settings = install_policy_load(&gcx, same_content, 11);
        {
            let current = gcx.privacy_policy_load.read().unwrap();
            assert!(Arc::ptr_eq(&changed.policy, &current.policy));
            assert_eq!(current.source_paths, vec![PathBuf::from("refreshed.yaml")]);
        }
        let failed = PolicyLoad {
            error: Some("failed to read stale.yaml".into()),
            source_paths: vec!["stale.yaml".into()],
            ..first.clone()
        };
        assert!(Arc::ptr_eq(
            &settings,
            &install_policy_load(&gcx, failed, 12)
        ));
        {
            let current = gcx.privacy_policy_load.read().unwrap();
            assert!(Arc::ptr_eq(&changed.policy, &current.policy));
            assert_eq!(current.error.as_deref(), Some("failed to read stale.yaml"));
            assert_eq!(current.source_paths, vec![PathBuf::from("stale.yaml")]);
        }
        install_policy_load(
            &gcx,
            PolicyLoad {
                policy: Arc::new(first.policy.as_ref().clone()),
                ..first.clone()
            },
            13,
        );
        let returned = gcx.privacy_policy_load.read().unwrap();
        assert_eq!(returned.policy, first.policy);
        assert!(!Arc::ptr_eq(&returned.policy, &first.policy));
        assert!(!Arc::ptr_eq(&returned.policy, &changed.policy));
        assert!(returned.error.is_none());
        assert_eq!(returned.source_paths, first.source_paths);
        assert_eq!(
            gcx.tool_catalog_generations
                .privacy
                .load(std::sync::atomic::Ordering::SeqCst),
            generation + 2
        );
    }

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

    #[test]
    fn read_preparation_allows_controlled_but_legacy_check_stays_all_or_nothing() {
        let privacy_settings = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: vec!["*.controlled".to_string()],
                blocked: vec!["*.blocked".to_string()],
            },
            loaded_ts: 0,
        });
        let controlled = PathBuf::from("context.controlled");
        let blocked = PathBuf::from("context.blocked");

        assert!(
            check_file_privacy_for_read_preparation(privacy_settings.clone(), &controlled,).is_ok()
        );
        assert!(
            check_file_privacy_for_read_preparation(privacy_settings.clone(), &blocked,).is_err()
        );

        assert!(check_file_privacy(
            privacy_settings.clone(),
            &controlled,
            &FilePrivacyLevel::AllowToSendAnywhere,
        )
        .is_err());
        assert!(check_file_privacy(
            privacy_settings,
            &controlled,
            &FilePrivacyLevel::OnlySendToServersIControl,
        )
        .is_ok());
    }
}
