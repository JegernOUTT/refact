use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::actor::redact_sensitive;
use crate::buddy::types::BuddyThreadMeta;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::internal_roles::{event, EventSubkind};
use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolBuddyLaunchInvestigation {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolBuddyLaunchInvestigation {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_launch_investigation".to_string(),
            display_name: "Buddy Launch Investigation".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Launch a new Buddy investigation chat preloaded with facts, diagnostic IDs, and a redacted log excerpt. Returns the new chat_id.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fact_keys": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Fact keys to preload into the investigation."
                    },
                    "diagnostic_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Diagnostic IDs to include in the investigation context."
                    },
                    "log_excerpt": {
                        "type": "string",
                        "description": "Relevant log snippet (will be redacted before inclusion)."
                    },
                    "config_summary": {
                        "type": "string",
                        "description": "Brief summary of relevant configuration."
                    },
                    "initial_user_message": {
                        "type": "string",
                        "description": "Initial message to start the investigation chat."
                    }
                },
                "required": ["initial_user_message"]
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
        let initial_user_message = args
            .get("initial_user_message")
            .and_then(|v| v.as_str())
            .ok_or("argument `initial_user_message` is missing or not a string")?
            .to_string();

        let fact_keys: Vec<String> = args
            .get("fact_keys")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let diagnostic_ids: Vec<String> = args
            .get("diagnostic_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let log_excerpt = args
            .get("log_excerpt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let config_summary = args
            .get("config_summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut msg_parts = vec![initial_user_message.clone()];
        if !fact_keys.is_empty() {
            msg_parts.push(format!("Fact keys: {}", fact_keys.join(", ")));
        }
        if !diagnostic_ids.is_empty() {
            msg_parts.push(format!("Diagnostic IDs: {}", diagnostic_ids.join(", ")));
        }
        if !log_excerpt.is_empty() {
            let redacted = redact_sensitive(&log_excerpt);
            msg_parts.push(format!("Log excerpt:\n{}", redacted));
        }
        if !config_summary.is_empty() {
            msg_parts.push(format!("Config summary: {}", config_summary));
        }
        let user_text = msg_parts.join("\n\n");

        let chat_facade = ccx.lock().await.app.chat.facade.clone();
        let chat_id = Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();

        let initial_message = event(
            EventSubkind::SystemNotice,
            "tool.buddy_launch_investigation",
            serde_json::json!({
                "chat_id": &chat_id,
                "fact_keys": &fact_keys,
                "diagnostic_ids": &diagnostic_ids,
                "has_log_excerpt": !log_excerpt.is_empty(),
                "has_config_summary": !config_summary.is_empty(),
                "created_at": &created_at,
            }),
            user_text,
        );

        let snapshot = TrajectorySnapshot {
            goal: None,
            goal_ledger: Vec::new(),
            goal_verification_blocked_until_ms: None,
            chat_id: chat_id.clone(),
            title: "Investigation".to_string(),
            model: String::new(),
            mode: "buddy".to_string(),
            tool_use: "agent".to_string(),
            messages: vec![initial_message],
            created_at,
            boost_reasoning: false,
            checkpoints_enabled: false,
            context_tokens_cap: None,
            include_project_info: true,
            is_title_generated: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            autonomous_no_confirm: false,
            version: 1,
            task_meta: None,
            worktree: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            reasoning_effort: None,
            thinking_budget: None,
            temperature: None,
            frequency_penalty: None,
            max_tokens: None,
            parallel_tool_calls: None,
            previous_response_id: None,
            active_skill: None,
            auto_enrichment_enabled: Some(true),
            buddy_meta: Some(BuddyThreadMeta {
                is_buddy_chat: true,
                buddy_chat_kind: "investigation".to_string(),
                workflow_id: None,
            }),
            auto_compact_enabled: None,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
        };

        chat_facade
            .save_trajectory_snapshot(snapshot)
            .await
            .map_err(|e| format!("failed to create investigation chat: {}", e))?;

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!(
                    "Investigation chat created (id: {}). Open the chat to start the investigation.",
                    chat_id
                )),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                preserve: Some(true),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
