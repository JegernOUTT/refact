use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolConfig, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolMcpSearch {}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[tokio::test]
    async fn tool_mcp_status_is_filtered_and_does_not_render_secrets() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let context = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                1,
                false,
                vec![],
                "mcp-status".into(),
                None,
                "provider/model".into(),
                None,
                None,
            )
            .await,
        ));
        let path = "/tmp/mcp_http_status_fixture.yaml";
        let mut session = crate::integrations::mcp::session_mcp::SessionMCP::detached(path);
        session.launched_cfg = json!({"url":"https://user:password@example.com", "headers":{"Authorization":"secret"}});
        session.connection_status =
            crate::integrations::mcp::session_mcp::MCPConnectionStatus::Failed {
                message: "https://user:password@example.com Authorization: secret".into(),
            };
        gcx.integration_sessions
            .lock()
            .await
            .insert(path.into(), Arc::new(AMutex::new(Box::new(session))));
        let text = unavailable_mcp_status_summary(&context).await;
        assert_eq!(text, " 1 MCP server(s) have no usable connection or catalog: http_status_fixture — failed: connection or catalog unavailable. Their tools will appear once connected.");
        {
            let mut loaded = gcx.privacy_policy_load.write().unwrap();
            let mut policy = loaded.policy.as_ref().clone();
            policy.tool_access = serde_json::from_value(json!({"providers": {
                "provider": {"mcp": []}
            }}))
            .unwrap();
            loaded.policy = Arc::new(policy);
        }
        assert_eq!(unavailable_mcp_status_summary(&context).await, "");
        context.lock().await.tool_access_bypass = true;
        assert_eq!(unavailable_mcp_status_summary(&context).await, text);
        context.lock().await.child_tool_ceiling = Some(crate::subchat::ToolsPolicy::Only(vec![
            "mcp_other_read".into(),
        ]));
        assert_eq!(unavailable_mcp_status_summary(&context).await, "");
        context.lock().await.child_tool_ceiling = Some(crate::subchat::ToolsPolicy::Only(vec![
            "mcp_http_status_fixture_read".into(),
        ]));
        assert_eq!(unavailable_mcp_status_summary(&context).await, text);
        context.lock().await.child_tool_ceiling = Some(crate::subchat::ToolsPolicy::None);
        assert_eq!(unavailable_mcp_status_summary(&context).await, "");
        gcx.integration_sessions.lock().await.clear();
        for path in ["/tmp/mcp_http_a.yaml", "/tmp/mcp_http_a_b.yaml"] {
            let session = crate::integrations::mcp::session_mcp::SessionMCP::detached(path);
            gcx.integration_sessions
                .lock()
                .await
                .insert(path.into(), Arc::new(AMutex::new(Box::new(session))));
        }
        context.lock().await.child_tool_ceiling = Some(crate::subchat::ToolsPolicy::Only(vec![
            "mcp_http_a_b_x".into(),
        ]));
        assert_eq!(unavailable_mcp_status_summary(&context).await, "");
        context.lock().await.child_tool_ceiling = None;
        assert_eq!(unavailable_mcp_status_summary(&context).await,
            " 2 MCP server(s) have no usable connection or catalog: http_a — disconnected; http_a_b — disconnected. Their tools will appear once connected.");
        gcx.integration_sessions.lock().await.clear();
    }
}

