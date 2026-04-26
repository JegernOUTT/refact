use std::collections::HashMap;
use std::sync::Arc;
use axum::Extension;
use axum::response::Json;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::mcp::mcp_naming;

const EXPORT_VERSION: u32 = 1;

fn is_secret_field(key: &str) -> bool {
    let key_lower = key.to_lowercase();
    key_lower.contains("token")
        || key_lower.contains("secret")
        || key_lower.contains("key")
        || key_lower.contains("password")
}

#[cfg(test)]
fn redact_env(env: &HashMap<String, String>) -> HashMap<String, String> {
    env.iter()
        .map(|(k, v)| {
            let redacted = if is_secret_field(k) {
                "<REDACTED>".to_string()
            } else {
                v.clone()
            };
            (k.clone(), redacted)
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedServer {
    pub config_name: String,
    pub transport: String,
    pub config: HashMap<String, Value>,
    #[serde(default)]
    pub tools_config: HashMap<String, Value>,
    #[serde(default)]
    pub confirmation: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBundle {
    pub version: u32,
    pub exported_at: String,
    pub servers: Vec<ExportedServer>,
}

fn parse_yaml_config(content: &str) -> HashMap<String, Value> {
    match serde_yaml::from_str::<HashMap<String, Value>>(content) {
        Ok(map) => map,
        Err(e) => {
            tracing::warn!("Failed to parse YAML config: {}", e);
            HashMap::new()
        }
    }
}

async fn collect_mcp_yaml_files(
    integrations_dir: &std::path::Path,
) -> Vec<(String, String, String)> {
    let mut result = Vec::new();
    let mut rd = match tokio::fs::read_dir(integrations_dir).await {
        Ok(rd) => rd,
        Err(_) => return result,
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy().to_string();
        if !fname_str.ends_with(".yaml") {
            continue;
        }
        let is_mcp = fname_str.starts_with("mcp_stdio_")
            || fname_str.starts_with("mcp_sse_")
            || fname_str.starts_with("mcp_http_");
        if !is_mcp {
            continue;
        }
        let config_name = fname_str.trim_end_matches(".yaml").to_string();
        let path_str = entry.path().to_string_lossy().to_string();
        let content = match tokio::fs::read_to_string(entry.path()).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Failed to read config file {}: {}",
                    entry.path().display(),
                    e
                );
                continue;
            }
        };
        result.push((config_name, path_str, content));
    }
    result
}

#[derive(Deserialize)]
pub struct ExportRequest {
    #[serde(default)]
    pub config_paths: Vec<String>,
    #[serde(default)]
    pub include_secrets: bool,
}

pub async fn handle_v1_mcp_export(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<ExportRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    let config_dir = gcx.read().await.config_dir.clone();
    let integrations_dir = config_dir.join("integrations.d");

    let all_files = collect_mcp_yaml_files(&integrations_dir).await;

    let filter: Option<std::collections::HashSet<String>> = if req.config_paths.is_empty() {
        None
    } else {
        Some(req.config_paths.iter().cloned().collect())
    };

    let mut servers = Vec::new();
    for (config_name, _path, content) in &all_files {
        if let Some(ref f) = filter {
            let yaml_name = format!("{}.yaml", config_name);
            if !f.contains(config_name.as_str()) && !f.contains(yaml_name.as_str()) {
                continue;
            }
        }

        let mut parsed = parse_yaml_config(content);
        let transport = mcp_naming::detect_transport(config_name);

        if !req.include_secrets {
            if let Some(Value::Object(env_map)) = parsed.get_mut("env") {
                for (k, v) in env_map.iter_mut() {
                    if is_secret_field(k) {
                        *v = Value::String("<REDACTED>".to_string());
                    }
                }
            }
            for (k, v) in parsed.iter_mut() {
                if is_secret_field(k) {
                    *v = Value::String("<REDACTED>".to_string());
                }
            }
        }

        let tools_config: HashMap<String, Value> = parsed
            .remove("tools")
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().collect())
            .unwrap_or_default();

        let confirmation: HashMap<String, Value> = parsed
            .remove("confirmation")
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().collect())
            .unwrap_or_default();

        servers.push(ExportedServer {
            config_name: config_name.clone(),
            transport,
            config: parsed,
            tools_config,
            confirmation,
        });
    }

    let bundle = ExportBundle {
        version: EXPORT_VERSION,
        exported_at: chrono::Utc::now().to_rfc3339(),
        servers,
    };

    Ok(Json(serde_json::to_value(&bundle).unwrap()))
}

#[derive(Deserialize)]
pub struct ImportRequest {
    pub bundle: ExportBundle,
    #[serde(default)]
    pub overwrite_existing: bool,
    #[serde(default)]
    pub secrets: HashMap<String, HashMap<String, String>>,
}

