use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::actor::{buddy_apply, buddy_enqueue_event, make_runtime_event, BuddyMutation};
use crate::buddy::jobs::autonomous_chats::redact_and_cap_text;
use crate::buddy::types::{BuddyActivity, BuddySpeechItem};
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const VALID_MOODS: &[&str] = &["playful", "pleased", "cautious", "curious", "celebratory", "apologetic", "urgent"];
const VALID_INTENTS: &[&str] = &["humor", "suggestion", "insight", "win", "error_alert", "greeting", "tour", "milestone", "memory_pulse_commentary"];
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

fn desc(config_path: &str, name: &str, display_name: &str, description: &str, input_schema: Value) -> ToolDesc {
    ToolDesc {
        name: name.to_string(),
        display_name: display_name.to_string(),
        source: ToolSource { source_type: ToolSourceType::Builtin, config_path: config_path.to_string() },
        experimental: false,
        allow_parallel: false,
        description: description.to_string(),
        input_schema,
        output_schema: None,
        annotations: None,
    }
}

fn required_str<'a>(args: &'a HashMap<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name).and_then(|v| v.as_str()).ok_or_else(|| format!("argument `{}` is missing or not a string", name))
}

fn optional_str(args: &HashMap<String, Value>, name: &str) -> Option<String> {
    args.get(name).and_then(|v| v.as_str()).map(str::to_string)
}

fn cap(text: &str, max_chars: usize) -> String {
    redact_and_cap_text(text, max_chars)
}

fn validate(value: &str, allowed: &[&str], name: &str) -> Result<(), String> {
    if allowed.contains(&value) { Ok(()) } else { Err(format!("invalid {}: {}", name, value)) }
}

fn ok_message(tool_call_id: &String, content: impl Into<String>) -> Result<(bool, Vec<ContextEnum>), String> {
    Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content.into()),
        tool_calls: None,
        tool_call_id: tool_call_id.clone(),
        ..Default::default()
    })]))
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
        desc(&self.config_path, "buddy_log_activity", "Buddy Log Activity", "Log a concise Buddy activity item.", json!({"type":"object","properties":{"title":{"type":"string","maxLength":80},"description":{"type":"string","maxLength":240},"icon":{"type":"string","default":"✨"},"mood":{"type":"string"},"chat_id":{"type":"string"}},"required":["title","description"],"additionalProperties":false}))
    }

    async fn tool_execute(&mut self, ccx: Arc<AMutex<AtCommandsContext>>, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<(bool, Vec<ContextEnum>), String> {
        let title = cap(required_str(args, "title")?, 80);
        let description = cap(required_str(args, "description")?, 240);
        let icon = cap(optional_str(args, "icon").as_deref().unwrap_or("✨"), 16);
        let (gcx, context_chat_id) = context(ccx).await;
        if !buddy_initialized(gcx.clone()).await {
            return ok_message(tool_call_id, "Buddy service not initialized; activity logging ignored");
        }
        let chat_id = optional_str(args, "chat_id").map(|s| cap(&s, 120)).filter(|s| !s.is_empty()).or_else(|| (!context_chat_id.is_empty()).then_some(context_chat_id));
        let activity = BuddyActivity { icon, title: title.clone(), description, timestamp: chrono::Utc::now().to_rfc3339(), activity_type: "buddy_tool".to_string(), chat_id };
        buddy_apply(crate::app_state::AppState::from_gcx(gcx).await, BuddyMutation { activity: Some(activity), ..Default::default() }).await;
        ok_message(tool_call_id, format!("Activity logged: {}", title))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }
}

#[async_trait]
impl Tool for ToolBuddySpeak {
    fn tool_description(&self) -> ToolDesc {
        desc(&self.config_path, "buddy_speak", "Buddy Speak", "Update Buddy's speech bubble.", json!({"type":"object","properties":{"text":{"type":"string","maxLength":240},"mood":{"type":"string","enum":VALID_MOODS},"intent":{"type":"string","enum":VALID_INTENTS},"scope":{"type":"string"}},"required":["text","mood","intent"],"additionalProperties":false}))
    }

    async fn tool_execute(&mut self, ccx: Arc<AMutex<AtCommandsContext>>, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<(bool, Vec<ContextEnum>), String> {
        let text = cap(required_str(args, "text")?, 240);
        let mood = required_str(args, "mood")?.to_string();
        validate(&mood, VALID_MOODS, "mood")?;
        validate(required_str(args, "intent")?, VALID_INTENTS, "intent")?;
        let (gcx, _) = context(ccx).await;
        let buddy_arc = gcx.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        let Some(svc) = lock.as_mut() else {
            return ok_message(tool_call_id, "Buddy service not initialized; speech ignored");
        };
        svc.update_speech(BuddySpeechItem {
            id: Uuid::new_v4().to_string(),
            text: text.clone(),
            mood,
            scope: cap(optional_str(args, "scope").as_deref().unwrap_or("global"), 80),
            persistent: false,
            ttl_seconds: 10,
            dedupe_key: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            controls: vec![],
            chat_id: None,
        });
        ok_message(tool_call_id, format!("Spoke: {}", text))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }
}

#[async_trait]
impl Tool for ToolBuddyRuntimeEvent {
    fn tool_description(&self) -> ToolDesc {
        desc(&self.config_path, "buddy_runtime_event", "Buddy Runtime Event", "Enqueue a Buddy runtime event.", json!({"type":"object","properties":{"title":{"type":"string","maxLength":80},"description":{"type":"string","maxLength":240},"status":{"type":"string","enum":VALID_STATUSES},"dedupe_key":{"type":"string"},"priority":{"type":"string","enum":VALID_PRIORITIES,"default":"normal"},"signal_type":{"type":"string","default":"workflow"}},"required":["title","status"],"additionalProperties":false}))
    }