pub async fn unavailable_mcp_status_summary(ccx: &Arc<AMutex<AtCommandsContext>>) -> String {
    let (gcx, model, bypass, app, agent, mut ceiling) = {
        let context = ccx.lock().await;
        (
            context.app.gcx.clone(),
            context.current_model.clone(),
            context.tool_access_bypass,
            context.app.clone(),
            context.background_agent_id.clone(),
            context.child_tool_ceiling.clone(),
        )
    };
    if let Some(agent) = agent {
        let Some(config) = app.agent_executions.saved_config(&agent) else {
            return String::new();
        };
        if matches!(config.tools, crate::subchat::ToolsPolicy::All) {
            return String::new();
        }
        ceiling = Some(ceiling.as_ref().map_or(config.tools.clone(), |ceiling| {
            crate::agents::tool_authority::intersect(ceiling, &config.tools)
        }));
    }
    let sessions: Vec<_> = gcx
        .integration_sessions
        .lock()
        .await
        .values()
        .cloned()
        .collect();
    let mut prefixes = std::collections::HashSet::new();
    let mut snapshots: Vec<(String, String)> = Vec::new();
    for session in sessions {
        let mut locked = session.lock().await;
        let Some(session) = locked
            .as_any_mut()
            .downcast_mut::<crate::integrations::mcp::session_mcp::SessionMCP>()
        else {
            continue;
        };
        prefixes.insert(
            crate::integrations::mcp::integr_mcp_common::tool_name_server_prefix(
                &session.config_path,
            ),
        );
        if session.launched_cfg.get("operator_tools").is_some() {
            continue;
        }
        use crate::integrations::mcp::session_mcp::MCPConnectionStatus;
        let state = match &session.connection_status {
            MCPConnectionStatus::Connected if !session.mcp_tools.is_empty() => continue,
            MCPConnectionStatus::Connected => "connected, catalog empty".into(),
            MCPConnectionStatus::Connecting => "connecting".into(),
            MCPConnectionStatus::Reconnecting {
                attempt,
            } => {
                let next = crate::integrations::mcp::integr_mcp_common::mcp_next_retry_in_secs(
                    &session.config_path,
                )
                .unwrap_or(0);
                format!("reconnecting (attempt {attempt}, next try in {next}s)")
            }
            MCPConnectionStatus::Failed {
                ..
            } => "failed: connection or catalog unavailable".into(),
            MCPConnectionStatus::NeedsAuth => "needs authentication".into(),
            MCPConnectionStatus::Disconnected => "disconnected".into(),
        };
        snapshots.push((session.config_path.clone(), state));
    }
    let mut rows = Vec::new();
    for snapshot in snapshots {
        if !bypass
            && !crate::tools::tools_list::mcp_config_allowed_for_model(
                gcx.clone(),
                &model,
                &snapshot.0,
            )
            .await
        {
            continue;
        }
        let prefix =
            crate::integrations::mcp::integr_mcp_common::tool_name_server_prefix(&snapshot.0);
        if let Some(ceiling) = &ceiling {
            let names = match ceiling {
                crate::subchat::ToolsPolicy::Only(names)
                | crate::subchat::ToolsPolicy::Supervised {
                    tools: names, ..
                } => names,
                _ => continue,
            };
            let mut allowed = false;
            for name in names.iter().filter(|name| {
                name.starts_with(&format!("{prefix}_"))
                    && prefixes
                        .iter()
                        .filter(|candidate| name.starts_with(&format!("{candidate}_")))
                        .count()
                        == 1
            }) {
                if crate::agents::tool_authority::ensure_context_tool(ccx, name)
                    .await
                    .is_ok()
                {
                    allowed = true;
                    break;
                }
            }
            if !allowed {
                continue;
            }
        }
        rows.push(format!(
            "{} — {}",
            prefix.strip_prefix("mcp_").unwrap_or(&prefix),
            snapshot.1
        ));
    }
    rows.sort();
    rows.dedup();
    if rows.is_empty() {
        String::new()
    } else {
        format!(" {} MCP server(s) have no usable connection or catalog: {}. Their tools will appear once connected.", rows.len(), rows.join("; "))
    }
}

