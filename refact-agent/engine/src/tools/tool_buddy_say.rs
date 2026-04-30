use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::types::{BuddyControl, BuddySpeechItem};
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const ADVERTISED_CONTROL_ACTIONS: &[&str] = &[
    "open_setup",
    "open_setup_mode",
    "open_stats",
    "open_buddy",
    "dismiss",
];

const LEGACY_SETUP_CONTROL_ACTIONS: &[(&str, &str)] = &[
    ("open_setup_mcp", "setup_mcp"),
    ("open_setup_skills", "setup_skills"),
    ("open_setup_commands", "setup_commands"),
    ("open_setup_agents_md", "setup_agents_md"),
    ("open_setup_subagents", "setup_subagents"),
];

const VALID_SETUP_MODES: &[&str] = &[
    "setup",
    "setup_mcp",
    "setup_skills",
    "setup_commands",
    "setup_agents_md",
    "setup_subagents",
];

pub struct ToolBuddySay {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolBuddySay {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_say".to_string(),
            display_name: "Buddy Say".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Display a speech bubble message next to Buddy on the frontend. Use this to communicate status, suggestions, or greetings to the user.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The message text to display in Buddy's speech bubble."
                    },
                    "mood": {
                        "type": "string",
                        "description": "Optional mood hint: happy, concerned, excited, thinking, neutral.",
                        "enum": ["happy", "concerned", "excited", "thinking", "neutral"]
                    },
                    "scope": {
                        "type": "string",
                        "description": "Where to show: dashboard, chat, or global. Default: global.",
                        "enum": ["dashboard", "chat", "global"]
                    },
                    "persistent": {
                        "type": "boolean",
                        "description": "If true, stays until dismissed or replaced. Default: false."
                    },
                    "ttl_seconds": {
                        "type": "number",
                        "description": "Auto-dismiss after N seconds. Default: 10."
                    },
                    "dedupe_key": {
                        "type": "string",
                        "description": "Replaces previous speech with the same key."
                    }
                },
                "required": ["text"]
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
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or("argument `text` is missing or not a string")?
            .to_string();

        if text.is_empty() {
            return Err("text cannot be empty".to_string());
        }

        let mood = args
            .get("mood")
            .and_then(|v| v.as_str())
            .unwrap_or("neutral")
            .to_string();
        let scope = args
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("global")
            .to_string();
        let persistent = args
            .get("persistent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let ttl_seconds = args
            .get("ttl_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(10);
        let dedupe_key = args
            .get("dedupe_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let speech = BuddySpeechItem {
            id: Uuid::new_v4().to_string(),
            text: text.clone(),
            mood,
            scope,
            persistent,
            ttl_seconds,
            dedupe_key,
            created_at: chrono::Utc::now().to_rfc3339(),
            controls: vec![],
            chat_id: None,
        };

        let gcx = ccx.lock().await.global_context.clone();
        let buddy_arc = gcx.read().await.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        let svc = lock
            .as_mut()
            .ok_or_else(|| "buddy service not initialized".to_string())?;
        svc.update_speech(speech);

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!("Speech bubble displayed: \"{}\"", text)),
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

pub struct ToolBuddyRenderControls {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolBuddyRenderControls {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "buddy_render_controls".to_string(),
            display_name: "Buddy Render Controls".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Render interactive buttons in Buddy's speech bubble. The user can click these to trigger actions.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "controls": {
                        "type": "array",
                        "description": "List of interactive controls to render.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "description": "Unique control id."
                                },
                                "label": {
                                    "type": "string",
                                    "description": "Button text."
                                },
                                "action": {
                                    "type": "string",
                                    "description": "Action: open_setup, open_setup_mode, open_stats, open_buddy, dismiss.",
                                    "enum": ADVERTISED_CONTROL_ACTIONS
                                },
                                "action_param": {
                                    "type": "string",
                                    "description": "Required setup mode when action=open_setup_mode."
                                },
                                "style": {
                                    "type": "string",
                                    "description": "Button style: primary, secondary, danger. Default: secondary.",
                                    "enum": ["primary", "secondary", "danger"]
                                }
                            },
                            "required": ["id", "label", "action"]
                        }
                    },
                    "speech_text": {
                        "type": "string",
                        "description": "Optional text to show alongside controls."
                    },
                    "dedupe_key": {
                        "type": "string",
                        "description": "Replaces previous controls with the same key."
                    }
                },
                "required": ["controls"]
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
        let controls_val = args
            .get("controls")
            .ok_or("argument `controls` is missing")?;

        let controls: Vec<BuddyControl> = match controls_val {
            Value::Array(_) => serde_json::from_value(controls_val.clone())
                .map_err(|e| format!("failed to parse controls: {}", e))?,
            Value::String(s) => serde_json::from_str(s)
                .map_err(|e| format!("failed to parse controls JSON: {}", e))?,
            _ => return Err("controls must be an array".to_string()),
        };

        if controls.is_empty() {
            return Err("controls array cannot be empty".to_string());
        }

        for c in &controls {
            let is_advertised = ADVERTISED_CONTROL_ACTIONS.contains(&c.action.as_str());
            let is_legacy_setup = LEGACY_SETUP_CONTROL_ACTIONS
                .iter()
                .any(|(action, _)| *action == c.action.as_str());
            if !is_advertised && !is_legacy_setup {
                return Err(format!(
                    "invalid action '{}' for control '{}'",
                    c.action, c.id
                ));
            }
            if c.action == "open_setup_mode" {
                let mode = c
                    .action_param
                    .as_deref()
                    .ok_or_else(|| format!("action_param is required for control '{}'", c.id))?;
                if !VALID_SETUP_MODES.contains(&mode) {
                    return Err(format!(
                        "invalid setup mode '{}' for control '{}'",
                        mode, c.id
                    ));
                }
            }
        }

        let speech_text = args
            .get("speech_text")
            .and_then(|v| v.as_str())
            .unwrap_or("What would you like to do?")
            .to_string();
        let dedupe_key = args
            .get("dedupe_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let speech = BuddySpeechItem {
            id: uuid::Uuid::new_v4().to_string(),
            text: speech_text,
            mood: "neutral".to_string(),
            scope: "global".to_string(),
            persistent: true,
            ttl_seconds: 30,
            dedupe_key,
            created_at: chrono::Utc::now().to_rfc3339(),
            controls,
            chat_id: None,
        };

        let gcx = ccx.lock().await.global_context.clone();
        let buddy_arc = gcx.read().await.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        let svc = lock
            .as_mut()
            .ok_or_else(|| "buddy service not initialized".to_string())?;
        svc.update_speech(speech);

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(
                    "Controls rendered in Buddy's speech bubble.".to_string(),
                ),
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
