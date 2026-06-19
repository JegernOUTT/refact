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
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token: None,
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
            scheduler: SchedulerConfig::default(),
        }
    }
}

fn default_port() -> u16 {
    8488
}

fn default_bind() -> String {
    "0.0.0.0".to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn daemon_config_missing_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = load_from_path(&dir.path().join("daemon.yaml"))
            .await
            .unwrap();
        assert_eq!(config, DaemonConfig::default());
    }

    #[tokio::test]
    async fn daemon_config_parses_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.yaml");
        tokio::fs::write(
            &path,
            "port: 9999\nbind: 127.0.0.1\nidle_timeout_secs: 5\nauth:\n  enabled: true\n  token: secret\nscheduler:\n  enabled: false\n",
        )
        .await
        .unwrap();
        let config = load_from_path(&path).await.unwrap();
        assert_eq!(config.port, 9999);
        assert_eq!(config.bind, "127.0.0.1");
        assert_eq!(config.idle_timeout_secs, 5);
        assert_eq!(config.auth.enabled, true);
        assert_eq!(config.auth.token.as_deref(), Some("secret"));
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
}
