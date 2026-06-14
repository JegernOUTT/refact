use serde::Serialize;
use serde_json::Value;

use crate::client::{
    KnowledgeGraphResponse, KnowledgeNode, McpServerInfoResponse, McpViewData,
    SlashCommandsListResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyView {
    Mcp,
    Skills,
    Memories,
}

impl ReadOnlyView {
    pub fn command_name(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Skills => "skills",
            Self::Memories => "memories",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Mcp => "MCP",
            Self::Skills => "Skills",
            Self::Memories => "Memories",
        }
    }

    pub fn loading_overlay(self) -> ViewOverlay {
        ViewOverlay {
            title: self.title().to_string(),
            rendered_lines: vec![format!("Loading /{} data…", self.command_name())],
            raw_lines: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewOverlay {
    pub title: String,
    pub rendered_lines: Vec<String>,
    pub raw_lines: Vec<String>,
}

pub fn mcp_overlay(data: &McpViewData) -> ViewOverlay {
    let mut lines = vec![
        "MCP".to_string(),
        "Configured servers from /v1/integrations; live status from /v1/mcp-server-info."
            .to_string(),
        format!("Servers: {}", data.servers.len()),
    ];
    if !data.error_log.is_empty() {
        lines.push(format!("Integration warnings: {}", data.error_log.len()));
    }
    lines.push(String::new());

    if data.servers.is_empty() {
        lines.push("No configured MCP servers found via /v1/integrations.".to_string());
    } else {
        for server in &data.servers {
            let scope = if server.project_path.is_empty() {
                "global".to_string()
            } else {
                format!("project {}", server.project_path)
            };
            match &server.info {
                Some(info) => push_mcp_server_lines(&mut lines, server, info, &scope),
                None => {
                    lines.push(format!(
                        "• {} [{}] — status unavailable",
                        server.name, server.transport
                    ));
                    lines.push(format!("  scope: {scope}"));
                    lines.push(format!("  config: {}", server.config_path));
                    let error = server
                        .error
                        .as_deref()
                        .unwrap_or("backend did not return details");
                    lines.push(format!("  notice: {error}"));
                }
            }
            lines.push(String::new());
        }
    }

    ViewOverlay {
        title: "MCP".to_string(),
        rendered_lines: trim_trailing_blank(lines),
        raw_lines: raw_lines(data),
    }
}

pub fn skills_overlay(data: &SlashCommandsListResponse) -> ViewOverlay {
    let mut lines = vec![
        "Skills".to_string(),
        "Available skills and slash commands from /v1/slash-commands.".to_string(),
        format!(
            "Skills: {} · Slash commands: {}",
            data.skills.len(),
            data.commands.len()
        ),
        String::new(),
        "Skills".to_string(),
    ];

    let mut skills = data.skills.iter().collect::<Vec<_>>();
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    if skills.is_empty() {
        lines.push("No skills returned by /v1/slash-commands.".to_string());
    } else {
        for skill in skills {
            let invocable = if skill.user_invocable {
                "user-invocable"
            } else {
                "model-only"
            };
            lines.push(format!(
                "• /{} — {} · {}",
                skill.name, invocable, skill.source
            ));
            if !skill.description.trim().is_empty() {
                lines.push(format!("  {}", compact_text(&skill.description)));
            }
        }
    }

    lines.push(String::new());
    lines.push("Slash commands".to_string());
    let mut commands = data.commands.iter().collect::<Vec<_>>();
    commands.sort_by(|left, right| left.name.cmp(&right.name));
    if commands.is_empty() {
        lines.push("No slash commands returned by /v1/slash-commands.".to_string());
    } else {
        for command in commands {
            let hint = command.argument_hint.as_deref().unwrap_or_default();
            let title = if hint.is_empty() {
                format!("/{}", command.name)
            } else {
                format!("/{} {}", command.name, hint)
            };
            lines.push(format!("• {title} — {}", command.source));
            if !command.description.trim().is_empty() {
                lines.push(format!("  {}", compact_text(&command.description)));
            }
        }
    }

    ViewOverlay {
        title: "Skills".to_string(),
        rendered_lines: trim_trailing_blank(lines),
        raw_lines: raw_lines(data),
    }
}

pub fn memories_overlay(data: &KnowledgeGraphResponse) -> ViewOverlay {
    let stats = &data.stats;
    let mut lines = vec![
        "Memories".to_string(),
        "Knowledge graph summary from /v1/knowledge-graph.".to_string(),
        format!(
            "Docs: {} active / {} total · Tags: {} · Files: {} · Entities: {} · Edges: {}",
            stats.active_docs,
            stats.doc_count,
            stats.tag_count,
            stats.file_count,
            stats.entity_count,
            stats.edge_count
        ),
    ];
    if stats.deprecated_docs > 0 || stats.trajectory_count > 0 {
        lines.push(format!(
            "Deprecated docs: {} · Trajectory docs: {}",
            stats.deprecated_docs, stats.trajectory_count
        ));
    }
    lines.push(String::new());

    let mut docs = data
        .nodes
        .iter()
        .filter(|node| node.node_type.starts_with("doc"))
        .collect::<Vec<_>>();
    docs.sort_by(|left, right| left.label.cmp(&right.label));
    if docs.is_empty() {
        lines.push("No knowledge documents returned by /v1/knowledge-graph.".to_string());
    } else {
        lines.push("Knowledge entries".to_string());
        for doc in docs {
            push_knowledge_doc_lines(&mut lines, doc);
        }
    }

    let mut tags = data
        .nodes
        .iter()
        .filter(|node| node.node_type == "tag")
        .map(|node| node.label.as_str())
        .collect::<Vec<_>>();
    tags.sort_unstable();
    if !tags.is_empty() {
        lines.push(String::new());
        lines.push(format!("Tags: {}", tags.join(", ")));
    }

    ViewOverlay {
        title: "Memories".to_string(),
        rendered_lines: trim_trailing_blank(lines),
        raw_lines: raw_lines(data),
    }
}

fn push_mcp_server_lines(
    lines: &mut Vec<String>,
    server: &crate::client::McpServerSummary,
    info: &McpServerInfoResponse,
    scope: &str,
) {
    let status = status_summary(&info.status);
    let auth = status_summary(&info.auth_status);
    lines.push(format!(
        "• {} [{}] — {} · auth {} · tools {} · resources {} · prompts {}",
        server.name,
        server.transport,
        status,
        auth,
        info.tools.len(),
        info.resources.len(),
        info.prompts.len()
    ));
    lines.push(format!("  scope: {scope}"));
    lines.push(format!("  config: {}", server.config_path));
    if let Some(name) = &info.server_name {
        let version = info.server_version.as_deref().unwrap_or("unknown");
        let protocol = info.protocol_version.as_deref().unwrap_or("unknown");
        lines.push(format!("  server: {name} {version} · protocol {protocol}"));
    }
    if info.tools.is_empty() {
        lines.push("  tools: none returned".to_string());
    } else {
        lines.push("  tools:".to_string());
        for tool in &info.tools {
            let name = if tool.internal_name.is_empty() {
                tool.name.as_str()
            } else {
                tool.internal_name.as_str()
            };
            if tool.description.trim().is_empty() {
                lines.push(format!("    - {name}"));
            } else {
                lines.push(format!("    - {name}: {}", compact_text(&tool.description)));
            }
        }
    }
}

fn push_knowledge_doc_lines(lines: &mut Vec<String>, doc: &KnowledgeNode) {
    let kind = doc
        .kind
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            doc.node_type
                .trim_start_matches("doc_")
                .trim_start_matches("doc")
        });
    let kind = if kind.is_empty() { "doc" } else { kind };
    let tags = doc
        .tags
        .as_ref()
        .filter(|tags| !tags.is_empty())
        .map(|tags| format!(" · tags {}", tags.join(", ")))
        .unwrap_or_default();
    let created = doc
        .created
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!(" · {value}"))
        .unwrap_or_default();
    lines.push(format!("• {} — {}{}{}", doc.label, kind, tags, created));
    if let Some(path) = doc.file_path.as_deref() {
        lines.push(format!("  {path}"));
    }
}

