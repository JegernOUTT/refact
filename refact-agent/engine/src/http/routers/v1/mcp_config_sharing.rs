use axum::extract::State;
use axum::response::Json;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWriteExt;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::integrations::mcp::mcp_naming;

const EXPORT_VERSION: u32 = 1;
const REDACTED: &str = "<REDACTED>";
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn is_secret_field(key: &str) -> bool {
    let key_lower = key.to_lowercase();
    [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "client_secret",
        "authorization",
        "bearer",
        "cookie",
        "credential",
    ]
    .iter()
    .any(|needle| key_lower.contains(needle))
}

fn redact_secrets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if is_secret_field(key) && !child.is_object() && !child.is_array() {
                    *child = Value::String(REDACTED.to_string());
                } else {
                    redact_secrets(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_secrets(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
fn redact_env(env: &HashMap<String, String>) -> HashMap<String, String> {
    env.iter()
        .map(|(k, v)| {
            let redacted = if is_secret_field(k) {
                REDACTED.to_string()
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
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let gcx = app.gcx.clone();
    let req = serde_json::from_slice::<ExportRequest>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    let config_dir = gcx.config_dir.clone();
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
            for (key, value) in &mut parsed {
                if is_secret_field(key) {
                    *value = Value::String(REDACTED.to_string());
                } else {
                    redact_secrets(value);
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
    pub secrets: HashMap<String, Vec<ImportSecret>>,
}

#[derive(Deserialize)]
pub struct ImportSecret {
    pub path: Vec<String>,
    pub value: String,
}

#[cfg(test)]
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

fn apply_secrets_to_config(config: &mut HashMap<String, Value>, secrets: &[ImportSecret]) {
    for secret in secrets {
        let Some((first, rest)) = secret.path.split_first() else {
            continue;
        };
        if rest.is_empty() {
            if config.contains_key(first) {
                config.insert(first.clone(), Value::String(secret.value.clone()));
            }
            continue;
        }
        let Some(mut cursor) = config.get_mut(first) else {
            continue;
        };
        for (index, segment) in rest.iter().enumerate() {
            let is_last = index + 1 == rest.len();
            match cursor {
                Value::Object(map) => {
                    let Some(child) = map.get_mut(segment) else {
                        break;
                    };
                    if is_last {
                        *child = Value::String(secret.value.clone());
                        break;
                    }
                    cursor = child;
                }
                Value::Array(items) => {
                    let Some(child) = segment
                        .parse::<usize>()
                        .ok()
                        .and_then(|array_index| items.get_mut(array_index))
                    else {
                        break;
                    };
                    if is_last {
                        *child = Value::String(secret.value.clone());
                        break;
                    }
                    cursor = child;
                }
                _ => break,
            }
        }
    }
}

fn sanitize_imported_server_name(name: &str) -> String {
    let mut out: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_').to_string();
    let out = if out.is_empty() {
        "server".to_string()
    } else {
        out
    };
    out.chars().take(40).collect()
}

fn unique_temp_path(config_path: &std::path::Path) -> std::path::PathBuf {
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let filename = config_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    config_path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        filename,
        std::process::id(),
        sequence
    ))
}

async fn write_imported_config(
    config_path: &std::path::Path,
    content: &str,
    overwrite_existing: bool,
) -> std::io::Result<()> {
    let tmp_path = unique_temp_path(config_path);
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = tokio::fs::set_permissions(
            &tmp_path,
            std::fs::Permissions::from_mode(0o600),
        )
        .await
        {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(error);
        }
    }
    if let Err(error) = file.write_all(content.as_bytes()).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(error);
    }
    if let Err(error) = file.sync_all().await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(error);
    }
    drop(file);

    let result = if overwrite_existing {
        #[cfg(target_os = "windows")]
        if config_path.exists() {
            if let Err(error) = tokio::fs::remove_file(config_path).await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(error);
            }
        }
        tokio::fs::rename(&tmp_path, config_path).await
    } else {
        match tokio::fs::hard_link(&tmp_path, config_path).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                Ok(())
            }
            Err(error) => Err(error),
        }
    };
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }
    result
}

