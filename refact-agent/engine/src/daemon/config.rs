use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

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
pub struct DaemonConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    #[serde(default)]
    pub auth: AuthConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            bind: default_bind(),
            idle_timeout_secs: default_idle_timeout_secs(),
            auth: AuthConfig::default(),
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
            "port: 9999\nbind: 127.0.0.1\nidle_timeout_secs: 5\nauth:\n  enabled: true\n  token: secret\n",
        )
        .await
        .unwrap();
        let config = load_from_path(&path).await.unwrap();
        assert_eq!(config.port, 9999);
        assert_eq!(config.bind, "127.0.0.1");
        assert_eq!(config.idle_timeout_secs, 5);
        assert_eq!(config.auth.enabled, true);
        assert_eq!(config.auth.token.as_deref(), Some("secret"));
    }
}