fn status_summary(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Object(map) => {
            let status = map
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let message = map
                .get("message")
                .or_else(|| map.get("error"))
                .and_then(Value::as_str);
            let attempt = map.get("attempt").and_then(Value::as_u64);
            match (message, attempt) {
                (Some(message), _) => format!("{status}: {message}"),
                (None, Some(attempt)) => format!("{status} attempt {attempt}"),
                (None, None) => status.to_string(),
            }
        }
        Value::Null => "unknown".to_string(),
        other => other.to_string(),
    }
}

fn compact_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn raw_lines<T: Serialize>(value: &T) -> Vec<String> {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|error| format!("failed to render raw JSON: {error}"))
        .lines()
        .map(str::to_string)
        .collect()
}

fn trim_trailing_blank(mut lines: Vec<String>) -> Vec<String> {
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{KnowledgeStats, McpServerSummary, McpToolInfo, SkillInfo};
    use serde_json::json;

    #[test]
    fn mcp_overlay_renders_empty_state_notice() {
        let overlay = mcp_overlay(&McpViewData {
            servers: Vec::new(),
            error_log: Vec::new(),
        });
        assert!(overlay
            .rendered_lines
            .iter()
            .any(|line| line.contains("No configured MCP servers")));
    }

    #[test]
    fn mcp_overlay_renders_server_status_and_tools() {
        let overlay = mcp_overlay(&McpViewData {
            servers: vec![McpServerSummary {
                name: "mcp_stdio_demo".to_string(),
                transport: "stdio".to_string(),
                project_path: String::new(),
                config_path: "/tmp/mcp.yaml".to_string(),
                error: None,
                info: Some(McpServerInfoResponse {
                    config_path: "/tmp/mcp.yaml".to_string(),
                    status: json!({"status":"connected"}),
                    auth_status: json!("authenticated"),
                    server_name: Some("demo".to_string()),
                    server_version: Some("1.0".to_string()),
                    protocol_version: Some("2024-11-05".to_string()),
                    tools: vec![McpToolInfo {
                        name: "lookup".to_string(),
                        description: "Look up docs".to_string(),
                        input_schema: json!({"type":"object"}),
                        annotations: None,
                        internal_name: "demo_lookup".to_string(),
                    }],
                    resources: Vec::new(),
                    prompts: Vec::new(),
                    capabilities: json!({}),
                    logs_tail: Vec::new(),
                    metrics: json!({}),
                }),
            }],
            error_log: Vec::new(),
        });
        let text = overlay.rendered_lines.join("\n");
        assert!(text.contains("connected"));
        assert!(text.contains("demo_lookup"));
    }

    #[test]
    fn skills_overlay_lists_skills_and_commands() {
        let overlay = skills_overlay(&SlashCommandsListResponse {
            skills: vec![SkillInfo {
                name: "explain".to_string(),
                description: "Explain code".to_string(),
                user_invocable: true,
                source: "project_refact".to_string(),
            }],
            commands: Vec::new(),
        });
        let text = overlay.rendered_lines.join("\n");
        assert!(text.contains("/explain"));
        assert!(text.contains("No slash commands returned"));
    }

    #[test]
    fn memories_overlay_lists_knowledge_docs() {
        let overlay = memories_overlay(&KnowledgeGraphResponse {
            stats: KnowledgeStats {
                doc_count: 1,
                tag_count: 1,
                active_docs: 1,
                ..Default::default()
            },
            nodes: vec![KnowledgeNode {
                id: "doc-1".to_string(),
                node_type: "doc_decision".to_string(),
                label: "A decision".to_string(),
                title: Some("A decision".to_string()),
                content: None,
                tags: Some(vec!["architecture".to_string()]),
                created: Some("2026-06-14".to_string()),
                file_path: Some(".refact/knowledge/a.md".to_string()),
                kind: Some("decision".to_string()),
            }],
            edges: Vec::new(),
        });
        let text = overlay.rendered_lines.join("\n");
        assert!(text.contains("A decision"));
        assert!(text.contains("architecture"));
    }
}
