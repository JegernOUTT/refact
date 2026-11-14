use std::sync::atomic::{AtomicBool, Ordering};
use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::integrations::mcp::{
    integr_mcp_common, mcp_interactions, redaction,
    session_mcp::{MCPConnectionStatus, SessionMCP},
};
use crate::integrations::setting_up_integrations::{self as configs, IntegrationRecord};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
};

pub struct ToolManageMcp;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    List,
    Status {
        name: String,
    },
    Logs {
        name: String,
        lines: Option<u64>,
    },
    Reconnect {
        name: String,
        wait_secs: Option<u64>,
    },
    Save {
        name: String,
        transport: String,
        scope: Option<String>,
        settings: Value,
        wait_secs: Option<u64>,
    },
    Remove {
        name: String,
        scope: Option<String>,
    },
}

fn validate_name(name: &str) -> Result<(), String> {
    if !regex::Regex::new(r"^[a-z0-9][a-z0-9_-]{0,62}$")
        .unwrap()
        .is_match(name)
        || ["http_", "sse_", "stdio_", "mcp_"]
            .iter()
            .any(|p| name.starts_with(p))
    {
        return Err(
            "Use a short name matching ^[a-z0-9][a-z0-9_-]{0,62}$ without a transport prefix"
                .into(),
        );
    }
    Ok(())
}

fn parse(args: &HashMap<String, Value>) -> Result<Request, String> {
    let allowed: &[&str] = match args.get("action").and_then(Value::as_str) {
        Some("list") => &["action"],
        Some("status") => &["action", "name"],
        Some("logs") => &["action", "name", "lines"],
        Some("reconnect") => &["action", "name", "wait_secs"],
        Some("save") => &[
            "action",
            "name",
            "transport",
            "scope",
            "settings",
            "wait_secs",
        ],
        Some("remove") => &["action", "name", "scope"],
        _ => return Err("action must be list|status|logs|reconnect|save|remove".into()),
    };
    if args.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err("Unsupported manage_mcp argument for this action".into());
    }

    let request: Request = serde_json::from_value(json!(args)).map_err(|_| {
        "Invalid manage_mcp arguments (unknown or incorrectly typed field)".to_string()
    })?;
    match &request {
        Request::List => {}
        Request::Status {
            name,
        }
        | Request::Logs {
            name, ..
        }
        | Request::Reconnect {
            name, ..
        }
        | Request::Save {
            name, ..
        }
        | Request::Remove {
            name, ..
        } => validate_name(name)?,
    }
    match &request {
        Request::Logs {
            lines: Some(n), ..
        } if *n == 0 || *n > 200 => return Err("lines must be 1..200".into()),
        Request::Reconnect {
            wait_secs: Some(n),
            ..
        }
        | Request::Save {
            wait_secs: Some(n),
            ..
        } if *n > 60 => return Err("wait_secs must be 0..60".into()),
        _ => {}
    }
    if let Request::Save {
        transport,
        settings,
        ..
    } = &request
    {
        if !["http", "sse", "stdio"].contains(&transport.as_str()) || !settings.is_object() {
            return Err("save requires transport http|sse|stdio and object settings".into());
        }
    }
    if let Request::Save {
        scope: Some(scope),
        ..
    }
    | Request::Remove {
        scope: Some(scope),
        ..
    } = &request
    {
        if !["project", "global"].contains(&scope.as_str()) {
            return Err("scope must be project|global".into());
        }
    }
    Ok(request)
}

fn identity(record: &IntegrationRecord) -> Option<(&str, &str)> {
    let name = record.integr_name.strip_prefix("mcp_")?;
    Some(
        ["http", "sse", "stdio"]
            .into_iter()
            .find_map(|t| name.strip_prefix(&format!("{t}_")).map(|n| (t, n)))
            .unwrap_or(("stdio", name)),
    )
}

fn scope(record: &IntegrationRecord) -> &str {
    if record.project_path.is_empty() {
        "global"
    } else {
        "project"
    }
}

async fn read_raw(path: &str) -> Result<Value, String> {
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|_| "Cannot read MCP config".to_string())?;
    serde_yaml::from_str(&text).map_err(|_| "Invalid MCP config YAML".into())
}