#[async_trait]
impl Tool for ToolMcpSearch {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "mcp_tool_search".to_string(),
            experimental: false,
            allow_parallel: false,
            description:
                "Search available MCP tools by regex pattern (case-insensitive, matched \
                against tool name and description). Returns matching tool names and their full \
                JSON schemas as text. After discovering a tool here, call `mcp_call` to execute it."
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Regex pattern to match MCP tool names and descriptions. \
                            Examples: \"github\", \"file.*read|write\", \"git.*(commit|push)\""
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of tools to return (default 10)"
                    }
                },
                "required": ["query"]
            }),
            output_schema: None,
            annotations: None,
            display_name: "MCP Tool Search".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
        }
    }

    fn config(&self) -> Result<ToolConfig, String> {
        Ok(ToolConfig {
            enabled: true,
            allow_parallel: None,
        })
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        let re = regex::Regex::new(&format!("(?i){}", query))
            .map_err(|e| format!("Invalid regex pattern '{}': {}", query, e))?;

        let (gcx, model_id, bypass) = {
            let ccx = ccx.lock().await;
            (
                ccx.app.gcx.clone(),
                ccx.current_model.clone(),
                ccx.tool_access_bypass,
            )
        };
        let mcp_tools = crate::tools::tools_list::get_integration_mcp_tools(gcx.clone()).await;
        // Child tool authority must narrow the list that is actually reported.
        // Searching is discovery, and discovery of a tool the child may not call
        // is already an authority leak, so this filter runs before any other.
        let mut allowed_mcp_tools = Vec::with_capacity(mcp_tools.len());
        for tool in mcp_tools {
            if crate::agents::tool_authority::ensure_context_tool(
                &ccx,
                &tool.tool_description().name,
            )
            .await
            .is_ok()
            {
                allowed_mcp_tools.push(tool);
            }
        }
        let mut mcp_tools = allowed_mcp_tools;
        let policy = crate::tools::tools_list::tool_access_policy(gcx.clone()).await;
        if !bypass && !policy.tool_access.providers.is_empty() {
            let provider = crate::tools::tools_list::provider_of_model(&model_id);
            mcp_tools.retain(|tool| {
                crate::tools::tools_list::mcp_tool_allowed(
                    &policy,
                    provider,
                    &tool.tool_description(),
                )
            });
        }

        let matched: Vec<(String, String, Value)> = mcp_tools
            .iter()
            .filter(|tool| tool.config().unwrap_or_default().enabled)
            .map(|tool| tool.tool_description())
            .filter(|d| re.is_match(&d.name) || re.is_match(&d.description))
            .take(max_results)
            .map(|d| (d.name, d.description, d.input_schema))
            .collect();

        let total_mcp: usize = mcp_tools.len();
        let unavailable = unavailable_mcp_status_summary(&ccx).await;

        if matched.is_empty() {
            let text = format!(
                "No MCP tools found matching '{}'. Try a broader pattern. \
                 Use mcp_tool_search({{\"query\": \".\", \"max_results\": {}}}) to list all {} available tools.{}",
                query, total_mcp, total_mcp, unavailable
            );
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    tool_call_id: tool_call_id.clone(),
                    content: ChatContent::SimpleText(text),
                    tool_failed: Some(false),
                    ..Default::default()
                })],
            ));
        }

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                tool_call_id: tool_call_id.clone(),
                content: ChatContent::SimpleText(render_matches(&query, &matched, &unavailable)),
                tool_failed: Some(false),
                ..Default::default()
            })],
        ))
    }
}

// Schemas as text; no session state modified (cache-safe).
fn render_matches(query: &str, matched: &[(String, String, Value)], unavailable: &str) -> String {
    let mut lines = vec![format!(
        "Found {} MCP tool(s) matching '{}'. Use `mcp_call` to execute them.\n",
        matched.len(),
        query
    )];
    for (name, description, schema) in matched {
        lines.push(format!(
            "### {}\n{}\n\nInput schema:\n```json\n{}\n```\n",
            name,
            description,
            serde_json::to_string_pretty(schema).unwrap_or_default()
        ));
    }
    if !unavailable.is_empty() {
        lines.push(format!("Note:{unavailable}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod render_tests {
    use super::*;

    #[test]
    fn successful_search_appends_unavailable_summary_only_when_present() {
        let matched = vec![(
            "mcp_a_read".into(),
            "Read".into(),
            json!({"type": "object"}),
        )];
        let summary = " 1 MCP server(s) have no usable connection or catalog: b — disconnected. \
                       Their tools will appear once connected.";
        let with = render_matches("read", &matched, summary);
        assert!(with.starts_with("Found 1 MCP tool(s) matching 'read'"));
        assert!(with.contains("### mcp_a_read"));
        assert!(with.ends_with(&format!("Note:{summary}")));
        let without = render_matches("read", &matched, "");
        assert!(!without.contains("Note:"));
        assert!(!without.contains("no usable connection"));
    }
}