    async fn tool_execute(&mut self, ccx: Arc<AMutex<AtCommandsContext>>, tool_call_id: &String, args: &HashMap<String, Value>) -> Result<(bool, Vec<ContextEnum>), String> {
        let title = cap(required_str(args, "title")?, 80);
        let status = required_str(args, "status")?.to_string();
        validate(&status, VALID_STATUSES, "status")?;
        let priority = optional_str(args, "priority").unwrap_or_else(|| "normal".to_string());
        validate(&priority, VALID_PRIORITIES, "priority")?;
        let dedupe_key = optional_str(args, "dedupe_key").map(|s| cap(&s, 120)).filter(|s| !s.is_empty());
        let (gcx, _) = context(ccx).await;
        if !buddy_initialized(gcx.clone()).await {
            return ok_message(tool_call_id, "Buddy service not initialized; runtime event ignored");
        }
        let signal_type = cap(optional_str(args, "signal_type").as_deref().unwrap_or("workflow"), 80);
        let mut event = make_runtime_event(&signal_type, &title, "buddy_tool", dedupe_key.as_deref().unwrap_or(""), &status, Some(&priority));
        if dedupe_key.is_none() {
            event.dedupe_key = None;
        }
        event.description = optional_str(args, "description").map(|s| cap(&s, 240));
        buddy_enqueue_event(crate::app_state::AppState::from_gcx(gcx).await, event).await;
        ok_message(tool_call_id, format!("Runtime event enqueued: {}", title))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::actor::BuddyService;
    use crate::buddy::runtime_queue::RuntimeQueue;
    use crate::buddy::settings::BuddySettings;

    async fn ccx(gcx: Arc<GlobalContext>) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(AtCommandsContext::new_from_app(crate::app_state::AppState::from_gcx(gcx).await, 4000, 20, false, vec![], "test-chat".to_string(), None, "test-model".to_string(), None, None).await))
    }

    async fn gcx_with_buddy() -> (tempfile::TempDir, Arc<GlobalContext>) {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let service = BuddyService::new(dir.path().to_path_buf(), crate::buddy::state::default_buddy_state(), BuddySettings::default(), Vec::new(), RuntimeQueue::new(), tokio::sync::broadcast::channel(16).0, None);
        let buddy_arc = gcx.buddy.clone();
        *buddy_arc.lock().await = Some(service);
        (dir, gcx)
    }

    fn args(items: Vec<(&str, Value)>) -> HashMap<String, Value> {
        items.into_iter().map(|(key, value)| (key.to_string(), value)).collect()
    }

    #[tokio::test]
    async fn buddy_log_activity_does_not_panic_with_minimal_args() {
        let (_dir, gcx) = gcx_with_buddy().await;
        let mut tool = ToolBuddyLogActivity { config_path: String::new() };
        let result = tool.tool_execute(ccx(gcx).await, &"call".to_string(), &args(vec![("title", json!("Tiny win")), ("description", json!("A small helpful activity happened"))])).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn buddy_speak_rejects_invalid_intent() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut tool = ToolBuddySpeak { config_path: String::new() };
        let result = tool.tool_execute(ccx(gcx).await, &"call".to_string(), &args(vec![("text", json!("Hello")), ("mood", json!("playful")), ("intent", json!("not_real"))])).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn buddy_runtime_event_dedupes_on_dedupe_key() {
        let (_dir, gcx) = gcx_with_buddy().await;
        let mut tool = ToolBuddyRuntimeEvent { config_path: String::new() };
        let tool_call_id = "call".to_string();
        let call_args = args(vec![("title", json!("Workflow started")), ("status", json!("started")), ("dedupe_key", json!("same-key"))]);
        tool.tool_execute(ccx(gcx.clone()).await, &tool_call_id, &call_args).await.unwrap();
        let buddy_arc = gcx.buddy.clone();
        let first_len = buddy_arc.lock().await.as_ref().unwrap().runtime_queue.items.len();
        tool.tool_execute(ccx(gcx.clone()).await, &tool_call_id, &call_args).await.unwrap();
        let second_len = buddy_arc.lock().await.as_ref().unwrap().runtime_queue.items.len();
        assert_eq!(first_len, 1);
        assert_eq!(second_len, first_len);
    }
}