fn build_yaml_from_config(
    config: &HashMap<String, Value>,
    tools_config: &HashMap<String, Value>,
    confirmation: &HashMap<String, Value>,
) -> String {
    let mut full: serde_json::Map<String, Value> = serde_json::Map::new();
    for (k, v) in config {
        full.insert(k.clone(), v.clone());
    }
    if !tools_config.is_empty() {
        full.insert(
            "tools".to_string(),
            Value::Object(
                tools_config
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
        );
    }
    if !confirmation.is_empty() {
        full.insert(
            "confirmation".to_string(),
            Value::Object(
                confirmation
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
        );
    }
    let val = Value::Object(full);
    serde_yaml::to_string(&val).unwrap_or_default()
}

fn apply_secrets_to_config(config: &mut HashMap<String, Value>, secrets: &HashMap<String, String>) {
    for (field_path, secret_val) in secrets {
        if let Some(rest) = field_path.strip_prefix("env.") {
            if let Some(Value::Object(env_map)) = config.get_mut("env") {
                env_map.insert(rest.to_string(), Value::String(secret_val.clone()));
            }
        } else {
            config.insert(field_path.clone(), Value::String(secret_val.clone()));
        }
    }
}

pub async fn handle_v1_mcp_import(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let req = serde_json::from_slice::<ImportRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    let config_dir = gcx.read().await.config_dir.clone();
    let integrations_dir = config_dir.join("integrations.d");
    tokio::fs::create_dir_all(&integrations_dir)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cannot create integrations dir: {}", e),
            )
        })?;

    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let mut errors: Vec<Value> = Vec::new();

    for server in &req.bundle.servers {
        if let Err(e) = mcp_naming::validate_config_filename(&server.config_name) {
            errors.push(json!({ "config_name": server.config_name, "error": e }));
            continue;
        }
        let prefix = mcp_naming::config_prefix_for_transport(&server.transport);
        let config_name = if server.config_name.starts_with("mcp_stdio_")
            || server.config_name.starts_with("mcp_sse_")
            || server.config_name.starts_with("mcp_http_")
        {
            server.config_name.clone()
        } else {
            format!("{}{}", prefix, server.config_name)
        };
        if let Err(e) = mcp_naming::validate_config_filename(&config_name) {
            errors.push(json!({ "config_name": server.config_name, "error": e }));
            continue;
        }

        let filename = format!("{}.yaml", config_name);
        let config_path = integrations_dir.join(&filename);

        if config_path.exists() && !req.overwrite_existing {
            skipped.push(json!({ "config_name": config_name, "reason": "already exists" }));
            continue;
        }

        let mut config = server.config.clone();
        if let Some(server_secrets) = req
            .secrets
            .get(&config_name)
            .or_else(|| req.secrets.get(&server.config_name))
        {
            apply_secrets_to_config(&mut config, server_secrets);
        }

        let yaml_content =
            build_yaml_from_config(&config, &server.tools_config, &server.confirmation);
        let tmp_path = config_path.with_extension("yaml.tmp");
        if let Err(e) = tokio::fs::write(&tmp_path, &yaml_content).await {
            errors.push(json!({ "config_name": config_name, "error": e.to_string() }));
            continue;
        }
        if let Err(e) = tokio::fs::rename(&tmp_path, &config_path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            errors.push(json!({ "config_name": config_name, "error": e.to_string() }));
            continue;
        }

        imported.push(
            json!({ "config_name": config_name, "config_path": config_path.display().to_string() }),
        );
    }

    Ok(Json(json!({
        "imported": imported,
        "skipped": skipped,
        "errors": errors,
    })))
}