async fn reload_imported_config(
    gcx: std::sync::Arc<crate::global_context::GlobalContext>,
    path: &str,
) {
    let session = gcx.integration_sessions.lock().await.remove(path);
    if let Some(session) = session {
        let stop_future = {
            let mut session_locked = session.lock().await;
            session_locked.try_stop(session.clone())
        };
        Box::into_pin(stop_future).await;
    }
    crate::integrations::mcp::integr_mcp_common::tool_catalog_cache_remove(path);
    crate::integrations::mcp::mcp_resources::remove_indexed_resources(
        std::sync::Arc::downgrade(&gcx),
        path.to_string(),
    )
    .await;
    if let Some(filename) = std::path::Path::new(path).file_name() {
        let _ = crate::integrations::running_integrations::load_integrations(
            gcx,
            &[format!(
                "**/integrations.d/{}",
                filename.to_string_lossy()
            )],
        )
        .await;
    }
}

pub fn convert_mcp_servers_json_to_bundle(
    mcp_servers: &serde_json::Map<String, Value>,
) -> (Vec<ExportedServer>, Vec<Value>) {
    let mut servers = Vec::new();
    let mut errors = Vec::new();
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (name, entry) in mcp_servers {
        let base_name = sanitize_imported_server_name(name);
        let mut config_name = base_name.clone();
        let mut suffix = 2;
        while used_names.contains(&config_name) {
            config_name = format!("{}_{}", base_name, suffix);
            suffix += 1;
        }
        used_names.insert(config_name.clone());
        let obj = match entry.as_object() {
            Some(o) => o,
            None => {
                errors
                    .push(json!({ "config_name": name, "error": "server entry is not an object" }));
                continue;
            }
        };
        let mut config: HashMap<String, Value> = HashMap::new();
        let transport;
        if let Some(url) = obj.get("url").and_then(|v| v.as_str()) {
            transport = if url.trim_end_matches('/').ends_with("/sse") {
                "sse".to_string()
            } else {
                "http".to_string()
            };
            config.insert("url".to_string(), Value::String(url.to_string()));
            if let Some(headers) = obj.get("headers").filter(|v| v.is_object()) {
                config.insert("headers".to_string(), headers.clone());
            }
        } else if let Some(command) = obj.get("command").and_then(|v| v.as_str()) {
            transport = "stdio".to_string();
            let mut parts: Vec<String> = vec![command.to_string()];
            if let Some(args) = obj.get("args").and_then(|v| v.as_array()) {
                for a in args {
                    match a.as_str() {
                        Some(s) => parts.push(s.to_string()),
                        None => parts.push(a.to_string()),
                    }
                }
            }
            config.insert(
                "command".to_string(),
                Value::String(shell_words::join(parts.iter().map(|s| s.as_str()))),
            );
            if let Some(env) = obj.get("env").filter(|v| v.is_object()) {
                config.insert("env".to_string(), env.clone());
            }
        } else {
            errors.push(json!({
                "config_name": name,
                "error": "server entry has neither 'command' nor 'url'"
            }));
            continue;
        }
        servers.push(ExportedServer {
            config_name,
            transport,
            config,
            tools_config: HashMap::new(),
            confirmation: HashMap::new(),
        });
    }
    (servers, errors)
}