fn merge_settings(existing: &mut Value, patch: &Value, secret: bool) {
    if let (Some(dst), Some(src)) = (existing.as_object_mut(), patch.as_object()) {
        for (key, value) in src {
            let secret = secret || redaction::is_tool_secret_field(key);
            if secret && value.as_str() == Some("***") {
                continue;
            }
            merge_settings(dst.entry(key.clone()).or_insert(Value::Null), value, secret);
        }
    } else if !(secret && patch.as_str() == Some("***")) {
        *existing = patch.clone();
    }
}

fn collect_sensitive(value: &Value, secret: bool, values: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                collect_sensitive(
                    value,
                    secret || redaction::is_tool_secret_field(key),
                    values,
                );
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_sensitive(value, secret, values);
            }
        }
        Value::String(s) if secret => values.push(s.clone()),
        _ => {}
    }
}

fn substitutions(raw: &Value, substituted: &Value, values: &mut Vec<String>) {
    match raw {
        Value::Object(map) => {
            for (k, v) in map {
                substitutions(v, &substituted[k], values);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                substitutions(v, &substituted[i], values);
            }
        }
        Value::String(s) if s.contains('$') => {
            if let Some(s) = substituted.as_str() {
                values.push(s.to_owned());
            }
        }
        _ => {}
    }
}

fn sanitize(value: &mut Value, sensitive: &[String]) {
    redaction::redact_tool_secrets(value);
    fn walk(value: &mut Value, sensitive: &[String]) {
        match value {
            Value::Object(map) => {
                for value in map.values_mut() {
                    walk(value, sensitive);
                }
            }
            Value::Array(items) => {
                for value in items {
                    walk(value, sensitive);
                }
            }
            Value::String(s) => {
                *s = redaction::scrub_text(s, sensitive).replace(redaction::REDACTED, "***")
            }
            _ => {}
        }
    }
    walk(value, sensitive);
}

async fn entry(
    gcx: Arc<GlobalContext>,
    record: &IntegrationRecord,
    details: bool,
    lines: Option<usize>,
) -> Result<Value, String> {
    let (transport, name) = identity(record).ok_or("Not an MCP server")?;
    let raw = read_raw(&record.integr_config_path).await?;
    let mut sensitive = Vec::new();
    sensitive.extend(
        configs::get_vars_for_replacements(gcx.clone(), &mut Vec::new())
            .await
            .into_values(),
    );
    collect_sensitive(&record.config_unparsed, false, &mut sensitive);
    collect_sensitive(&raw, false, &mut sensitive);
    substitutions(&raw, &record.config_unparsed, &mut sensitive);
    let mut result = json!({"name":name,"transport":transport,"scope":scope(record),
        "config_path":record.integr_config_path,"privacy_destination":mcp_interactions::server_name_from_config_path(&record.integr_config_path),
        "status":"disabled","tools":0,"auth_status":"not_applicable"});
    let session = gcx
        .integration_sessions
        .lock()
        .await
        .get(&record.integr_config_path)
        .cloned();
    if let Some(session) = session {
        let mut locked = session.lock().await;
        if let Some(mcp) = locked.as_any_mut().downcast_mut::<SessionMCP>() {
            collect_sensitive(&mcp.launched_cfg, false, &mut sensitive);
            result["status"] = json!(match &mcp.connection_status {
                MCPConnectionStatus::Connected => "connected".into(),
                MCPConnectionStatus::Connecting => "connecting".into(),
                MCPConnectionStatus::Reconnecting {
                    attempt,
                } => format!(
                    "reconnecting attempt {attempt}, next try in {}s",
                    integr_mcp_common::mcp_next_retry_in_secs(&record.integr_config_path)
                        .unwrap_or(0)
                ),
                MCPConnectionStatus::Failed {
                    message,
                } => format!("failed: {message}"),
                MCPConnectionStatus::NeedsAuth => "needs authentication".into(),
                MCPConnectionStatus::Disconnected => "failed: disconnected".into(),
            });
            result["tools"] = json!(mcp.mcp_tools.len());
            result["auth_status"] = serde_json::to_value(&mcp.auth_status).unwrap_or(Value::Null);
            if details {
                result["tool_names"] = json!(mcp
                    .mcp_tools
                    .iter()
                    .take(100)
                    .map(|t| t.name.to_string())
                    .collect::<Vec<_>>());
            }
            if let Some(lines) = lines {
                if let Some(path) = &mcp.stderr_file_path {
                    let _ = crate::integrations::mcp::session_mcp::update_logs_from_stderr(
                        path,
                        mcp.stderr_cursor.clone(),
                        mcp.logs.clone(),
                    )
                    .await;
                }
                let logs = mcp.logs.lock().await;
                result["logs"] = json!(logs
                    .iter()
                    .skip(logs.len().saturating_sub(lines))
                    .collect::<Vec<_>>());
            }
        }
    } else if lines.is_some() {
        result["logs"] = json!([]);
    }
    if !record.on_your_laptop && !record.when_isolated {
        result["status"] = json!("disabled");
    }
    if details {
        result["settings"] = raw;
    }
    sanitize(&mut result, &sensitive);
    Ok(result)
}