pub async fn handle_v1_mcp_project_config(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Json<Value>, ScratchError> {
    let workspace_folders = {
        let gcx_locked = gcx.read().await;
        let folders = gcx_locked
            .documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .clone();
        folders
    };

    let mut project_configs: Vec<Value> = Vec::new();
    for folder in &workspace_folders {
        let config_path = folder.join(".refact").join("mcp-servers.json");
        if !config_path.exists() {
            continue;
        }
        let content = match tokio::fs::read_to_string(&config_path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Failed to read config file {}: {}",
                    config_path.display(),
                    e
                );
                continue;
            }
        };
        let bundle: ExportBundle = match serde_json::from_str(&content) {
            Ok(b) => b,
            Err(e) => {
                project_configs.push(json!({
                    "project_dir": folder.display().to_string(),
                    "error": format!("failed to parse mcp-servers.json: {}", e),
                }));
                continue;
            }
        };

        let config_dir = gcx.read().await.config_dir.clone();
        let integrations_dir = config_dir.join("integrations.d");

        let mut missing_servers = Vec::new();
        for server in &bundle.servers {
            if mcp_naming::validate_config_filename(&server.config_name).is_err() {
                missing_servers.push(server.config_name.clone());
                continue;
            }
            let prefix = mcp_naming::config_prefix_for_transport(&server.transport);
            let config_name = if server.config_name.starts_with("mcp_stdio_")
                || server.config_name.starts_with("mcp_sse_")
                || server.config_name.starts_with("mcp_http_")
            {
                server.config_name.clone()
            } else {
                format!("{}{}", prefix, server.config_name)
            };
            if mcp_naming::validate_config_filename(&config_name).is_err() {
                missing_servers.push(server.config_name.clone());
                continue;
            }
            let config_path = integrations_dir.join(format!("{}.yaml", config_name));
            if !config_path.exists() {
                missing_servers.push(server.config_name.clone());
            }
        }

        project_configs.push(json!({
            "project_dir": folder.display().to_string(),
            "config_path": config_path.display().to_string(),
            "server_count": bundle.servers.len(),
            "missing_servers": missing_servers,
        }));
    }

    Ok(Json(json!({ "project_configs": project_configs })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_config_name_rejects_traversal() {
        assert!(mcp_naming::validate_config_filename("../evil").is_err());
        assert!(mcp_naming::validate_config_filename("foo/../../bar").is_err());
        assert!(mcp_naming::validate_config_filename("mcp_stdio_ok").is_ok());
        assert!(mcp_naming::validate_config_filename("").is_err());
        assert!(mcp_naming::validate_config_filename("/etc/passwd").is_err());
        assert!(mcp_naming::validate_config_filename("a\\b").is_err());
        assert!(mcp_naming::validate_config_filename("mcp_http_my-server").is_ok());
    }

    #[test]
    fn test_is_secret_field_detects_secrets() {
        assert!(is_secret_field("GITHUB_PERSONAL_ACCESS_TOKEN"));
        assert!(is_secret_field("API_KEY"));
        assert!(is_secret_field("client_secret"));
        assert!(is_secret_field("db_password"));
        assert!(is_secret_field("bearer_token"));
    }

    #[test]
    fn test_is_secret_field_allows_safe_fields() {
        assert!(!is_secret_field("command"));
        assert!(!is_secret_field("init_timeout"));
        assert!(!is_secret_field("DEBUG_LEVEL"));
    }

    #[test]
    fn test_redact_env_redacts_secrets_only() {
        let mut env = HashMap::new();
        env.insert("GITHUB_TOKEN".to_string(), "ghp_real_token".to_string());
        env.insert("DEBUG".to_string(), "true".to_string());
        env.insert("API_KEY".to_string(), "secret123".to_string());

        let redacted = redact_env(&env);
        assert_eq!(redacted["GITHUB_TOKEN"], "<REDACTED>");
        assert_eq!(redacted["DEBUG"], "true");
        assert_eq!(redacted["API_KEY"], "<REDACTED>");
    }

    #[test]
    fn test_determine_transport_stdio() {
        assert_eq!(mcp_naming::detect_transport("mcp_stdio_github"), "stdio");
        assert_eq!(
            mcp_naming::detect_transport("mcp_stdio_brave_search"),
            "stdio"
        );
    }

    #[test]
    fn test_determine_transport_sse() {
        assert_eq!(mcp_naming::detect_transport("mcp_sse_myserver"), "sse");
    }

    #[test]
    fn test_determine_transport_http() {
        assert_eq!(mcp_naming::detect_transport("mcp_http_myserver"), "http");
    }

    #[test]
    fn test_config_prefix_for_transport() {
        assert_eq!(
            mcp_naming::config_prefix_for_transport("stdio"),
            "mcp_stdio_"
        );
        assert_eq!(mcp_naming::config_prefix_for_transport("sse"), "mcp_sse_");
        assert_eq!(mcp_naming::config_prefix_for_transport("http"), "mcp_http_");
        assert_eq!(
            mcp_naming::config_prefix_for_transport("streamable-http"),
            "mcp_http_"
        );
        assert_eq!(
            mcp_naming::config_prefix_for_transport("unknown"),
            "mcp_stdio_"
        );
    }

    #[test]
    fn test_export_bundle_roundtrip() {
        let bundle = ExportBundle {
            version: 1,
            exported_at: "2026-01-01T00:00:00Z".to_string(),
            servers: vec![ExportedServer {
                config_name: "mcp_stdio_github".to_string(),
                transport: "stdio".to_string(),
                config: {
                    let mut m = HashMap::new();
                    m.insert(
                        "command".to_string(),
                        Value::String("npx github".to_string()),
                    );
                    m
                },
                tools_config: HashMap::new(),
                confirmation: HashMap::new(),
            }],
        };
        let json_str = serde_json::to_string(&bundle).unwrap();
        let parsed: ExportBundle = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers[0].config_name, "mcp_stdio_github");
    }

    #[test]
    fn test_build_yaml_from_config_basic() {
        let mut config = HashMap::new();
        config.insert("command".to_string(), Value::String("npx test".to_string()));
        config.insert(
            "env".to_string(),
            Value::Object({
                let mut m = serde_json::Map::new();
                m.insert("TOKEN".to_string(), Value::String("abc".to_string()));
                m
            }),
        );
        let tools_config: HashMap<String, Value> = HashMap::new();
        let confirmation: HashMap<String, Value> = HashMap::new();
        let yaml = build_yaml_from_config(&config, &tools_config, &confirmation);
        assert!(
            yaml.contains("command") || yaml.contains("npx test"),
            "yaml must contain command"
        );
    }

    #[test]
    fn test_apply_secrets_to_config_env() {
        let mut config: HashMap<String, Value> = HashMap::new();
        config.insert(
            "env".to_string(),
            Value::Object({
                let mut m = serde_json::Map::new();
                m.insert(
                    "GITHUB_TOKEN".to_string(),
                    Value::String("<REDACTED>".to_string()),
                );
                m
            }),
        );

        let mut secrets = HashMap::new();
        secrets.insert("env.GITHUB_TOKEN".to_string(), "ghp_real".to_string());
        apply_secrets_to_config(&mut config, &secrets);

        if let Some(Value::Object(env_map)) = config.get("env") {
            assert_eq!(
                env_map["GITHUB_TOKEN"],
                Value::String("ghp_real".to_string())
            );
        } else {
            panic!("env should be present");
        }
    }

    #[tokio::test]
    async fn test_import_creates_files() {
        let tmp = tempfile::tempdir().unwrap();
        let integrations_dir = tmp.path().join("integrations.d");
        tokio::fs::create_dir_all(&integrations_dir).await.unwrap();

        let server = ExportedServer {
            config_name: "mcp_stdio_testserver".to_string(),
            transport: "stdio".to_string(),
            config: {
                let mut m = HashMap::new();
                m.insert("command".to_string(), Value::String("npx test".to_string()));
                m.insert("init_timeout".to_string(), Value::String("60".to_string()));
                m
            },
            tools_config: HashMap::new(),
            confirmation: HashMap::new(),
        };

        let yaml =
            build_yaml_from_config(&server.config, &server.tools_config, &server.confirmation);
        let config_path = integrations_dir.join("mcp_stdio_testserver.yaml");
        let tmp_path = config_path.with_extension("yaml.tmp");
        tokio::fs::write(&tmp_path, &yaml).await.unwrap();
        tokio::fs::rename(&tmp_path, &config_path).await.unwrap();

        assert!(config_path.exists(), "config file must be created");
        let content = tokio::fs::read_to_string(&config_path).await.unwrap();
        assert!(
            content.contains("npx test"),
            "yaml content must contain the command"
        );
    }

    #[tokio::test]
    async fn test_export_collects_mcp_yaml_files() {
        let tmp = tempfile::tempdir().unwrap();
        let integrations_dir = tmp.path().join("integrations.d");
        tokio::fs::create_dir_all(&integrations_dir).await.unwrap();

        tokio::fs::write(
            integrations_dir.join("mcp_stdio_github.yaml"),
            "command: \"npx github\"\nenv:\n  GITHUB_TOKEN: \"ghp_test\"\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            integrations_dir.join("not_mcp_integration.yaml"),
            "some: config\n",
        )
        .await
        .unwrap();

        let files = collect_mcp_yaml_files(&integrations_dir).await;
        assert_eq!(files.len(), 1, "must find exactly 1 MCP yaml file");
        assert_eq!(files[0].0, "mcp_stdio_github", "config_name must match");
        assert!(files[0].2.contains("npx github"), "content must be read");
    }

    #[test]
    fn test_secrets_not_redacted_when_include_secrets_true() {
        let mut env = HashMap::new();
        env.insert("GITHUB_TOKEN".to_string(), "ghp_real_token".to_string());
        let redacted = redact_env(&env);
        let not_redacted: HashMap<String, String> =
            env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        assert_eq!(
            redacted["GITHUB_TOKEN"], "<REDACTED>",
            "redact_env always redacts"
        );
        assert_eq!(
            not_redacted["GITHUB_TOKEN"], "ghp_real_token",
            "original not modified"
        );
    }
}
