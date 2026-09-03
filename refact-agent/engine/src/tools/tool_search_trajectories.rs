use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::memories::memories_search;

pub struct ToolSearchTrajectories {
    pub config_path: String,
}

fn hist_search_preview_notice(shown: usize, total: usize, limit: usize) -> String {
    format!(
        "\n⚠️ showing {} of {} preview characters (limit: hist_search_preview_chars = {}). 💡 Raise hist_search_preview_chars in trajectory settings or call get_trajectory_context() to expand.",
        shown, total, limit
    )
}

#[async_trait]
impl Tool for ToolSearchTrajectories {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "search_trajectories".to_string(),
            display_name: "Search Trajectories".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Search past chat trajectories for relevant patterns, solutions, and context. Returns matching trajectory IDs with message ranges that can be expanded using get_trajectory_context.".to_string(),
            input_schema: json_schema_from_params(&[("query", "string", "Search query to find relevant past conversations."), ("top_n", "string", "Maximum number of trajectories to return (default: 5).")], &["query"]),
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
        let gcx = ccx.lock().await.app.gcx.clone();

        let query = match args.get("query") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `query` is not a string: {:?}", v)),
            None => return Err("argument `query` is missing".to_string()),
        };

        let top_n: usize = match args.get("top_n") {
            Some(Value::String(s)) => s.parse().unwrap_or(5),
            Some(Value::Number(n)) => n.as_u64().unwrap_or(5) as usize,
            _ => 5,
        };

        let memories = memories_search(gcx.clone(), &query, 0, top_n, None).await?;

        let output = if memories.is_empty() {
            "No relevant trajectories found.".to_string()
        } else {
            let preview_chars = crate::runtime_settings::current().hist_search_preview_chars;
            let mut result = format!("Found {} relevant trajectories:\n\n", memories.len());
            for m in memories.iter() {
                result.push_str("───────────────────────────────────────\n");
                result.push_str(&format!("📁 {}\n", m.memid));
                if let Some(title) = &m.title {
                    result.push_str(&format!("📌 {}\n", title));
                }
                if let Some(score) = m.score {
                    result.push_str(&format!("⭐ Relevance: {:.0}%\n", score * 100.0));
                }
                if let Some((start, end)) = m.line_range {
                    result.push_str(&format!("📍 Messages: {}-{}\n", start, end));
                }
                result.push_str("\n");
                let total_chars = m.content.chars().count();
                let preview: String = m.content.chars().take(preview_chars).collect();
                result.push_str(&preview);
                if total_chars > preview_chars {
                    result.push_str(&hist_search_preview_notice(
                        preview_chars,
                        total_chars,
                        preview_chars,
                    ));
                }
                result.push_str("\n\n");
            }
            result.push_str("───────────────────────────────────────\n");
            result.push_str("💡 Use get_trajectory_context(trajectory_id, message_start, message_end) to expand.\n");
            result.push_str("\nNote: these are heuristic matches and may be unrelated.");
            result
        };

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::settings_guard::SettingsGuard;
    use serial_test::serial;

    #[test]
    #[serial(runtime_settings)]
    fn hist_search_preview_chars_setting_is_read_from_settings() {
        let _guard = SettingsGuard::install(|settings| {
            settings.hist_search_preview_chars = 25;
        });

        assert_eq!(
            crate::runtime_settings::current().hist_search_preview_chars,
            25
        );
        let notice = hist_search_preview_notice(25, 900, 25);
        assert!(
            notice.contains("showing 25 of 900 preview characters"),
            "{}",
            notice
        );
        assert!(
            notice.contains("hist_search_preview_chars = 25"),
            "{}",
            notice
        );
    }

    #[test]
    fn hist_search_preview_notice_quantifies_shown_and_total() {
        let notice = hist_search_preview_notice(400, 12_345, 400);
        assert!(
            notice.contains("showing 400 of 12345 preview characters"),
            "{}",
            notice
        );
        assert!(
            notice.contains("hist_search_preview_chars = 400"),
            "{}",
            notice
        );
    }
}
