use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

use refact_at_web::web_search::{
    provider_catalog, KeyRequirement, ProviderRuntimeConfig, WebSearchConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{custom_error::ScratchError, global_context::GlobalContext};

static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredProvider {
    pub enabled: bool,
    pub api_key: String,
}

impl Default for StoredProvider {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: String::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchSettings {
    pub schema_version: u32,
    pub deadline_ms: u64,
    pub provider_timeout_ms: u64,
    pub prefer_builtin_over_native: bool,
    pub searxng_url: String,
    pub providers: BTreeMap<String, StoredProvider>,
}

impl Default for WebSearchSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            deadline_ms: 12000,
            provider_timeout_ms: 6000,
            prefer_builtin_over_native: true,
            searxng_url: String::new(),
            providers: provider_catalog()
                .iter()
                .map(|p| {
                    (
                        p.id.into(),
                        StoredProvider {
                            enabled: p.default_enabled,
                            api_key: String::new(),
                        },
                    )
                })
                .collect(),
        }
    }
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

impl WebSearchSettings {
    fn runtime_with_env(&self, env: impl Fn(&str) -> Option<String>) -> WebSearchConfig {
        WebSearchConfig {
            providers: provider_catalog()
                .iter()
                .map(|info| {
                    let stored = self
                        .providers
                        .get(info.id)
                        .cloned()
                        .unwrap_or(StoredProvider {
                            enabled: info.default_enabled,
                            api_key: String::new(),
                        });
                    let api_key = nonempty(&stored.api_key).or_else(|| {
                        info.key_env_vars
                            .iter()
                            .find_map(|name| env(name).and_then(|v| nonempty(&v)))
                    });
                    (
                        info.id.to_string(),
                        ProviderRuntimeConfig {
                            enabled: stored.enabled,
                            api_key,
                        },
                    )
                })
                .collect(),
            searxng_url: nonempty(&self.searxng_url)
                .or_else(|| env("SEARXNG_URL").and_then(|v| nonempty(&v))),
            deadline: Duration::from_millis(self.deadline_ms),
            provider_timeout: Duration::from_millis(self.provider_timeout_ms),
        }
    }

    pub fn runtime_config(&self) -> WebSearchConfig {
        self.runtime_with_env(|name| std::env::var(name).ok())
    }

    pub fn response(&self) -> Value {
        self.response_with_runtime(&self.runtime_config())
    }