async fn wait_for_server(
    gcx: Arc<GlobalContext>,
    record: &IntegrationRecord,
    seconds: u64,
    abort: Arc<AtomicBool>,
) -> Result<Value, String> {
    let poll = async {
        loop {
            if abort.load(Ordering::SeqCst) || gcx.shutdown_flag.load(Ordering::SeqCst) {
                return Err("manage_mcp wait aborted".to_string());
            }
            let value = entry(gcx.clone(), record, false, None).await?;
            let state = value["status"].as_str().unwrap_or_default();
            if !(state == "connecting" || state.starts_with("reconnecting")) {
                return Ok(value);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    match tokio::time::timeout(Duration::from_secs(seconds), poll).await {
        Ok(result) => result,
        Err(_) => entry(gcx, record, false, None).await,
    }
}

async fn allowed(gcx: Arc<GlobalContext>, model: &str, path: &str) -> Result<(), String> {
    if super::tools_list::mcp_config_allowed_for_model(gcx, model, path).await {
        Ok(())
    } else {
        Err("MCP server is not allowed for this model provider".into())
    }
}

fn resolve<'a>(
    records: &'a [IntegrationRecord],
    name: &str,
    requested_scope: Option<&str>,
) -> Result<&'a IntegrationRecord, String> {
    let matches: Vec<_> = records
        .iter()
        .filter(|r| {
            identity(r).is_some_and(|(_, n)| n == name)
                && requested_scope.map_or(true, |s| scope(r) == s)
        })
        .collect();
    match matches.as_slice() {
        [record] => Ok(record),
        [] => Err(format!(
            "Unknown MCP server. Known names: {}",
            records
                .iter()
                .filter_map(|r| identity(r).map(|(_, n)| n))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => Err("Ambiguous MCP name; specify scope for remove, or use a unique short name".into()),
    }
}

async fn execute(ccx: Arc<Mutex<AtCommandsContext>>, request: Request) -> Result<Value, String> {
    let (gcx, model, execution_scope, abort) = {
        let c = ccx.lock().await;
        (
            c.app.gcx.clone(),
            c.current_model.clone(),
            c.execution_scope.clone(),
            c.abort_flag.clone(),
        )
    };
    let all = configs::integrations_all(gcx.clone(), false)
        .await
        .integrations;
    let mut records = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for record in all
        .into_iter()
        .filter(|r| r.integr_config_exists && identity(r).is_some())
    {
        if !seen.insert(record.integr_config_path.clone()) {
            continue;
        }
        if allowed(gcx.clone(), &model, &record.integr_config_path)
            .await
            .is_ok()
        {
            records.push(record);
        }
    }
    match request {
        Request::List => {
            let mut entries = Vec::new();
            for record in &records {
                entries.push(entry(gcx.clone(), record, false, None).await?);
            }
            Ok(json!({"servers":entries}))
        }
        Request::Save {
            name,
            transport,
            scope: requested_scope,
            settings,
            wait_secs,
        } => {
            let requested_scope = requested_scope.as_deref().unwrap_or("project");
            let current_root = execution_scope
                .as_ref()
                .map(|s| s.effective_root().to_path_buf());
            let (dirs, global) = configs::get_config_dirs(gcx.clone(), &current_root).await;
            let dir = if requested_scope == "global" {
                global
            } else {
                match dirs.as_slice() {
                    [dir] => dir.clone(),
                    _ => return Err("save needs one unambiguous project workspace".into()),
                }
            };
            let path = configs::join_config_path(&dir, &format!("mcp_{transport}_{name}"));
            allowed(gcx.clone(), &model, &path).await?;
            crate::integrations::mcp::management::validate_config_save_path(
                Path::new(&path),
                &dir.join("integrations.d"),
            )
            .map_err(|e| e.message)?;
            let mut merged = if Path::new(&path).exists() {
                read_raw(&path).await?
            } else {
                json!({})
            };
            merge_settings(&mut merged, &settings, false);
            configs::integration_config_save(gcx.clone(), &path, &merged)
                .await
                .map_err(|_| {
                    "MCP settings validation or save failed; check settings and server logs"
                        .to_string()
                })?;
            let updated = configs::integrations_all(gcx.clone(), false)
                .await
                .integrations;
            let record = updated
                .iter()
                .find(|r| r.integr_config_path == path)
                .ok_or("Saved MCP config not found")?;
            let mut result = wait_for_server(gcx, record, wait_secs.unwrap_or(10), abort).await?;
            let destination = mcp_interactions::server_name_from_config_path(&path);
            result["note"] = json!(format!("If a privacy zone guards data in this chat, add {destination} to that zone's destinations in Privacy settings. In lazy MCP mode new tools are reachable via the MCP search/call proxies once connected; otherwise you may need Refresh Tools."));
            Ok(result)
        }
        Request::Status {
            name,
        } => entry(gcx, resolve(&records, &name, None)?, true, None).await,
        Request::Logs {
            name,
            lines,
        } => {
            entry(
                gcx,
                resolve(&records, &name, None)?,
                false,
                Some(lines.unwrap_or(50) as usize),
            )
            .await
        }
        Request::Reconnect {
            name,
            wait_secs,
        } => {
            let record = resolve(&records, &name, None)?;
            crate::integrations::mcp::management::reconnect_mcp_server(
                gcx.clone(),
                &record.integr_config_path,
            )
            .await
            .map_err(|e| e.message)?;
            wait_for_server(gcx, record, wait_secs.unwrap_or(10), abort).await
        }
        Request::Remove {
            name,
            scope,
        } => {
            let record = resolve(&records, &name, scope.as_deref())?;
            crate::integrations::mcp::management::delete_integration_config(
                gcx,
                Path::new(&record.integr_config_path),
            )
            .await
            .map_err(|e| e.message)?;
            Ok(json!({"name":name,"removed":true}))
        }
    }
}

#[async_trait]
impl Tool for ToolManageMcp {
    fn config(&self) -> Result<crate::tools::tools_description::ToolConfig, String> {
        Ok(crate::tools::tools_description::ToolConfig {
            enabled: true,
            allow_parallel: None,
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "manage_mcp".into(), display_name: "Manage MCP".into(), experimental: false,
            allow_parallel: false,
            description: "Manage configured MCP servers without restarting. list returns status/count/auth/privacy destination; status adds redacted settings and up to 100 tool names; logs returns a redacted tail. reconnect reruns existing configuration without confirmation. save merges settings (*** keeps existing secrets); save/remove require confirmation. Use short names, not transport prefixes.".into(),
            input_schema: json!({"type":"object","additionalProperties":false,"required":["action"],"properties":{
                "action":{"type":"string","enum":["list","status","logs","reconnect","save","remove"]},
                "name":{"type":"string","pattern":"^[a-z0-9][a-z0-9_-]{0,62}$","description":"Required except list. Short name without http_/sse_/stdio_ prefix."},
                "transport":{"type":"string","enum":["http","sse","stdio"],"description":"Required for save"},
                "scope":{"type":"string","enum":["project","global"],"description":"save default project; remove optional disambiguation"},
                "settings":{"type":"object","description":"Required for save. Provided keys override; omitted keys and *** secrets are retained."},
                "lines":{"type":"integer","minimum":1,"maximum":200,"default":50},
                "wait_secs":{"type":"integer","minimum":0,"maximum":60,"default":10}
            }}),
            output_schema: None, annotations: None,
            source: ToolSource { source_type: ToolSourceType::Builtin, config_path: String::new() },
        }
    }

    async fn match_against_confirm_deny(
        &self,
        _ccx: Arc<Mutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let request = parse(args)?;
        let (mutates, command) = match request {
            Request::Save {
                name,
                transport,
                scope,
                ..
            } => (
                true,
                format!(
                    "manage_mcp save mcp_{transport}_{name}.yaml ({})",
                    scope.as_deref().unwrap_or("project")
                ),
            ),
            Request::Remove {
                name,
                scope,
            } => (
                true,
                format!(
                    "manage_mcp remove {name} ({})",
                    scope.as_deref().unwrap_or("configured scope")
                ),
            ),
            _ => (false, String::new()),
        };
        Ok(MatchConfirmDeny {
            result: if mutates {
                MatchConfirmDenyResult::CONFIRMATION
            } else {
                MatchConfirmDenyResult::PASS
            },
            command,
            rule: if mutates {
                "MCP configuration change requires approval".into()
            } else {
                String::new()
            },
        })
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let value = execute(ccx, parse(args)?).await?;
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".into(),
                tool_call_id: tool_call_id.clone(),
                content: ChatContent::SimpleText(value.to_string()),
                ..Default::default()
            })],
        ))
    }
}

