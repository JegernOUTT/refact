use std::collections::VecDeque;
use std::time::Duration;

use refact_core::chat_types::{ChatMessage, Checkpoint};
use tokio::sync::Mutex;

use crate::diagnostics::is_ui_only_message;
use crate::{TaskMeta, ThreadParams};

const MAX_QUEUE_SIZE: usize = 100;
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const SESSION_CLEANUP_INTERVAL: Duration = Duration::from_secs(5 * 60);
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const STREAM_TOTAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const STREAM_HEARTBEAT: Duration = Duration::from_secs(2);

pub const SEGMENT_SUMMARY_KIND: &str = "llm_segment_summary";

pub fn max_queue_size() -> usize {
    MAX_QUEUE_SIZE
}

pub fn session_idle_timeout() -> Duration {
    SESSION_IDLE_TIMEOUT
}

pub fn session_cleanup_interval() -> Duration {
    SESSION_CLEANUP_INTERVAL
}

pub fn stream_idle_timeout() -> Duration {
    STREAM_IDLE_TIMEOUT
}

pub fn stream_total_timeout() -> Duration {
    STREAM_TOTAL_TIMEOUT
}

pub fn stream_heartbeat() -> Duration {
    STREAM_HEARTBEAT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueCommandOutcome {
    Accepted,
    Duplicate,
    Full,
}

#[derive(Debug)]
pub struct BurstGuard {
    inner: Mutex<BurstGuardInner>,
}

#[derive(Debug, Default)]
struct BurstGuardInner {
    recent: VecDeque<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurstGuardDecision {
    Allow,
    Defer,
}

impl BurstGuard {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BurstGuardInner::default()),
        }
    }

    pub async fn record_and_check(&self) -> BurstGuardDecision {
        let now = chrono::Utc::now();
        let mut guard = self.inner.lock().await;
        while let Some(front) = guard.recent.front() {
            if now.signed_duration_since(*front).num_seconds() > 10 {
                guard.recent.pop_front();
            } else {
                break;
            }
        }
        if guard.recent.len() >= 5 {
            BurstGuardDecision::Defer
        } else {
            guard.recent.push_back(now);
            BurstGuardDecision::Allow
        }
    }
}

impl Default for BurstGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum TrajectorySourceIdentity {
    #[default]
    Normal,
    Task {
        task_id: String,
        role: String,
        agent_id: Option<String>,
        card_id: Option<String>,
        planner_chat_id: Option<String>,
    },
    Buddy,
}

impl TrajectorySourceIdentity {
    pub fn task(
        task_id: String,
        role: String,
        agent_id: Option<String>,
        card_id: Option<String>,
        planner_chat_id: Option<String>,
    ) -> Self {
        Self::Task {
            task_id,
            role,
            agent_id,
            card_id,
            planner_chat_id,
        }
    }

    pub fn from_task_meta(task_meta: &TaskMeta) -> Self {
        Self::task(
            task_meta.task_id.clone(),
            task_meta.role.clone(),
            task_meta.agent_id.clone(),
            task_meta.card_id.clone(),
            task_meta.planner_chat_id.clone(),
        )
    }

    pub fn from_extra(extra: &serde_json::Map<String, serde_json::Value>) -> Result<Self, String> {
        let buddy_meta_present = extra
            .get("buddy_meta")
            .is_some_and(|value| !value.is_null());
        let task_meta_value = extra.get("task_meta").filter(|value| !value.is_null());

        if buddy_meta_present && task_meta_value.is_some() {
            return Err("trajectory cannot contain both task_meta and buddy_meta".to_string());
        }
        if buddy_meta_present {
            return Ok(Self::Buddy);
        }
        if let Some(value) = task_meta_value {
            let task_meta = serde_json::from_value::<TaskMeta>(value.clone())
                .map_err(|e| format!("invalid task_meta: {}", e))?;
            return Ok(Self::from_task_meta(&task_meta));
        }
        Ok(Self::Normal)
    }

    pub fn from_json(json: &serde_json::Value) -> Result<Self, String> {
        let Some(root) = json.as_object() else {
            return Err("trajectory JSON root must be an object".to_string());
        };
        Self::from_extra(root)
    }

    pub fn from_session_parts(thread: &ThreadParams) -> Self {
        if thread.buddy_meta.is_some() {
            Self::Buddy
        } else if let Some(task_meta) = thread.task_meta.as_ref() {
            Self::from_task_meta(task_meta)
        } else {
            Self::Normal
        }
    }

    pub fn emits_generic_event(&self) -> bool {
        !matches!(self, Self::Buddy)
    }
}

#[derive(Debug, Clone)]
pub struct PendingBrowserMessage {
    pub pending_message_id: String,
    pub content: serde_json::Value,
    pub attachments: Vec<serde_json::Value>,
    pub checkpoints: Vec<Checkpoint>,
    pub context_files: Vec<serde_json::Value>,
    pub suppress_auto_enrichment: bool,
    pub skill_activation_name: Option<String>,
    pub skill_context_msg: Option<ChatMessage>,
}

#[derive(Debug, Clone)]
pub struct PendingSkillDeactivation {
    pub start_index: usize,
    pub report: String,
    pub skill_name: String,
    pub activation_tool_call_id: Option<String>,
}

pub fn is_segment_summary(message: &ChatMessage) -> bool {
    if message.role != "assistant" || is_ui_only_message(message) {
        return false;
    }
    message
        .extra
        .get("compression")
        .and_then(|value| value.get("kind"))
        .and_then(|value| value.as_str())
        == Some(SEGMENT_SUMMARY_KIND)
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::chat_types::ChatContent;
    use serde_json::json;

    #[tokio::test]
    async fn burst_guard_allows_first_five_calls() {
        let guard = BurstGuard::new();
        for _ in 0..5 {
            assert_eq!(guard.record_and_check().await, BurstGuardDecision::Allow);
        }
    }

    #[tokio::test]
    async fn burst_guard_defers_sixth_call() {
        let guard = BurstGuard::new();
        for _ in 0..5 {
            assert_eq!(guard.record_and_check().await, BurstGuardDecision::Allow);
        }

        assert_eq!(guard.record_and_check().await, BurstGuardDecision::Defer);
    }

    #[tokio::test]
    async fn burst_guard_allows_after_window_slides() {
        let guard = BurstGuard::new();
        for _ in 0..5 {
            assert_eq!(guard.record_and_check().await, BurstGuardDecision::Allow);
        }

        tokio::time::sleep(std::time::Duration::from_secs(11)).await;

        assert_eq!(guard.record_and_check().await, BurstGuardDecision::Allow);
    }

    #[test]
    fn is_segment_summary_detects_assistant_compression_kind() {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "compression".to_string(),
            json!({ "kind": SEGMENT_SUMMARY_KIND }),
        );
        let summary = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("summary".to_string()),
            extra,
            ..Default::default()
        };

        assert!(is_segment_summary(&summary));
    }

    #[test]
    fn is_segment_summary_rejects_ui_only_messages() {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "compression".to_string(),
            json!({ "kind": SEGMENT_SUMMARY_KIND }),
        );
        extra.insert("_ui_only".to_string(), json!(true));
        let summary = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("summary".to_string()),
            extra,
            ..Default::default()
        };

        assert!(!is_segment_summary(&summary));
    }
}