    fn response_with_runtime(&self, runtime: &WebSearchConfig) -> Value {
        let mut config = json!({"schema_version":1,"deadline_ms":self.deadline_ms,"provider_timeout_ms":self.provider_timeout_ms,"prefer_builtin_over_native":self.prefer_builtin_over_native,"searxng_url":self.searxng_url,"providers":{}});
        let providers: Vec<Value> = provider_catalog().iter().map(|info| {
            let stored = self.providers.get(info.id).cloned().unwrap_or_default();
            let key_present = runtime.providers.get(info.id).and_then(|p| p.api_key.as_ref()).is_some();
            let credential_status = if info.key == KeyRequirement::None { "not_applicable" } else if nonempty(&stored.api_key).is_some() { "stored" } else if key_present { "env" } else { "missing" };
            let mut entry = json!({"enabled":stored.enabled});
            if info.key != KeyRequirement::None { entry["api_key"] = json!(if nonempty(&stored.api_key).is_some() { "***" } else { "" }); }
            config["providers"][info.id] = entry;
            let active = stored.enabled && (info.key != KeyRequirement::Required || key_present) && (info.id != "searxng" || runtime.searxng_url.is_some());
            json!({"id":info.id,"display_name":info.display_name,"tier":info.tier,"category":info.category,"key":info.key,"key_env_vars":info.key_env_vars,"signup_url":info.signup_url,"description":info.description,"default_enabled":info.default_enabled,"credential_status":credential_status,"active":active})
        }).collect();
        json!({"config":config,"providers":providers})
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("schema_version must be 1".into());
        }
        if !(3000..=60000).contains(&self.deadline_ms) {
            return Err("deadline_ms must be 3000–60000".into());
        }
        if !(1000..=30000).contains(&self.provider_timeout_ms) {
            return Err("provider_timeout_ms must be 1000–30000".into());
        }
        if !self.searxng_url.is_empty() {
            let url = url::Url::parse(&self.searxng_url)
                .map_err(|_| "searxng_url must be an http(s) URL")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("searxng_url must be an http(s) URL without credentials".into());
            }
        }
        Ok(())
    }

    fn merge(&self, patch: Value) -> Result<Self, String> {
        let patch = patch.as_object().ok_or("config must be an object")?;
        let mut value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        for (key, replacement) in patch {
            if key == "providers" {
                for (id, entry) in replacement
                    .as_object()
                    .ok_or("providers must be an object")?
                {
                    let info = provider_catalog()
                        .iter()
                        .find(|p| p.id == id)
                        .ok_or_else(|| format!("unknown provider: {id}"))?;
                    for (field, replacement) in entry
                        .as_object()
                        .ok_or("provider config must be an object")?
                    {
                        match field.as_str() {
                            "enabled" if replacement.is_boolean() => {
                                value[key][id][field] = replacement.clone()
                            }
                            "api_key"
                                if info.key != KeyRequirement::None && replacement.is_string() =>
                            {
                                let secret = replacement
                                    .as_str()
                                    .ok_or("api_key must be a string")?
                                    .trim();
                                if secret == "***" {
                                    if self
                                        .providers
                                        .get(id)
                                        .and_then(|p| nonempty(&p.api_key))
                                        .is_none()
                                    {
                                        return Err(format!(
                                            "provider {id} has no stored key to preserve"
                                        ));
                                    }
                                } else {
                                    value[key][id][field] = json!(secret);
                                }
                            }
                            _ => return Err(format!("invalid provider field: {id}.{field}")),
                        }
                    }
                }
            } else if matches!(
                key.as_str(),
                "schema_version"
                    | "deadline_ms"
                    | "provider_timeout_ms"
                    | "prefer_builtin_over_native"
                    | "searxng_url"
            ) {
                value[key] = replacement.clone();
            } else {
                return Err(format!("unknown settings field: {key}"));
            }
        }
        let mut merged: Self =
            serde_json::from_value(value).map_err(|e| format!("invalid settings: {e}"))?;
        merged.searxng_url = merged.searxng_url.trim().to_string();
        merged.validate()?;
        Ok(merged)
    }
}

async fn load_from_path(path: &Path) -> WebSearchSettings {
    let raw = match tokio::fs::read(path).await {
        Ok(raw) => raw,
        Err(_) => return WebSearchSettings::default(),
    };
    let value = match serde_yaml::from_slice::<serde_yaml::Value>(&raw)
        .ok()
        .and_then(|value| value.as_mapping().cloned())
    {
        Some(value) => value,
        None => {
            tracing::warn!(
                "Cannot load web-search settings {}; using defaults",
                path.display()
            );
            return WebSearchSettings::default();
        }
    };
    let mut settings = WebSearchSettings::default();
    for (field, value) in &value {
        let Some(field) = field.as_str() else {
            continue;
        };
        if field == "providers" {
            continue;
        }
        match serde_json::to_value(value)
            .ok()
            .and_then(|value| settings.merge(json!({(field): value})).ok())
        {
            Some(merged) => settings = merged,
            None => tracing::warn!(
                "Ignoring invalid or unsupported web-search settings field; retaining its default"
            ),
        }
    }
    if let Some(providers) = value
        .get(serde_yaml::Value::String("providers".into()))
        .and_then(serde_yaml::Value::as_mapping)
    {
        for (id, entry) in providers {
            if let Some(provider) = id.as_str().and_then(|id| settings.providers.get_mut(id)) {
                if let Some(enabled) = entry.get("enabled").and_then(serde_yaml::Value::as_bool) {
                    provider.enabled = enabled;
                }
                if let Some(key) = entry.get("api_key").and_then(serde_yaml::Value::as_str) {
                    provider.api_key = key.to_string();
                }
            }
        }
    }
    settings
}

