use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::scheduler::SchedulerConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token: None,
            username: None,
            password: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    Wake,
    Agent,
}

impl Default for HookKind {
    fn default() -> Self {
        Self::Wake
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookMapping {
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub kind: HookKind,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub deliver: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub mappings: HashMap<String, HookMapping>,
    #[serde(default)]
    pub default_project: Option<String>,
    #[serde(default)]
    pub allowed_projects: Option<Vec<String>>,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token: None,
            mappings: HashMap::new(),
            default_project: None,
            allowed_projects: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdnsConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
}

impl Default for MdnsConfig {
    fn default() -> Self {
        Self { enabled: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub mdns: MdnsConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            bind: default_bind(),
            idle_timeout_secs: default_idle_timeout_secs(),
            auth: AuthConfig::default(),
            hooks: HooksConfig::default(),
            mdns: MdnsConfig::default(),
            scheduler: SchedulerConfig::default(),
        }
    }
}

fn default_port() -> u16 {
    8488
}

pub const DAEMON_PORT_ENV: &str = "REFACT_DAEMON_PORT";

pub fn resolve_port_override(
    cli_port: Option<u16>,
    env_value: Option<&str>,
) -> Result<Option<u16>, String> {
    if cli_port.is_some() {
        return Ok(cli_port);
    }
    let Some(raw) = env_value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    raw.parse::<u16>().map(Some).map_err(|_| {
        format!(
            "invalid {DAEMON_PORT_ENV} value `{raw}`: expected an integer between 0 and 65535"
        )
    })
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_idle_timeout_secs() -> u64 {
    1800
}

pub async fn load() -> Result<DaemonConfig, String> {
    load_from_path(&crate::daemon::paths::daemon_config_path()).await
}

pub async fn load_from_path(path: &Path) -> Result<DaemonConfig, String> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => serde_yaml::from_str(&content)
            .map_err(|error| format!("failed to parse {}: {error}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(DaemonConfig::default()),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

pub async fn save_to_path(config: &DaemonConfig, path: &Path) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let yaml = serde_yaml::to_string(config)
        .map_err(|error| format!("failed to serialize daemon config: {error}"))?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let tmp = path.with_extension("yaml.tmp");
    let _ = tokio::fs::remove_file(&tmp).await;
    #[cfg(unix)]
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .await
        .map_err(|error| format!("failed to write {}: {error}", tmp.display()))?;
    #[cfg(not(unix))]
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .await
        .map_err(|error| format!("failed to write {}: {error}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|error| format!("failed to chmod {}: {error}", tmp.display()))?;
    }
    file.write_all(yaml.as_bytes())
        .await
        .map_err(|error| format!("failed to write daemon config: {error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("failed to flush {}: {error}", tmp.display()))?;
    drop(file);
    #[cfg(windows)]
    {
        let _ = tokio::fs::remove_file(path).await;
    }
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|error| format!("failed to publish {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_to_path_round_trips_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.yaml");
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("tok".to_string()),
                username: Some("alice".to_string()),
                password: Some("hunter2".to_string()),
            },
            ..DaemonConfig::default()
        };
        save_to_path(&config, &path).await.unwrap();
        let loaded = load_from_path(&path).await.unwrap();
        assert_eq!(loaded, config);
        assert!(!dir.path().join("daemon.yaml.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[tokio::test]
    async fn daemon_config_missing_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = load_from_path(&dir.path().join("daemon.yaml"))
            .await
            .unwrap();
        assert_eq!(config, DaemonConfig::default());
    }

    #[test]
    fn daemon_config_defaults_to_loopback_with_auth_opt_in_and_mdns_auto() {
        let config = DaemonConfig::default();

        assert_eq!(config.bind, "127.0.0.1");
        assert!(!config.auth.enabled);
        assert_eq!(config.mdns.enabled, None);
    }

    #[tokio::test]
    async fn daemon_config_parses_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.yaml");
        tokio::fs::write(
            &path,
            "port: 9999\nbind: 0.0.0.0\nidle_timeout_secs: 5\nauth:\n  enabled: true\n  token: secret\nmdns:\n  enabled: false\nscheduler:\n  enabled: false\n",
        )
        .await
        .unwrap();
        let config = load_from_path(&path).await.unwrap();
        assert_eq!(config.port, 9999);
        assert_eq!(config.bind, "0.0.0.0");
        assert_eq!(config.idle_timeout_secs, 5);
        assert_eq!(config.auth.enabled, true);
        assert_eq!(config.auth.token.as_deref(), Some("secret"));
        assert_eq!(config.mdns.enabled, Some(false));
        assert!(!config.scheduler.enabled);
    }

    #[tokio::test]
    async fn daemon_config_parses_hooks_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.yaml");
        tokio::fs::write(
            &path,
            "hooks:\n  enabled: true\n  token: hook-secret\n  default_project: demo\n  allowed_projects: [demo, other]\n  mappings:\n    deploy:\n      project: demo\n      kind: agent\n      mode: agent\n      model: test-model\n      deliver:\n        type: chat\n",
        )
        .await
        .unwrap();

        let config = load_from_path(&path).await.unwrap();

        assert!(config.hooks.enabled);
        assert_eq!(config.hooks.token.as_deref(), Some("hook-secret"));
        assert_eq!(config.hooks.default_project.as_deref(), Some("demo"));
        assert_eq!(
            config.hooks.allowed_projects.as_deref().unwrap(),
            ["demo".to_string(), "other".to_string()]
        );
        let mapping = config.hooks.mappings.get("deploy").unwrap();
        assert_eq!(mapping.kind, HookKind::Agent);
        assert_eq!(mapping.project.as_deref(), Some("demo"));
        assert_eq!(mapping.mode.as_deref(), Some("agent"));
        assert_eq!(mapping.model.as_deref(), Some("test-model"));
        assert_eq!(mapping.deliver.as_ref().unwrap()["type"], "chat");
    }

    #[test]
    fn hooks_config_defaults_disabled() {
        let config = DaemonConfig::default();

        assert!(!config.hooks.enabled);
        assert!(config.hooks.token.is_none());
        assert!(config.hooks.mappings.is_empty());
        assert!(config.hooks.default_project.is_none());
        assert!(config.hooks.allowed_projects.is_none());
    }

    #[test]
    fn port_override_prefers_cli_over_env() {
        assert_eq!(
            resolve_port_override(Some(9000), Some("8490")).unwrap(),
            Some(9000)
        );
        assert_eq!(
            resolve_port_override(Some(9000), Some("banana")).unwrap(),
            Some(9000)
        );
    }

    #[test]
    fn port_override_reads_env_when_no_cli_flag() {
        assert_eq!(
            resolve_port_override(None, Some("8490")).unwrap(),
            Some(8490)
        );
        assert_eq!(
            resolve_port_override(None, Some(" 0 ")).unwrap(),
            Some(0)
        );
    }

    #[test]
    fn port_override_ignores_absent_or_blank_env() {
        assert_eq!(resolve_port_override(None, None).unwrap(), None);
        assert_eq!(resolve_port_override(None, Some("")).unwrap(), None);
        assert_eq!(resolve_port_override(None, Some("   ")).unwrap(), None);
    }

    #[test]
    fn port_override_rejects_invalid_env() {
        let error = resolve_port_override(None, Some("banana")).unwrap_err();
        assert!(error.contains("REFACT_DAEMON_PORT"));
        assert!(error.contains("banana"));

        let error = resolve_port_override(None, Some("70000")).unwrap_err();
        assert!(error.contains("70000"));
    }
}