pub async fn handle_v1_mcp_import(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Json<Value>, ScratchError> {
    let gcx = app.gcx.clone();
    let mut raw: Value = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e)))?;

    let mut conversion_errors: Vec<Value> = Vec::new();
    if raw.get("from_project").and_then(|v| v.as_bool()) == Some(true) {
        let workspace_folders = {
            let folders = gcx
                .documents_state
                .workspace_folders
                .lock()
                .unwrap()
                .clone();
            folders
        };
        let overwrite_existing = raw
            .get("overwrite_existing")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut project_servers = Vec::new();
        let mut found_project_config = false;
        for folder in &workspace_folders {
            let config_path = folder.join(".refact").join("mcp-servers.json");
            let content = match tokio::fs::read_to_string(&config_path).await {
                Ok(content) => content,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    conversion_errors.push(json!({
                        "config_path": config_path.display().to_string(),
                        "error": error.to_string(),
                    }));
                    continue;
                }
            };
            found_project_config = true;
            let parsed = match serde_json::from_str::<Value>(&content) {
                Ok(parsed) => parsed,
                Err(error) => {
                    conversion_errors.push(json!({
                        "config_path": config_path.display().to_string(),
                        "error": format!("failed to parse: {}", error),
                    }));
                    continue;
                }
            };
            if let Ok(bundle) = serde_json::from_value::<ExportBundle>(parsed.clone()) {
                project_servers.extend(bundle.servers);
            } else if let Some(mcp_servers) = parsed
                .get("mcpServers")
                .or_else(|| parsed.as_object().map(|_| &parsed))
                .and_then(Value::as_object)
            {
                let (servers, errors) = convert_mcp_servers_json_to_bundle(mcp_servers);
                project_servers.extend(servers);
                conversion_errors.extend(errors.into_iter().map(|error| {
                    json!({
                        "config_path": config_path.display().to_string(),
                        "error": error,
                    })
                }));
            } else {
                conversion_errors.push(json!({
                    "config_path": config_path.display().to_string(),
                    "error": "unsupported project MCP config shape",
                }));
            }
        }
        if !found_project_config {
            return Err(ScratchError::new(
                StatusCode::NOT_FOUND,
                "no .refact/mcp-servers.json found in any workspace folder".to_string(),
            ));
        }
        raw = json!({
            "bundle": ExportBundle {
                version: EXPORT_VERSION,
                exported_at: String::new(),
                servers: project_servers,
            },
            "overwrite_existing": overwrite_existing,
        });
    }

    let raw = if raw.get("bundle").is_none()
        && raw.get("mcpServers").is_none()
        && raw.get("servers").map_or(false, |s| s.is_array())
        && raw.get("version").is_some()
    {
        let mut top = raw.as_object().cloned().unwrap_or_default();
        let overwrite = top.remove("overwrite_existing");
        let secrets = top.remove("secrets");
        let mut wrapped = serde_json::Map::new();
        wrapped.insert("bundle".to_string(), Value::Object(top));
        if let Some(overwrite) = overwrite {
            wrapped.insert("overwrite_existing".to_string(), overwrite);
        }
        if let Some(secrets) = secrets {
            wrapped.insert("secrets".to_string(), secrets);
        }
        Value::Object(wrapped)
    } else {
        raw
    };
    let req: ImportRequest = if let Some(mcp_servers) = raw
        .get("mcpServers")
        .or_else(|| raw.get("bundle").and_then(|b| b.get("mcpServers")))
        .and_then(|v| v.as_object())
    {
        let (servers, errors) = convert_mcp_servers_json_to_bundle(mcp_servers);
        conversion_errors.extend(errors);
        ImportRequest {
            bundle: ExportBundle {
                version: 1,
                exported_at: String::new(),
                servers,
            },
            overwrite_existing: raw
                .get("overwrite_existing")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            secrets: HashMap::new(),
        }
    } else {
        serde_json::from_value::<ImportRequest>(raw).map_err(|e| {
            ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON: {}", e))
        })?
    };

    let config_dir = gcx.config_dir.clone();
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
    let mut errors: Vec<Value> = conversion_errors;
    let mut touched_paths = Vec::new();
    let mut used_names = HashSet::new();
    if !req.overwrite_existing {
        let mut entries = tokio::fs::read_dir(&integrations_dir).await.map_err(|error| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        })? {
            if let Some(name) = entry.path().file_stem().and_then(|name| name.to_str()) {
                used_names.insert(name.to_string());
            }
        }
    }

    for server in &req.bundle.servers {
        if let Err(e) = mcp_naming::validate_config_filename(&server.config_name) {
            errors.push(json!({ "config_name": server.config_name, "error": e }));
            continue;
        }
        let prefix = mcp_naming::config_prefix_for_transport(&server.transport);
        let base_config_name = if server.config_name.starts_with("mcp_stdio_")
            || server.config_name.starts_with("mcp_sse_")
            || server.config_name.starts_with("mcp_http_")
        {
            server.config_name.clone()
        } else {
            format!("{}{}", prefix, server.config_name)
        };
        let mut config_name = base_config_name.clone();
        if !req.overwrite_existing {
            let mut suffix = 2;
            while used_names.contains(&config_name) {
                config_name = format!("{}_{}", base_config_name, suffix);
                suffix += 1;
            }
            used_names.insert(config_name.clone());
        }
        if let Err(e) = mcp_naming::validate_config_filename(&config_name) {
            errors.push(json!({ "config_name": server.config_name, "error": e }));
            continue;
        }

        let filename = format!("{}.yaml", config_name);
        let config_path = integrations_dir.join(&filename);

        let mut config = server.config.clone();
        if !server.tools_config.is_empty() {
            config.insert(
                "tools".to_string(),
                Value::Object(server.tools_config.clone().into_iter().collect()),
            );
        }
        if !server.confirmation.is_empty() {
            config.insert(
                "confirmation".to_string(),
                Value::Object(server.confirmation.clone().into_iter().collect()),
            );
        }
        if let Some(server_secrets) = req
            .secrets
            .get(&config_name)
            .or_else(|| req.secrets.get(&server.config_name))
        {
            apply_secrets_to_config(&mut config, server_secrets);
        }

        let yaml_content = serde_yaml::to_string(&config).unwrap_or_default();
        if let Err(e) =
            write_imported_config(&config_path, &yaml_content, req.overwrite_existing).await
        {
            if !req.overwrite_existing && e.kind() == std::io::ErrorKind::AlreadyExists {
                skipped.push(json!({ "config_name": config_name, "reason": "already exists" }));
                continue;
            }
            errors.push(json!({ "config_name": config_name, "error": e.to_string() }));
            continue;
        }

        let config_path_str = config_path.display().to_string();
        touched_paths.push(config_path_str.clone());
        imported.push(
            json!({ "config_name": config_name, "config_path": config_path_str }),
        );
    }

    for path in touched_paths {
        reload_imported_config(gcx.clone(), &path).await;
    }

    Ok(Json(json!({
        "imported": imported,
        "skipped": skipped,
        "errors": errors,
    })))
}

