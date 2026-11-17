use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::actor::{buddy_apply, buddy_enqueue_event, make_runtime_event, BuddyMutation};
use crate::buddy::jobs::autonomous_chats::redact_and_cap_text;
use crate::buddy::types::{BuddyActivity, BuddySpeechItem};
use crate::call_validation::ContextEnum;
use crate::global_context::GlobalContext;
use crate::tools::tools_description::{Tool, ToolDesc};

use super::{desc, result};

const VALID_MOODS: &[&str] = &[
    "playful",
    "pleased",
    "cautious",
    "curious",
    "celebratory",
    "apologetic",
    "urgent",
];
const VALID_INTENTS: &[&str] = &[
    "humor",
    "suggestion",
    "insight",
    "win",
    "error_alert",
    "greeting",
    "tour",
    "milestone",
    "memory_pulse_commentary",
];
const VALID_STATUSES: &[&str] = &["started", "in_progress", "completed", "failed"];
const VALID_PRIORITIES: &[&str] = &["low", "normal", "high", "critical"];

pub struct ToolBuddyLogActivity {
    pub config_path: String,
}

pub struct ToolBuddySpeak {
    pub config_path: String,
}

pub struct ToolBuddyRuntimeEvent {
    pub config_path: String,
}

fn required_str<'a>(args: &'a HashMap<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("argument `{}` is missing or not a string", name))
}

fn optional_str(args: &HashMap<String, Value>, name: &str) -> Option<String> {
    args.get(name).and_then(|v| v.as_str()).map(str::to_string)
}

fn cap(text: &str, max_chars: usize) -> String {
    redact_and_cap_text(text, max_chars)
}

fn validate(value: &str, allowed: &[&str], name: &str) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!("invalid {}: {}", name, value))
    }
}

async fn context(ccx: Arc<AMutex<AtCommandsContext>>) -> (Arc<GlobalContext>, String) {
    let lock = ccx.lock().await;
    (lock.app.gcx.clone(), lock.chat_id.clone())
}

async fn buddy_initialized(gcx: Arc<GlobalContext>) -> bool {
    let buddy_arc = gcx.buddy.clone();
    let initialized = buddy_arc.lock().await.is_some();
    initialized
}

#[async_trait]
impl Tool for ToolBuddyLogActivity {
    fn tool_description(&self) -> ToolDesc {
        desc(
            &self.config_path,
            "buddy_log_activity",
            "Buddy Log Activity",
            "Log a concise Buddy activity item.",
            json!({"type":"object","properties":{"title":{"type":"string","maxLength":80},"description":{"type":"string","maxLength":240},"icon":{"type":"string","default":"✨"},"mood":{"type":"string"},"chat_id":{"type":"string"}},"required":["title","description"],"additionalProperties":false}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let title = cap(required_str(args, "title")?, 80);
        let description = cap(required_str(args, "description")?, 240);
        let icon = cap(optional_str(args, "icon").as_deref().unwrap_or("✨"), 16);
        let (gcx, context_chat_id) = context(ccx).await;
        if !buddy_initialized(gcx.clone()).await {
            return Ok(result(
                tool_call_id,
                "Buddy service not initialized; activity logging ignored",
            ));
        }
        let chat_id = optional_str(args, "chat_id")
            .map(|s| cap(&s, 120))
            .filter(|s| !s.is_empty())
            .or_else(|| (!context_chat_id.is_empty()).then_some(context_chat_id));
        let activity = BuddyActivity {
            icon,
            title: title.clone(),
            description,
            timestamp: chrono::Utc::now().to_rfc3339(),
            activity_type: "buddy_tool".to_string(),
            chat_id,
            failure_category: None,
            failure_summary: None,
        };
        buddy_apply(
            crate::app_state::AppState::from_gcx(gcx).await,
            BuddyMutation {
                activity: Some(activity),
                ..Default::default()
            },
        )
        .await;
        Ok(result(tool_call_id, format!("Activity logged: {}", title)))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolBuddySpeak {
    fn tool_description(&self) -> ToolDesc {
        desc(
            &self.config_path,
            "buddy_speak",
            "Buddy Speak",
            "Update Buddy's speech bubble.",
            json!({"type":"object","properties":{"text":{"type":"string","maxLength":240},"mood":{"type":"string","enum":VALID_MOODS},"intent":{"type":"string","enum":VALID_INTENTS},"scope":{"type":"string"}},"required":["text","mood","intent"],"additionalProperties":false}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let text = cap(required_str(args, "text")?, 240);
        let mood = required_str(args, "mood")?.to_string();
        validate(&mood, VALID_MOODS, "mood")?;
        let intent = required_str(args, "intent")?.to_string();
        validate(&intent, VALID_INTENTS, "intent")?;
        let (gcx, _) = context(ccx).await;
        let buddy_arc = gcx.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        let Some(svc) = lock.as_mut() else {
            return Ok(result(
                tool_call_id,
                "Buddy service not initialized; speech ignored",
            ));
        };
        svc.update_speech_user_initiated(BuddySpeechItem {
            editorial_delivery_required: false,
            id: Uuid::new_v4().to_string(),
            text: text.clone(),
            mood,
            scope: cap(
                optional_str(args, "scope").as_deref().unwrap_or("global"),
                80,
            ),
            persistent: false,
            ttl_seconds: 10,
            dedupe_key: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            controls: vec![],
            chat_id: None,
            speech_intent: Some(intent),
        });
        Ok(result(tool_call_id, format!("Spoke: {}", text)))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolBuddyRuntimeEvent {
    fn tool_description(&self) -> ToolDesc {
        desc(
            &self.config_path,
            "buddy_runtime_event",
            "Buddy Runtime Event",
            "Enqueue a Buddy runtime event.",
            json!({"type":"object","properties":{"title":{"type":"string","maxLength":80},"description":{"type":"string","maxLength":240},"status":{"type":"string","enum":VALID_STATUSES},"dedupe_key":{"type":"string"},"priority":{"type":"string","enum":VALID_PRIORITIES,"default":"normal"},"signal_type":{"type":"string","default":"workflow"}},"required":["title","status"],"additionalProperties":false}),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let title = cap(required_str(args, "title")?, 80);
        let status = required_str(args, "status")?.to_string();
        validate(&status, VALID_STATUSES, "status")?;
        let priority = optional_str(args, "priority").unwrap_or_else(|| "normal".to_string());
        validate(&priority, VALID_PRIORITIES, "priority")?;
        let dedupe_key = optional_str(args, "dedupe_key")
            .map(|s| cap(&s, 120))
            .filter(|s| !s.is_empty());
        let (gcx, _) = context(ccx).await;
        if !buddy_initialized(gcx.clone()).await {
            return Ok(result(
                tool_call_id,
                "Buddy service not initialized; runtime event ignored",
            ));
        }
        let signal_type = cap(
            optional_str(args, "signal_type")
                .as_deref()
                .unwrap_or("workflow"),
            80,
        );
        let mut event = make_runtime_event(
            &signal_type,
            &title,
            "buddy_tool",
            dedupe_key.as_deref().unwrap_or(""),
            &status,
            Some(&priority),
        );
        if dedupe_key.is_none() {
            event.dedupe_key = None;
        }
        event.description = optional_str(args, "description").map(|s| cap(&s, 240));
        buddy_enqueue_event(crate::app_state::AppState::from_gcx(gcx).await, event).await;
        Ok(result(
            tool_call_id,
            format!("Runtime event enqueued: {}", title),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
