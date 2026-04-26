use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::fs;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::files_correction::get_project_dirs;

pub struct ToolTrajectoryContext {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolTrajectoryContext {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "get_trajectory_context".to_string(),
            display_name: "Get Trajectory Context".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description:
                "Get more context from a specific trajectory around given message indices."
                    .to_string(),
            input_schema: json_schema_from_params(
                &[
                    (
                        "trajectory_id",
                        "string",
                        "The trajectory ID to retrieve context from.",
                    ),
                    ("message_start", "string", "Starting message index."),
                    ("message_end", "string", "Ending message index."),
                    (
                        "expand_by",
                        "string",
                        "Number of messages to include before/after (default: 3).",
                    ),
                ],
                &["trajectory_id", "message_start", "message_end"],
            ),
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
        let trajectory_id =
            match args.get("trajectory_id") {
                Some(Value::String(s)) => s.clone(),
                _ => return Err(
                    "⚠️ Missing trajectory_id. 💡 Check .refact/trajectories/ for available IDs"
                        .to_string(),
                ),
            };

        let msg_start: usize = match args.get("message_start") {
            Some(Value::String(s)) => s
                .parse()
                .map_err(|_| "⚠️ message_start must be a number. 💡 Use 0 for first message")?,
            Some(Value::Number(n)) => {
                n.as_u64()
                    .ok_or("⚠️ message_start must be a positive number")? as usize
            }
            _ => return Err("⚠️ Missing message_start. 💡 Use 0 for first message".to_string()),
        };

        let msg_end: usize = match args.get("message_end") {
            Some(Value::String(s)) => s.parse().map_err(|_| "⚠️ message_end must be a number")?,
            Some(Value::Number(n)) => {
                n.as_u64()
                    .ok_or("⚠️ message_end must be a positive number")? as usize
            }
            _ => {
                return Err(
                    "⚠️ Missing message_end. 💡 Use knowledge() to find relevant message ranges"
                        .to_string(),
                )
            }
        };

        if msg_start > msg_end {
            return Err(format!(
                "⚠️ message_start ({}) > message_end ({}). 💡 Swap values or adjust range",
                msg_start, msg_end
            ));
        }

        let expand_by: usize = match args.get("expand_by") {
            Some(Value::String(s)) => s.parse().unwrap_or(3),
            Some(Value::Number(n)) => n.as_u64().unwrap_or(3) as usize,
            _ => 3,
        };

        let gcx = ccx.lock().await.global_context.clone();
        let project_dirs = get_project_dirs(gcx.clone()).await;
        let traj_path = project_dirs.iter()
            .map(|dir| dir.join(".refact/trajectories").join(format!("{}.json", trajectory_id)))
            .find(|p| p.exists())
            .ok_or(format!("⚠️ Trajectory '{}' not found. 💡 Check .refact/trajectories/ or use knowledge() to search", trajectory_id))?;

        let content = fs::read_to_string(&traj_path)
            .await
            .map_err(|e| format!("Failed to read trajectory: {}", e))?;

        let trajectory: Value = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse trajectory: {}", e))?;

        let messages = trajectory
            .get("messages")
            .and_then(|v| v.as_array())
            .ok_or("⚠️ No messages in trajectory. 💡 This trajectory may be empty or corrupted")?;

        if messages.is_empty() {
            return Err("⚠️ Trajectory has no messages. 💡 Try a different trajectory".to_string());
        }

        if msg_start >= messages.len() {
            return Err(format!(
                "⚠️ message_start ({}) >= total messages ({}). 💡 Use range 0-{}",
                msg_start,
                messages.len(),
                messages.len().saturating_sub(1)
            ));
        }

        let title = trajectory
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled");
        let actual_start = msg_start.saturating_sub(expand_by);
        let actual_end = (msg_end + expand_by).min(messages.len().saturating_sub(1));

        let mut output = String::new();
        output.push_str("╭──────────────────────────────────────╮\n");
        output.push_str(&format!("│ 📁 {}│\n", pad_right(&trajectory_id, 36)));
        output.push_str(&format!("│ 📌 {}│\n", pad_right(title, 36)));
        output.push_str(&format!(
            "│ 📍 Messages {}-{} (requested {}-{}) │\n",
            actual_start, actual_end, msg_start, msg_end
        ));
        output.push_str("╰──────────────────────────────────────╯\n\n");

        for (i, msg) in messages.iter().enumerate() {
            if i < actual_start || i > actual_end {
                continue;
            }

            let role = msg
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if role == "context_file" || role == "cd_instruction" || role == "system" {
                continue;
            }

            let content_text = extract_content(msg);
            if content_text.trim().is_empty() {
                continue;
            }

            let is_highlighted = i >= msg_start && i <= msg_end;
            let role_icon = match role {
                "user" => "👤",
                "assistant" => "🤖",
                "tool" => "🔧",
                _ => "💬",
            };

            if is_highlighted {
                output.push_str(&format!(
                    "┏━ {} [{}] {} ━━━━━━━━━━━━━━━━━━━━━━━\n",
                    role_icon,
                    i,
                    role.to_uppercase()
                ));
            } else {
                output.push_str(&format!(
                    "┌─ {} [{}] {} ─────────────────────────\n",
                    role_icon,
                    i,
                    role.to_uppercase()
                ));
            }
            output.push_str(&content_text);
            output.push_str("\n\n");
        }

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

fn pad_right(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.chars().take(width - 3).collect::<String>() + "..."
    } else {
        format!("{}{}", s, " ".repeat(width - len))
    }
}

fn extract_content(msg: &Value) -> String {
    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
        return content.to_string();
    }

    if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
        return content_arr
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(|t| t.as_str())
                    .or_else(|| item.get("m_content").and_then(|t| t.as_str()))
            })
            .collect::<Vec<_>>()
            .join("\n");
    }

    if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
        return tool_calls
            .iter()
            .filter_map(|tc| {
                tc.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
            })
            .map(|s| format!("[tool: {}]", s))
            .collect::<Vec<_>>()
            .join(" ");
    }

    String::new()
}