pub async fn handle_v1_mcp_project_config(
    State(app): State<AppState>,
) -> Result<Json<Value>, ScratchError> {
    let gcx = app.gcx.clone();
    let workspace_folders = {
        let folders = gcx
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
        // Accept both shapes the import endpoint accepts: a Refact export
        // bundle and a Claude/VS Code style `mcpServers` map.
        let bundle: ExportBundle = match serde_json::from_str::<ExportBundle>(&content) {
            Ok(b) => b,
            Err(bundle_err) => {
                let as_value: Option<Value> = serde_json::from_str(&content).ok();
                let converted = as_value
                    .as_ref()
                    .and_then(|v| v.get("mcpServers").or(Some(v)))
                    .and_then(|v| v.as_object())
                    .filter(|map| {
                        !map.is_empty()
                            && map.values().all(|entry| {
                                entry.get("command").is_some() || entry.get("url").is_some()
                            })
                    })
                    .map(|map| {
                        let (servers, _errors) = convert_mcp_servers_json_to_bundle(map);
                        ExportBundle {
                            version: 1,
                            exported_at: String::new(),
                            servers,
                        }
                    });
                match converted {
                    Some(b) => b,
                    None => {
                        project_configs.push(json!({
                            "project_dir": folder.display().to_string(),
                            "error": format!("failed to parse mcp-servers.json: {}", bundle_err),
                        }));
                        continue;
                    }
                }
            }
        };

        let config_dir = gcx.config_dir.clone();
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
    }

    #[test]
    fn test_apply_secrets_to_config_nested_paths() {
        let mut config: HashMap<String, Value> = HashMap::from([
            (
                "env".to_string(),
                serde_json::json!({ "API_KEY": "<REDACTED>" }),
            ),
            (
                "oauth_tokens".to_string(),
                serde_json::json!({ "access_token": "a", "client_secret": "<REDACTED>" }),
            ),
            (
                "bearer_token".to_string(),
                Value::String("<REDACTED>".to_string()),
            ),
        ]);
        let secrets = vec![
            ImportSecret {
                path: vec!["env".to_string(), "API_KEY".to_string()],
                value: "sk-env".to_string(),
            },
            ImportSecret {
                path: vec!["oauth_tokens".to_string(), "client_secret".to_string()],
                value: "oauth-secret".to_string(),
            },
            ImportSecret {
                path: vec!["bearer_token".to_string()],
                value: "bearer-secret".to_string(),
            },
        ];

        apply_secrets_to_config(&mut config, &secrets);

        assert_eq!(config["env"]["API_KEY"], "sk-env");
        assert_eq!(
            config["oauth_tokens"]["client_secret"], "oauth-secret",
            "nested non-env secrets must be restored in place"
        );
        assert_eq!(
            config["oauth_tokens"]["access_token"], "a",
            "sibling values must be untouched"
        );
        assert_eq!(config["bearer_token"], "bearer-secret");
    }

    #[test]
    fn test_apply_secrets_handles_non_map_intermediate() {
        let mut config = HashMap::from([("env".to_string(), Value::Bool(true))]);
        let secrets = vec![ImportSecret {
            path: vec!["env".to_string(), "TOKEN".to_string()],
            value: "v".to_string(),
        }];
        apply_secrets_to_config(&mut config, &secrets);
        assert_eq!(config["env"], Value::Bool(true));
    }

    #[test]
    fn test_apply_secrets_preserves_dotted_key() {
        let mut config = HashMap::from([(
            "env".to_string(),
            json!({ "FOO.BAR": REDACTED }),
        )]);
        let secrets = vec![ImportSecret {
            path: vec!["env".to_string(), "FOO.BAR".to_string()],
            value: "dotted-secret".to_string(),
        }];
        apply_secrets_to_config(&mut config, &secrets);
        assert_eq!(config["env"]["FOO.BAR"], "dotted-secret");
    }

    #[test]
    fn test_redact_secrets_nested_objects() {
        let mut value = json!({
            "oauth_tokens": { "access_token": "access", "safe": "visible" },
            "headers": { "Authorization": "Bearer secret", "Accept": "application/json" },
        });
        redact_secrets(&mut value);
        assert_eq!(value["oauth_tokens"]["access_token"], REDACTED);
        assert_eq!(value["oauth_tokens"]["safe"], "visible");
        assert_eq!(value["headers"]["Authorization"], REDACTED);
        assert_eq!(value["headers"]["Accept"], "application/json");
    }

    #[test]
    fn test_validate_config_name_rejects_traversal_legacy_shape() {
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

        let secrets = vec![ImportSecret {
            path: vec!["env".to_string(), "GITHUB_TOKEN".to_string()],
            value: "ghp_real".to_string(),
        }];
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

    #[test]
    fn test_convert_claude_desktop_stdio_and_url() {
        let raw: Value = serde_json::json!({
            "github": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"],
                "env": {"GITHUB_TOKEN": "x"}
            },
            "Linear MCP": { "url": "https://mcp.linear.app/mcp" },
            "legacy-sse": { "url": "https://old.example.com/sse" },
            "broken": { "neither": true }
        });
        let (servers, errors) = convert_mcp_servers_json_to_bundle(raw.as_object().unwrap());
        assert_eq!(servers.len(), 3);
        assert_eq!(errors.len(), 1);

        let github = servers.iter().find(|s| s.config_name == "github").unwrap();
        assert_eq!(github.transport, "stdio");
        assert_eq!(
            github.config.get("command").and_then(|v| v.as_str()),
            Some("npx -y @modelcontextprotocol/server-github")
        );
        assert!(github.config.get("env").is_some());

        let linear = servers
            .iter()
            .find(|s| s.config_name == "linear_mcp")
            .unwrap();
        assert_eq!(linear.transport, "http");
        assert_eq!(
            linear.config.get("url").and_then(|v| v.as_str()),
            Some("https://mcp.linear.app/mcp")
        );

        let sse = servers
            .iter()
            .find(|s| s.config_name == "legacy_sse")
            .unwrap();
        assert_eq!(sse.transport, "sse");
    }

    #[test]
    fn test_sanitize_imported_server_name() {
        assert_eq!(sanitize_imported_server_name("My Server!"), "my_server");
        assert_eq!(sanitize_imported_server_name("___"), "server");
        assert_eq!(
            sanitize_imported_server_name("a".repeat(60).as_str()).len(),
            40
        );
    }

    #[test]
    fn test_convert_colliding_names_are_deduplicated() {
        let raw: Value = serde_json::json!({
            "Linear MCP": { "url": "https://a.example.com/mcp" },
            "linear-mcp": { "url": "https://b.example.com/mcp" },
            "linear_mcp": { "url": "https://c.example.com/mcp" }
        });
        let (servers, errors) = convert_mcp_servers_json_to_bundle(raw.as_object().unwrap());
        assert!(errors.is_empty());
        assert_eq!(servers.len(), 3);
        let mut names: Vec<String> = servers.iter().map(|s| s.config_name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["linear_mcp", "linear_mcp_2", "linear_mcp_3"]);
    }

    #[test]
    fn test_convert_command_with_spaces_is_shell_quoted() {
        let raw: Value = serde_json::json!({
            "spacey": { "command": "/opt/my tools/bin/mcp", "args": ["--flag", "value with space"] }
        });
        let (servers, errors) = convert_mcp_servers_json_to_bundle(raw.as_object().unwrap());
        assert!(errors.is_empty());
        let cmd = servers[0]
            .config
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(cmd, "'/opt/my tools/bin/mcp' --flag 'value with space'");
    }
}