async fn backup_malformed_settings(path: &Path) -> Result<(), ScratchError> {
    use tokio::io::AsyncWriteExt;
    let server_error = |error: std::io::Error| {
        ScratchError::new(hyper::StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    };
    let raw = match tokio::fs::read(path).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(server_error(error)),
    };
    if serde_yaml::from_slice::<serde_yaml::Value>(&raw).is_ok() {
        return Ok(());
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let backup = path.with_extension(format!("yaml.bak.{}-{nonce}", std::process::id()));
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&backup).await.map_err(server_error)?;
    file.write_all(&raw).await.map_err(server_error)?;
    file.sync_all().await.map_err(server_error)?;
    tracing::warn!(
        "Backed up syntactically invalid web-search settings to {} before saving",
        backup.display()
    );
    Ok(())
}

pub async fn load_web_search_settings(gcx: Arc<GlobalContext>) -> WebSearchSettings {
    load_from_path(&gcx.config_dir.join("web-search.yaml")).await
}

pub async fn save_web_search_settings(
    gcx: Arc<GlobalContext>,
    patch: Value,
) -> Result<WebSearchSettings, ScratchError> {
    let _guard = WRITE_LOCK.lock().await;
    let path = gcx.config_dir.join("web-search.yaml");
    let settings = load_from_path(&path)
        .await
        .merge(patch)
        .map_err(|e| ScratchError::new(hyper::StatusCode::BAD_REQUEST, e))?;
    let yaml = serde_yaml::to_string(&settings)
        .map_err(|e| ScratchError::new(hyper::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    backup_malformed_settings(&path).await?;
    crate::http::routers::v1::settings_config::atomic_write(&path, &yaml).await?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_fields_preserve_valid_settings_and_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web-search.yaml");
        tokio::fs::write(&path, "? [unknown, compound]\n: ignored\nschema_version: 99\ndeadline_ms: broken\nprovider_timeout_ms: 4000\nprefer_builtin_over_native: false\nsearxng_url: 'file:///bad'\nproviders:\n  brave:\n    enabled: broken\n    api_key: retained-secret\n  exa:\n    enabled: false\n    api_key: 123\n  parallel: broken\n").await.unwrap();
        let loaded = load_from_path(&path).await;
        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.deadline_ms, 12000);
        assert_eq!(loaded.provider_timeout_ms, 4000);
        assert!(!loaded.prefer_builtin_over_native);
        assert!(loaded.searxng_url.is_empty());
        assert_eq!(loaded.providers["brave"].api_key, "retained-secret");
        assert!(!loaded.providers["exa"].enabled);
        assert!(loaded.providers["exa"].api_key.is_empty());
        assert!(loaded.providers["parallel"].enabled);
        assert_eq!(
            loaded.merge(json!({"deadline_ms":9000})).unwrap().providers["brave"].api_key,
            "retained-secret"
        );
    }

    #[tokio::test]
    async fn malformed_yaml_backup_retains_exact_bytes_and_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web-search.yaml");
        let raw = b"providers: [broken\napi_key: secret\xff";
        tokio::fs::write(&path, raw).await.unwrap();
        backup_malformed_settings(&path).await.unwrap();
        crate::http::routers::v1::settings_config::atomic_write(&path, "deadline_ms: 9000\n")
            .await
            .unwrap();
        let backup = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|entry| entry != &path)
            .unwrap();
        assert!(backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("web-search.yaml.bak."));
        assert_eq!(tokio::fs::read(&backup).await.unwrap(), raw);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        backup_malformed_settings(&path).await.unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        tokio::fs::write(&path, b"providers: [another corruption")
            .await
            .unwrap();
        backup_malformed_settings(&path).await.unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
        assert_eq!(tokio::fs::read(&backup).await.unwrap(), raw);
    }

    #[test]
    fn sparse_merge_validation_and_secret_redaction() {
        let defaults = WebSearchSettings::default();
        let stored = defaults.merge(json!({"providers":{"brave":{"api_key":"secret","enabled":false}},"deadline_ms":9000})).expect("merge");
        for patch in [
            json!({}),
            json!({"providers":{"brave":{"enabled":true}}}),
            json!({"providers":{"brave":{"api_key":"***"}}}),
        ] {
            assert_eq!(
                stored.merge(patch).expect("merge").providers["brave"].api_key,
                "secret"
            );
        }
        let runtime = stored.runtime_with_env(|_| None);
        let response = stored.response_with_runtime(&runtime);
        assert_eq!(
            response["config"]["providers"]["brave"],
            json!({"enabled":false,"api_key":"***"})
        );
        assert!(!response.to_string().contains("secret"));
        let brave = response["providers"]
            .as_array()
            .expect("providers")
            .iter()
            .find(|p| p["id"] == "brave")
            .expect("brave");
        assert_eq!(brave["credential_status"], "stored");
        assert_eq!(brave["active"], false);
        assert_eq!(
            stored
                .merge(json!({"providers":{"brave":{"api_key":""}}}))
                .expect("clear")
                .providers["brave"]
                .api_key,
            ""
        );
        for bad in [
            json!({"providers":{"brave":{"api_key":"***"}}}),
            json!({"providers":{"unknown":{}}}),
            json!({"deadline_ms":2999}),
            json!({"deadline_ms":60001}),
            json!({"provider_timeout_ms":999}),
            json!({"provider_timeout_ms":30001}),
            json!({"searxng_url":"file:///secret"}),
            json!({"searxng_url":"garbage"}),
            json!({"schema_version":2}),
            json!({"prefer_builtin_over_native":"false"}),
        ] {
            assert!(defaults.merge(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn environment_precedence_status_and_activity() {
        let defaults = WebSearchSettings::default();
        let env = |name: &str| match name {
            "BRAVE_SEARCH_API_KEY" => Some("primary".into()),
            "BRAVE_API_KEY" => Some("secondary".into()),
            "SEARXNG_URL" => Some("https://search.example".into()),
            _ => None,
        };
        let runtime = defaults.runtime_with_env(env);
        assert_eq!(
            runtime.providers["brave"].api_key.as_deref(),
            Some("primary")
        );
        assert_eq!(
            runtime.searxng_url.as_deref(),
            Some("https://search.example")
        );
        let response = defaults.response_with_runtime(&runtime);
        let providers = response["providers"].as_array().expect("providers");
        for (id, status, active) in [
            ("brave", "env", true),
            ("serper", "missing", false),
            ("searxng", "not_applicable", true),
            ("exa", "missing", true),
        ] {
            let provider = providers.iter().find(|p| p["id"] == id).expect("provider");
            assert_eq!(provider["credential_status"], status);
            assert_eq!(provider["active"], active);
        }
        assert_eq!(response["config"]["providers"]["brave"]["api_key"], "");
        assert!(response["config"]["providers"]["searxng"]
            .get("api_key")
            .is_none());
        let stored = defaults.merge(json!({"providers":{"brave":{"api_key":"stored"}},"searxng_url":"https://stored.example"})).expect("stored");
        let runtime = stored.runtime_with_env(env);
        assert_eq!(
            runtime.providers["brave"].api_key.as_deref(),
            Some("stored")
        );
        assert_eq!(
            runtime.searxng_url.as_deref(),
            Some("https://stored.example")
        );
    }

    #[tokio::test]
    async fn storage_defaults_corruption_atomic_permissions_and_unknown_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("web-search.yaml");
        let missing = load_from_path(&path).await;
        assert_eq!(missing.deadline_ms, 12000);
        assert_eq!(missing.provider_timeout_ms, 6000);
        assert!(missing.prefer_builtin_over_native);
        assert_eq!(missing.providers.len(), provider_catalog().len());
        tokio::fs::write(&path, "providers: [broken")
            .await
            .expect("write");
        assert_eq!(load_from_path(&path).await.deadline_ms, 12000);
        tokio::fs::write(&path,"deadline_ms: 8000\nproviders:\n  bogus:\n    enabled: false\n  exa:\n    enabled: false\n").await.expect("write");
        let loaded = load_from_path(&path).await;
        assert_eq!(loaded.deadline_ms, 8000);
        assert!(!loaded.providers.contains_key("bogus"));
        assert!(!loaded.providers["exa"].enabled);
        assert!(loaded.providers["parallel"].enabled);
        crate::http::routers::v1::settings_config::atomic_write(
            &path,
            &serde_yaml::to_string(&loaded).expect("yaml"),
        )
        .await
        .expect("atomic write");
        assert_eq!(load_from_path(&path).await.deadline_ms, 8000);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                tokio::fs::metadata(&path)
                    .await
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