#[cfg(test)]
mod manage_mcp_tests {
    use super::*;

    async fn fixture() -> (
        tempfile::TempDir,
        Arc<GlobalContext>,
        Arc<Mutex<AtCommandsContext>>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join(".config/refact"),
        )
        .await;
        gcx.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .push(dir.path().to_path_buf());
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let context = AtCommandsContext::new_from_app(
            app,
            4096,
            1,
            false,
            vec![],
            "manage-mcp-test".into(),
            None,
            "provider/model".into(),
            None,
            None,
        )
        .await;
        (dir, gcx, Arc::new(Mutex::new(context)))
    }

    #[tokio::test]
    async fn manage_mcp_confirmation_is_per_action() {
        let (_dir, _gcx, ccx) = fixture().await;
        for (action, confirmation) in [
            ("list", false),
            ("status", false),
            ("logs", false),
            ("reconnect", false),
            ("save", true),
            ("remove", true),
        ] {
            let mut value = json!({"action":action});
            if action != "list" {
                value["name"] = json!("unreal");
            }
            if action == "save" {
                value["transport"] = json!("http");
                value["settings"] = json!({});
            }
            let matched = ToolManageMcp
                .match_against_confirm_deny(ccx.clone(), &serde_json::from_value(value).unwrap())
                .await
                .unwrap();
            assert_eq!(
                matches!(matched.result, MatchConfirmDenyResult::CONFIRMATION),
                confirmation,
                "{action}"
            );
            assert_eq!(
                matches!(matched.result, MatchConfirmDenyResult::PASS),
                !confirmation,
                "{action}"
            );
        }
    }

    #[tokio::test]
    async fn manage_mcp_save_update_status_logs_access_and_remove() {
        let (dir, gcx, ccx) = fixture().await;
        let settings = json!({"url":"invalid-url", "bearer_token":"bearer-private", "oauth2_client_secret":"oauth-private", "headers":{"Authorization":"header-private"},"available":{"on_your_laptop":false,"when_isolated":false}});
        let saved = execute(
            ccx.clone(),
            Request::Save {
                name: "fixture".into(),
                transport: "http".into(),
                scope: None,
                settings,
                wait_secs: Some(0),
            },
        )
        .await
        .unwrap();
        assert_eq!(saved["privacy_destination"], "fixture");
        assert_eq!(saved["scope"], "project");
        let path = dir
            .path()
            .join(".refact/integrations.d/mcp_http_fixture.yaml");
        assert!(path.exists());
        execute(ccx.clone(), Request::Save { name:"fixture".into(), transport:"http".into(), scope:None, settings:json!({"bearer_token":"***","headers":{"Authorization":"***"},"url":"$TEST_URL"}), wait_secs:Some(0) }).await.unwrap();
        let raw = read_raw(path.to_str().unwrap()).await.unwrap();
        assert_eq!(raw["bearer_token"], "bearer-private");
        assert_eq!(raw["headers"]["Authorization"], "header-private");
        std::fs::write(
            gcx.config_dir.join("variables.yaml"),
            "TEST_URL: substituted-private\n",
        )
        .unwrap();
        let key = path.to_string_lossy().into_owned();
        integr_mcp_common::remove_mcp_session_for_config(gcx.clone(), &key).await;
        let mut session = SessionMCP::detached(&key);
        session.connection_status = MCPConnectionStatus::Failed {
            message: "bearer-private substituted-private".into(),
        };
        session.logs.lock().await.extend([
            "Bearer raw-private".into(),
            "Authorization: header-private".into(),
            "substituted-private oauth-private".into(),
        ]);
        gcx.integration_sessions
            .lock()
            .await
            .insert(key.clone(), Arc::new(Mutex::new(Box::new(session))));
        for request in [
            Request::List,
            Request::Status {
                name: "fixture".into(),
            },
            Request::Logs {
                name: "fixture".into(),
                lines: Some(2),
            },
        ] {
            let value = execute(ccx.clone(), request).await.unwrap();
            let text = value.to_string();
            for secret in [
                "bearer-private",
                "oauth-private",
                "header-private",
                "substituted-private",
                "raw-private",
            ] {
                assert!(!text.contains(secret), "{secret}: {text}");
            }
        }
        {
            let session = gcx
                .integration_sessions
                .lock()
                .await
                .get(&key)
                .cloned()
                .unwrap();
            session
                .lock()
                .await
                .as_any_mut()
                .downcast_mut::<SessionMCP>()
                .unwrap()
                .connection_status = MCPConnectionStatus::Reconnecting {
                attempt: 2,
            };
        }
        let error = execute(
            ccx.clone(),
            Request::Reconnect {
                name: "fixture".into(),
                wait_secs: Some(0),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error, "MCP server is already connecting or reconnecting");
        {
            let mut loaded = gcx.privacy_policy_load.write().unwrap();
            let mut policy = loaded.policy.as_ref().clone();
            policy.tool_access =
                serde_json::from_value(json!({"providers":{"provider":{"mcp":[]}}})).unwrap();
            loaded.policy = Arc::new(policy);
        }
        assert_eq!(
            execute(ccx.clone(), Request::List).await.unwrap(),
            json!({"servers":[]})
        );
        assert!(execute(
            ccx.clone(),
            Request::Status {
                name: "fixture".into()
            }
        )
        .await
        .is_err());
        assert!(execute(
            ccx.clone(),
            Request::Remove {
                name: "fixture".into(),
                scope: None
            }
        )
        .await
        .is_err());
        assert!(allowed(gcx.clone(), "provider/model", &key).await.is_err());
        {
            let mut loaded = gcx.privacy_policy_load.write().unwrap();
            let mut policy = loaded.policy.as_ref().clone();
            policy.tool_access = Default::default();
            loaded.policy = Arc::new(policy);
        }
        let outside = dir.path().join("mcp_http_outside.yaml");
        std::fs::write(&outside, "{}").unwrap();
        assert!(
            crate::integrations::mcp::management::delete_integration_config(gcx.clone(), &outside)
                .await
                .is_err()
        );
        let alien = dir
            .path()
            .join("alien/.refact/integrations.d/mcp_http_alien.yaml");
        std::fs::create_dir_all(alien.parent().unwrap()).unwrap();
        std::fs::write(&alien, "{}").unwrap();
        assert!(
            crate::integrations::mcp::management::delete_integration_config(gcx.clone(), &alien)
                .await
                .is_err()
        );
        assert!(alien.exists());
        assert!(outside.exists());
        execute(
            ccx,
            Request::Remove {
                name: "fixture".into(),
                scope: Some("project".into()),
            },
        )
        .await
        .unwrap();
        assert!(!path.exists());
        assert!(!gcx.integration_sessions.lock().await.contains_key(&key));
    }

    #[test]
    fn manage_mcp_strict_validation_and_schema() {
        let schema = ToolManageMcp.tool_description().input_schema;
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["action"]["enum"],
            json!(["list", "status", "logs", "reconnect", "save", "remove"])
        );
        for value in [
            json!({"action":"list","approved":true}),
            json!({"action":"logs","name":"a","lines":201}),
            json!({"action":"save","name":"a","transport":"http","settings":[],"wait_secs":0}),
        ] {
            assert!(parse(&serde_json::from_value(value).unwrap()).is_err());
        }
        for name in [
            "../a", "a/b", "a\\b", "http_a", "sse_a", "stdio_a", "A", "a..b",
        ] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        validate_name("unreal-1").unwrap();
    }

    #[tokio::test]
    async fn manage_mcp_entry_redacts_failed_status_and_log_tail() {
        let (dir, gcx, _ccx) = fixture().await;
        let path = dir
            .path()
            .join(".refact/integrations.d/mcp_http_redaction.yaml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let raw = json!({"url":"$HOST", "bearer_token":"bearer-private", "oauth2_client_secret":"oauth-private", "headers":{"Authorization":"header-private", "X-Custom":"custom-private"}});
        std::fs::write(&path, serde_yaml::to_string(&raw).unwrap()).unwrap();
        let key = path.to_string_lossy().into_owned();
        let record = IntegrationRecord {
            integr_name: "mcp_http_redaction".into(),
            integr_config_path: key.clone(),
            integr_config_exists: true,
            on_your_laptop: true,
            config_unparsed: json!({"url":"substituted-private"}),
            ..Default::default()
        };
        integr_mcp_common::remove_mcp_session_for_config(gcx.clone(), &key).await;
        let mut session = SessionMCP::detached(&key);
        session.connection_status = MCPConnectionStatus::Failed {
            message: "bearer-private substituted-private".into(),
        };
        session.logs.lock().await.extend([
            "older".into(),
            "Bearer raw-private".into(),
            "custom-private header-private oauth-private substituted-private".into(),
        ]);
        gcx.integration_sessions
            .lock()
            .await
            .insert(key.clone(), Arc::new(Mutex::new(Box::new(session))));
        let status = entry(gcx.clone(), &record, true, None).await.unwrap();
        assert_eq!(status["status"], "failed: *** ***");
        assert_eq!(status["settings"]["headers"]["X-Custom"], "***");
        let logs = entry(gcx.clone(), &record, false, Some(2)).await.unwrap();
        assert_eq!(logs["logs"], json!(["Bearer ***", "*** *** *** ***"]));
        assert!(logs.get("settings").is_none());
        crate::integrations::mcp::management::delete_integration_config(gcx, &path)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn manage_mcp_wait_returns_promptly_on_abort() {
        let (dir, gcx, _ccx) = fixture().await;
        let path = dir.path().join(".refact/integrations.d/mcp_http_slow.yaml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "url: http://127.0.0.1:9/mcp\n").unwrap();
        let key = path.to_string_lossy().into_owned();
        let record = IntegrationRecord {
            integr_name: "mcp_http_slow".into(),
            integr_config_path: key.clone(),
            integr_config_exists: true,
            on_your_laptop: true,
            ..Default::default()
        };
        let mut session = SessionMCP::detached(&key);
        session.connection_status = MCPConnectionStatus::Connecting;
        gcx.integration_sessions
            .lock()
            .await
            .insert(key, Arc::new(Mutex::new(Box::new(session))));
        let abort = Arc::new(AtomicBool::new(false));
        tokio::spawn({
            let abort = abort.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                abort.store(true, Ordering::SeqCst);
            }
        });
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_server(gcx.clone(), &record, 60, abort),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        let still_connecting =
            wait_for_server(gcx, &record, 0, Arc::new(AtomicBool::new(false))).await;
        assert_eq!(still_connecting.unwrap()["status"], "connecting");
    }

    #[test]
    fn manage_mcp_legacy_and_transport_identities() {
        for (file, transport, name) in [
            ("mcp_http_unreal", "http", "unreal"),
            ("mcp_sse_old", "sse", "old"),
            ("mcp_stdio_local", "stdio", "local"),
            ("mcp_legacy", "stdio", "legacy"),
        ] {
            let record = IntegrationRecord {
                integr_name: file.into(),
                ..Default::default()
            };
            assert_eq!(identity(&record), Some((transport, name)));
        }
    }

    #[test]
    fn manage_mcp_merge_preserves_secrets_and_redacts_output() {
        let mut existing = json!({"bearer_token":"bearer-private","oauth2_client_secret":"oauth-private","headers":{"Authorization":"header-private"},"command":"old","args":["$VAR"]});
        merge_settings(
            &mut existing,
            &json!({"bearer_token":"***","headers":{"Authorization":"***"},"command":"new"}),
            false,
        );
        assert_eq!(existing["command"], "new");
        assert_eq!(existing["bearer_token"], "bearer-private");
        assert_eq!(existing["headers"]["Authorization"], "header-private");
        assert_eq!(existing["oauth2_client_secret"], "oauth-private");
        let mut values = vec![];
        collect_sensitive(&existing, false, &mut values);
        substitutions(
            &existing,
            &json!({"args":["substituted-private"]}),
            &mut values,
        );
        let mut result = json!({"settings":existing,"logs":["Bearer raw-private token=query-private api_key=key-private", "Authorization: auth-private", "substituted-private bearer-private oauth-private header-private"]});
        sanitize(&mut result, &values);
        let text = result.to_string();
        for secret in [
            "bearer-private",
            "oauth-private",
            "header-private",
            "substituted-private",
            "raw-private",
            "query-private",
            "key-private",
            "auth-private",
        ] {
            assert!(!text.contains(secret), "{secret}: {text}");
        }
        assert_eq!(result["settings"]["bearer_token"], "***");
    }
}
