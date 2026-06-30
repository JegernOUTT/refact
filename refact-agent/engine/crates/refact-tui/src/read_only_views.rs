use serde::Serialize;
use serde_json::Value;

use crate::client::{
    CompetitorImportInfoResponse, CompetitorImportRunResponse, HookInfo, HooksResponse,
    ImportStatus, KnowledgeGraphResponse, KnowledgeNode, McpServerInfoResponse, McpViewData,
    SlashCommandsListResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyView {
    Mcp,
    Skills,
    Memories,
    Hooks,
    Import,
}

impl ReadOnlyView {
    pub fn command_name(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Skills => "skills",
            Self::Memories => "memories",
            Self::Hooks => "hooks",
            Self::Import => "import",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Mcp => "MCP",
            Self::Skills => "Skills",
            Self::Memories => "Memories",
            Self::Hooks => "Hooks",
            Self::Import => "Import",
        }
    }

    pub fn loading_overlay(self) -> ViewOverlay {
        ViewOverlay {
            title: self.title().to_string(),
            rendered_lines: vec![format!("Loading /{} data…", self.command_name())],
            raw_lines: Vec::new(),
            surface: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewOverlay {
    pub title: String,
    pub rendered_lines: Vec<String>,
    pub raw_lines: Vec<String>,
    pub surface: Option<ViewOverlaySurface>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewOverlaySurface {
    pub summary_lines: Vec<String>,
    pub rows: Vec<ViewOverlayRow>,
    pub empty_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewOverlayRow {
    pub name: String,
    pub description: Option<String>,
    pub category_tag: Option<String>,
    pub disabled_reason: Option<String>,
    pub is_disabled: bool,
}

pub fn mcp_overlay(data: &McpViewData) -> ViewOverlay {
    let mut summary_lines = vec![
        "Configured servers from /v1/integrations; live status from /v1/mcp-server-info."
            .to_string(),
        format!("Servers: {}", data.servers.len()),
    ];
    if !data.error_log.is_empty() {
        summary_lines.push(format!("Integration warnings: {}", data.error_log.len()));
    }
    let mut lines = overlay_lines("MCP", &summary_lines);
    let mut rows = Vec::new();
    lines.push(String::new());

    if data.servers.is_empty() {
        let empty = "No configured MCP servers found via /v1/integrations.".to_string();
        lines.push(empty.clone());
        rows.push(surface_note_row(empty));
    } else {
        for server in &data.servers {
            let scope = if server.project_path.is_empty() {
                "global".to_string()
            } else {
                format!("project {}", server.project_path)
            };
            match &server.info {
                Some(info) => {
                    push_mcp_server_lines(&mut lines, server, info, &scope);
                    push_mcp_server_rows(&mut rows, server, info, &scope);
                }
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
                    rows.push(surface_item_row(
                        server.name.clone(),
                        Some(format!(
                            "[{}] status unavailable · scope {scope}",
                            server.transport
                        )),
                    ));
                    rows.push(surface_item_row("config", Some(server.config_path.clone())));
                    rows.push(surface_item_row("notice", Some(error.to_string())));
                }
            }
            lines.push(String::new());
        }
    }

    ViewOverlay {
        title: "MCP".to_string(),
        rendered_lines: trim_trailing_blank(lines),
        raw_lines: raw_lines(data),
        surface: Some(ViewOverlaySurface {
            summary_lines,
            rows,
            empty_message: "No configured MCP servers found via /v1/integrations.".to_string(),
        }),
    }
}

pub fn skills_overlay(data: &SlashCommandsListResponse) -> ViewOverlay {
    let summary_lines = vec![
        "Available skills and slash commands from /v1/slash-commands.".to_string(),
        format!(
            "Skills: {} · Slash commands: {}",
            data.skills.len(),
            data.commands.len()
        ),
    ];
    let mut lines = overlay_lines("Skills", &summary_lines);
    let mut rows = Vec::new();
    lines.extend([String::new(), "Skills".to_string()]);
    rows.push(surface_header_row("Skills"));

    let mut skills = data.skills.iter().collect::<Vec<_>>();
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    if skills.is_empty() {
        let empty = "No skills returned by /v1/slash-commands.".to_string();
        lines.push(empty.clone());
        rows.push(surface_note_row(empty));
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
            let mut row = surface_item_row(
                format!("/{}", skill.name),
                non_empty_compact(&skill.description),
            );
            row.category_tag = Some(format!("{invocable} · {}", skill.source));
            rows.push(row);
        }
    }

    lines.push(String::new());
    lines.push("Slash commands".to_string());
    rows.push(surface_header_row("Slash commands"));
    let mut commands = data.commands.iter().collect::<Vec<_>>();
    commands.sort_by(|left, right| left.name.cmp(&right.name));
    if commands.is_empty() {
        let empty = "No slash commands returned by /v1/slash-commands.".to_string();
        lines.push(empty.clone());
        rows.push(surface_note_row(empty));
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
            let mut row = surface_item_row(title, non_empty_compact(&command.description));
            row.category_tag = Some(command.source.clone());
            rows.push(row);
        }
    }

    ViewOverlay {
        title: "Skills".to_string(),
        rendered_lines: trim_trailing_blank(lines),
        raw_lines: raw_lines(data),
        surface: Some(ViewOverlaySurface {
            summary_lines,
            rows,
            empty_message: "No skills or slash commands returned by /v1/slash-commands."
                .to_string(),
        }),
    }
}

pub fn memories_overlay(data: &KnowledgeGraphResponse) -> ViewOverlay {
    let stats = &data.stats;
    let mut summary_lines = vec![
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
        summary_lines.push(format!(
            "Deprecated docs: {} · Trajectory docs: {}",
            stats.deprecated_docs, stats.trajectory_count
        ));
    }
    let mut lines = overlay_lines("Memories", &summary_lines);
    let mut rows = Vec::new();
    lines.push(String::new());

    let mut docs = data
        .nodes
        .iter()
        .filter(|node| node.node_type.starts_with("doc"))
        .collect::<Vec<_>>();
    docs.sort_by(|left, right| left.label.cmp(&right.label));
    if docs.is_empty() {
        let empty = "No knowledge documents returned by /v1/knowledge-graph.".to_string();
        lines.push(empty.clone());
        rows.push(surface_note_row(empty));
    } else {
        lines.push("Knowledge entries".to_string());
        rows.push(surface_header_row("Knowledge entries"));
        for doc in docs {
            push_knowledge_doc_lines(&mut lines, doc);
            rows.push(knowledge_doc_row(doc));
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
        rows.push(surface_header_row("Tags"));
        rows.push(surface_item_row("tags", Some(tags.join(", "))));
    }

    ViewOverlay {
        title: "Memories".to_string(),
        rendered_lines: trim_trailing_blank(lines),
        raw_lines: raw_lines(data),
        surface: Some(ViewOverlaySurface {
            summary_lines,
            rows,
            empty_message: "No knowledge documents returned by /v1/knowledge-graph.".to_string(),
        }),
    }
}

pub fn hooks_overlay(data: &HooksResponse) -> ViewOverlay {
    let mut lines = vec![
        "Hooks".to_string(),
        "Configured local hooks from /v1/ext/hooks.".to_string(),
        format!("Hooks: {}", data.hooks.len()),
    ];
    if !data.file_path.trim().is_empty() {
        lines.push(format!("File: {}", data.file_path));
    }
    lines.push(String::new());

    if data.hooks.is_empty() {
        lines.push("No configured hooks returned by /v1/ext/hooks.".to_string());
    } else {
        let mut hooks = data.hooks.iter().collect::<Vec<_>>();
        hooks.sort_by(|left, right| {
            left.event
                .cmp(&right.event)
                .then_with(|| left.command.cmp(&right.command))
        });
        for hook in hooks {
            push_hook_lines(&mut lines, hook);
        }
    }

    ViewOverlay {
        title: "Hooks".to_string(),
        rendered_lines: trim_trailing_blank(lines),
        raw_lines: raw_lines(data),
        surface: None,
    }
}

pub fn import_sources_overlay(data: &CompetitorImportInfoResponse) -> ViewOverlay {
    let mut lines = vec![
        "Import".to_string(),
        "Competitor import sources from /v1/ext/competitor-import.".to_string(),
        format!("Sources: {}", data.sources.len()),
        String::new(),
    ];

    if data.sources.is_empty() {
        lines.push(
            "No competitor import sources returned by /v1/ext/competitor-import.".to_string(),
        );
    } else {
        for source in &data.sources {
            let label = if source.label.trim().is_empty() {
                source.id.as_str()
            } else {
                source.label.as_str()
            };
            lines.push(format!("• {} — {}", source.id, label));
            if source.roots.is_empty() {
                lines.push("  roots: none reported".to_string());
            } else {
                lines.push(format!("  roots: {}", source.roots.join(", ")));
            }
        }
        lines.push(String::new());
        lines.push("Run /import <source> to import into this project.".to_string());
        lines.push("Use /import <source> global for global customizations.".to_string());
        lines.push("Use /import all to scan every source.".to_string());
    }

    ViewOverlay {
        title: "Import".to_string(),
        rendered_lines: trim_trailing_blank(lines),
        raw_lines: raw_lines(data),
        surface: None,
    }
}

pub fn import_run_overlay(data: &CompetitorImportRunResponse) -> ViewOverlay {
    let report = &data.report;
    let source = data.source.as_deref().unwrap_or("all");
    let mut lines = vec![
        "Import".to_string(),
        format!("Imported {source} customizations in {} scope.", data.scope),
        format!(
            "Discovered: {} · created {} · updated {} · unchanged {} · stale {}",
            report.discovered_candidates,
            import_status_count(report, ImportStatus::Created),
            import_status_count(report, ImportStatus::Updated),
            import_status_count(report, ImportStatus::Unchanged),
            import_status_count(report, ImportStatus::Stale),
        ),
        format!(
            "Conflicts: {} · user-modified {} · unsupported {} · errors {}",
            import_status_count(report, ImportStatus::Conflict),
            import_status_count(report, ImportStatus::UserModified),
            import_status_count(report, ImportStatus::Unsupported),
            import_status_count(report, ImportStatus::Error),
        ),
    ];
    if let Some(completed_at) = report.completed_at.as_deref() {
        lines.push(format!("Completed: {completed_at}"));
    }
    lines.push(String::new());

    if report.top_issues.is_empty() {
        lines.push("No issues reported.".to_string());
    } else {
        lines.push("Top issues".to_string());
        for issue in &report.top_issues {
            let competitor = issue.competitor.as_deref().unwrap_or("unknown");
            let kind = issue.kind.as_deref().unwrap_or("item");
            let path = issue.path.as_deref().unwrap_or("no path");
            lines.push(format!(
                "• {:?} · {competitor} {kind} · {path}",
                issue.status
            ));
            if !issue.message.trim().is_empty() {
                lines.push(format!("  {}", compact_text(&issue.message)));
            }
        }
    }

    ViewOverlay {
        title: "Import".to_string(),
        rendered_lines: trim_trailing_blank(lines),
        raw_lines: raw_lines(data),
        surface: None,
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

fn push_mcp_server_rows(
    rows: &mut Vec<ViewOverlayRow>,
    server: &crate::client::McpServerSummary,
    info: &McpServerInfoResponse,
    scope: &str,
) {
    let status = status_summary(&info.status);
    let auth = status_summary(&info.auth_status);
    let mut server_row = surface_item_row(
        server.name.clone(),
        Some(format!(
            "[{}] {} · auth {} · tools {} · resources {} · prompts {} · scope {scope}",
            server.transport,
            status,
            auth,
            info.tools.len(),
            info.resources.len(),
            info.prompts.len()
        )),
    );
    server_row.category_tag = Some(server.transport.clone());
    rows.push(server_row);
    rows.push(surface_item_row("config", Some(server.config_path.clone())));
    if let Some(name) = &info.server_name {
        let version = info.server_version.as_deref().unwrap_or("unknown");
        let protocol = info.protocol_version.as_deref().unwrap_or("unknown");
        rows.push(surface_item_row(
            "server",
            Some(format!("{name} {version} · protocol {protocol}")),
        ));
    }
    rows.push(surface_header_row("Tools"));
    if info.tools.is_empty() {
        rows.push(surface_note_row("tools: none returned".to_string()));
    } else {
        for tool in &info.tools {
            let name = if tool.internal_name.is_empty() {
                tool.name.clone()
            } else {
                tool.internal_name.clone()
            };
            rows.push(surface_item_row(name, non_empty_compact(&tool.description)));
        }
    }
}

fn push_knowledge_doc_lines(lines: &mut Vec<String>, doc: &KnowledgeNode) {
    let kind = knowledge_doc_kind(doc);
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

fn knowledge_doc_row(doc: &KnowledgeNode) -> ViewOverlayRow {
    let mut parts = Vec::new();
    if let Some(tags) = doc.tags.as_ref().filter(|tags| !tags.is_empty()) {
        parts.push(format!("tags {}", tags.join(", ")));
    }
    if let Some(created) = doc
        .created
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(created.to_string());
    }
    if let Some(path) = doc
        .file_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(path.to_string());
    }
    let mut row = surface_item_row(
        doc.label.clone(),
        (!parts.is_empty()).then(|| parts.join(" · ")),
    );
    row.category_tag = Some(knowledge_doc_kind(doc));
    row
}

fn knowledge_doc_kind(doc: &KnowledgeNode) -> String {
    let kind = doc
        .kind
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            doc.node_type
                .trim_start_matches("doc_")
                .trim_start_matches("doc")
        });
    if kind.is_empty() {
        "doc".to_string()
    } else {
        kind.to_string()
    }
}

fn overlay_lines(title: &str, summary_lines: &[String]) -> Vec<String> {
    let mut lines = vec![title.to_string()];
    lines.extend(summary_lines.iter().cloned());
    lines
}

fn surface_header_row(name: impl Into<String>) -> ViewOverlayRow {
    ViewOverlayRow {
        name: name.into(),
        description: None,
        category_tag: None,
        disabled_reason: None,
        is_disabled: true,
    }
}

fn surface_note_row(message: String) -> ViewOverlayRow {
    ViewOverlayRow {
        name: message,
        description: None,
        category_tag: None,
        disabled_reason: None,
        is_disabled: true,
    }
}

fn surface_item_row(name: impl Into<String>, description: Option<String>) -> ViewOverlayRow {
    ViewOverlayRow {
        name: name.into(),
        description,
        category_tag: None,
        disabled_reason: None,
        is_disabled: false,
    }
}

fn non_empty_compact(value: &str) -> Option<String> {
    let compact = compact_text(value);
    (!compact.is_empty()).then_some(compact)
}

fn push_hook_lines(lines: &mut Vec<String>, hook: &HookInfo) {
    let matcher = hook
        .matcher
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("*");
    let timeout = hook
        .timeout
        .map(|timeout| format!(" · timeout {timeout}s"))
        .unwrap_or_default();
    lines.push(format!("• {} · matcher {matcher}{timeout}", hook.event));
    lines.push(format!("  {}", compact_text(&hook.command)));
}

fn import_status_count(data: &crate::client::ImportReport, status: ImportStatus) -> usize {
    data.status_counts.get(&status).copied().unwrap_or_default()
}

pub fn import_run_notice(data: &CompetitorImportRunResponse) -> String {
    let source = data.source.as_deref().unwrap_or("all");
    let created = import_status_count(&data.report, ImportStatus::Created);
    let updated = import_status_count(&data.report, ImportStatus::Updated);
    let errors = import_status_count(&data.report, ImportStatus::Error);
    format!(
        "/import {source} {} complete: discovered {}, created {}, updated {}, errors {}",
        data.scope, data.report.discovered_candidates, created, updated, errors
    )
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
    use crate::client::{
        CompetitorImportInfoResponse, CompetitorImportRunResponse, CompetitorImportSourceInfo,
        HookInfo, HooksResponse, ImportReport, KnowledgeStats, McpServerSummary, McpToolInfo,
        SkillInfo,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

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

    #[test]
    fn hooks_overlay_renders_empty_state_notice() {
        let overlay = hooks_overlay(&HooksResponse {
            hooks: Vec::new(),
            raw_content: String::new(),
            file_path: "/repo/.refact/hooks.yaml".to_string(),
        });
        let text = overlay.rendered_lines.join("\n");
        assert!(text.contains("No configured hooks"));
        assert!(text.contains("/repo/.refact/hooks.yaml"));
    }

    #[test]
    fn hooks_overlay_lists_hook_details() {
        let overlay = hooks_overlay(&HooksResponse {
            hooks: vec![HookInfo {
                event: "PreToolUse".to_string(),
                matcher: Some("Bash".to_string()),
                command: "./check.sh".to_string(),
                timeout: Some(30),
            }],
            raw_content: "hooks: {}".to_string(),
            file_path: "/repo/.refact/hooks.yaml".to_string(),
        });
        let text = overlay.rendered_lines.join("\n");
        assert!(text.contains("PreToolUse"));
        assert!(text.contains("matcher Bash"));
        assert!(text.contains("./check.sh"));
    }

    #[test]
    fn import_sources_overlay_lists_available_sources() {
        let overlay = import_sources_overlay(&CompetitorImportInfoResponse {
            sources: vec![CompetitorImportSourceInfo {
                id: "claude_code".to_string(),
                label: "Claude Code".to_string(),
                roots: vec!["~/.claude".to_string(), "<project>/.claude".to_string()],
            }],
        });
        let text = overlay.rendered_lines.join("\n");
        assert!(text.contains("claude_code"));
        assert!(text.contains("Run /import <source>"));
    }

    #[test]
    fn import_run_overlay_summarizes_status_counts() {
        let mut status_counts = BTreeMap::new();
        status_counts.insert(ImportStatus::Created, 2);
        status_counts.insert(ImportStatus::Error, 1);
        let data = CompetitorImportRunResponse {
            scope: "project".to_string(),
            source: Some("claude_code".to_string()),
            report: ImportReport {
                completed_at: None,
                reported_sources: Vec::new(),
                discovered_candidates: 3,
                status_counts,
                competitor_counts: BTreeMap::new(),
                kind_counts: BTreeMap::new(),
                top_issues: Vec::new(),
            },
        };
        let text = import_run_overlay(&data).rendered_lines.join("\n");
        assert!(text.contains("Discovered: 3"));
        assert!(text.contains("created 2"));
        assert!(text.contains("errors 1"));
        assert!(import_run_notice(&data).contains("/import claude_code project complete"));
    }
}
