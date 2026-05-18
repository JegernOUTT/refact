use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

static API_KEY_PATTERNS: &[&str] = &[
    r#""api_key"\s*:\s*"[^"]+""#,
    r#""token"\s*:\s*"[^"]+""#,
    r#""secret"\s*:\s*"[^"]+""#,
    r"sk-[a-zA-Z0-9]{20,}",
    r"Bearer \S+",
];

fn redact_config(text: &str) -> String {
    let mut result = text.to_string();
    for pat in API_KEY_PATTERNS {
        if let Ok(re) = regex::Regex::new(pat) {
            result = re.replace_all(&result, "[REDACTED]").to_string();
        }
    }
    result
}

pub struct ToolBuddyGetContext {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolBuddyGetContext {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_get_internal_context".to_string(),
            display_name: "Buddy Get Internal Context".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Return sanitized internal Refact configuration and context. API keys and secrets are redacted. Useful for understanding project setup, available integrations, and configuration state.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "sections": {
                        "type": "array",
                        "description": "Which sections to include. Default: all. Options: local_config, global_config, integrations, mcp_servers, modes, setup_status, project_info.",
                        "items": {
                            "type": "string",
                            "enum": ["local_config", "global_config", "integrations", "mcp_servers", "modes", "setup_status", "project_info"]
                        }
                    }
                },
                "required": []
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let all_sections = vec![
            "local_config",
            "global_config",
            "integrations",
            "mcp_servers",
            "modes",
            "setup_status",
            "project_info",
        ];

        let requested: Vec<String> = match args.get("sections") {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            Some(Value::String(s)) => serde_json::from_str::<Vec<String>>(s)
                .unwrap_or_else(|_| all_sections.iter().map(|s| s.to_string()).collect()),
            _ => all_sections.iter().map(|s| s.to_string()).collect(),
        };

        let gcx = ccx.lock().await.app.gcx.clone();
        let (config_dir, project_dirs) = {
            (
                gcx.config_dir.clone(),
                crate::files_correction::get_project_dirs(gcx.clone()),
            )
        };
        let project_dirs = project_dirs.await;
        let project_root = project_dirs.into_iter().next();

        let mut result = serde_json::Map::new();

        for section in &requested {
            match section.as_str() {
                "global_config" => {
                    let content = read_dir_summary(&config_dir, 3).await;
                    result.insert(section.clone(), Value::String(redact_config(&content)));
                }
                "local_config" => {
                    let content = match &project_root {
                        Some(root) => read_dir_summary(&root.join(".refact"), 3).await,
                        None => "no project root found".to_string(),
                    };
                    result.insert(section.clone(), Value::String(redact_config(&content)));
                }
                "integrations" => {
                    let content = read_integrations_summary(gcx.clone()).await;
                    result.insert(section.clone(), Value::String(redact_config(&content)));
                }
                "mcp_servers" => {
                    let content = read_mcp_summary(&config_dir).await;
                    result.insert(section.clone(), Value::String(redact_config(&content)));
                }
                "modes" => {
                    let content = read_modes_summary(gcx.clone()).await;
                    result.insert(section.clone(), Value::String(content));
                }
                "setup_status" => {
                    let content = read_setup_status(gcx.clone(), &project_root).await;
                    result.insert(section.clone(), Value::String(content));
                }
                "project_info" => {
                    let content = match &project_root {
                        Some(root) => format!("project_root: {:?}", root),
                        None => "no project root".to_string(),
                    };
                    result.insert(section.clone(), Value::String(content));
                }
                _ => {}
            }
        }

        let output = serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string());

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(output),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

async fn read_dir_summary(dir: &std::path::Path, depth: usize) -> String {
    if depth == 0 || !dir.exists() {
        return format!("{:?} (not found)", dir);
    }
    let mut lines = vec![format!("{:?}:", dir)];
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return format!("{:?} (read error)", dir);
    };
    let mut count = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        if path.is_dir() {
            lines.push(format!("  {}/", name));
        } else {
            let size = tokio::fs::metadata(&path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            lines.push(format!("  {} ({}B)", name, size));
        }
        count += 1;
        if count >= 30 {
            lines.push("  ... (truncated)".to_string());
            break;
        }
    }
    lines.join("\n")
}

async fn read_integrations_summary(
    gcx: Arc<crate::global_context::GlobalContext>,
) -> String {
    let config_dir = gcx.config_dir.clone();
    let integr_dir = config_dir.join("integrations.d");
    if !integr_dir.exists() {
        return "no integrations.d directory found".to_string();
    }
    let Ok(mut entries) = tokio::fs::read_dir(&integr_dir).await else {
        return "failed to read integrations.d".to_string();
    };
    let mut names = vec![];
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) {
            names.push(name);
        }
    }
    names.sort();
    format!("integrations: {}", names.join(", "))
}

async fn read_mcp_summary(config_dir: &std::path::Path) -> String {
    let mcp_dir = config_dir.join("integrations.d");
    let Ok(mut entries) = tokio::fs::read_dir(&mcp_dir).await else {
        return "no MCP configurations found".to_string();
    };
    let mut mcp_names = vec![];
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("mcp_") {
            mcp_names.push(name);
        }
    }
    if mcp_names.is_empty() {
        return "no MCP servers configured".to_string();
    }
    format!("MCP servers: {}", mcp_names.join(", "))
}

async fn read_modes_summary(
    gcx: Arc<crate::global_context::GlobalContext>,
) -> String {
    use crate::yaml_configs::customization_registry::get_project_registry;
    let Some(registry) = get_project_registry(gcx).await else {
        return "no mode registry available".to_string();
    };
    let mut modes: Vec<String> = registry.modes.keys().cloned().collect();
    modes.sort();
    format!("available modes: {}", modes.join(", "))
}

async fn read_setup_status(
    gcx: Arc<crate::global_context::GlobalContext>,
    project_root: &Option<std::path::PathBuf>,
) -> String {
    let Some(root) = project_root else {
        return "no project root".to_string();
    };
    let agents_md = root.join("AGENTS.md");
    let has_agents_md = tokio::fs::try_exists(&agents_md).await.unwrap_or(false);

    let buddy_arc = gcx.buddy.clone();
    let lock = buddy_arc.lock().await;
    let stage = lock
        .as_ref()
        .map(|s| s.state.progression.stage_name.clone())
        .unwrap_or_default();
    let xp = lock.as_ref().map(|s| s.state.progression.xp).unwrap_or(0);

    format!(
        "agents_md_exists: {}, buddy_stage: {}, buddy_xp: {}",
        has_agents_md, stage, xp
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buddy_get_context_sections() {
        let all = vec![
            "local_config",
            "global_config",
            "integrations",
            "mcp_servers",
            "modes",
            "setup_status",
            "project_info",
        ];
        assert_eq!(all.len(), 7);
    }

    #[test]
    fn test_redact_config() {
        let raw = r#"{"api_key": "sk-secret123456789012345", "name": "test"}"#;
        let redacted = redact_config(raw);
        assert!(!redacted.contains("sk-secret123456789012345"));
        assert!(redacted.contains("[REDACTED]"));
        assert!(redacted.contains("test"));
    }
}
