use serde::Deserialize;
use serde_json::{Map, Value};

use crate::text_safety::{sanitize_tool_inline, sanitize_tool_text};

const CLIENT_MESSAGE_ID_EXTRA_KEY: &str = "client_message_id";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackgroundAgentSummary {
    pub agent_id: String,
    pub parent_chat_id: String,
    pub child_chat_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub progress: Option<String>,
    pub step_count: u64,
    pub last_activity: Option<String>,
    pub target_files: Vec<String>,
    pub edited_files: Vec<String>,
    pub diff_summary: Option<String>,
    pub conflict_summary: Option<String>,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub change_seq: u64,
}

impl BackgroundAgentSummary {
    fn from_raw(raw: &Value) -> Self {
        Self {
            agent_id: value_string(raw, &["agent_id", "agentId"]),
            parent_chat_id: value_string(raw, &["parent_chat_id", "parentChatId"]),
            child_chat_id: optional_value_string(raw, &["child_chat_id", "childChatId"]),
            kind: value_string(raw, &["kind"]),
            status: value_string(raw, &["status"]),
            title: value_string(raw, &["title"]),
            progress: optional_value_string(raw, &["progress"]),
            step_count: value_u64(raw, &["step_count", "stepCount"]),
            last_activity: optional_value_string(raw, &["last_activity", "lastActivity"]),
            target_files: string_values(raw, &["target_files", "targetFiles"]),
            edited_files: string_values(raw, &["edited_files", "editedFiles"]),
            diff_summary: optional_value_string(raw, &["diff_summary", "diffSummary"]),
            conflict_summary: optional_value_string(raw, &["conflict_summary", "conflictSummary"]),
            result_summary: optional_value_string(raw, &["result_summary", "resultSummary"]),
            error: optional_value_string(raw, &["error"]),
            started_at: optional_value_string(raw, &["started_at", "startedAt"]),
            finished_at: optional_value_string(raw, &["finished_at", "finishedAt"]),
            change_seq: value_u64(raw, &["change_seq", "changeSeq"]),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserTab {
    pub tab_id: String,
    pub url: String,
    pub title: String,
}

impl BrowserTab {
    fn from_raw(raw: &Value) -> Self {
        Self {
            tab_id: value_string(raw, &["tab_id", "tabId"]),
            url: value_string(raw, &["url"]),
            title: value_string(raw, &["title"]),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserSnapshot {
    pub runtime_id: String,
    pub connected: bool,
    pub active_tab: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub tabs: Vec<BrowserTab>,
}

impl BrowserSnapshot {
    fn from_raw(raw: &Value) -> Self {
        Self {
            runtime_id: value_string(raw, &["runtime_id", "runtimeId"]),
            connected: value_bool(raw, &["connected"]),
            active_tab: optional_value_string(raw, &["active_tab", "activeTab"]),
            url: optional_value_string(raw, &["url"]),
            title: optional_value_string(raw, &["title"]),
            tabs: raw
                .get("tabs")
                .and_then(Value::as_array)
                .map(|tabs| tabs.iter().map(BrowserTab::from_raw).collect())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProcessCompletedEvent {
    pub process_id: String,
    pub status: String,
    pub exit_code: Option<i64>,
    pub short_description: String,
    pub mode: String,
}

impl ProcessCompletedEvent {
    fn from_raw(raw: &Value) -> Self {
        Self {
            process_id: value_string(raw, &["process_id", "processId"]),
            status: value_string(raw, &["status"]),
            exit_code: raw
                .get("exit_code")
                .or_else(|| raw.get("exitCode"))
                .and_then(Value::as_i64),
            short_description: value_string(raw, &["short_description", "shortDescription"]),
            mode: value_string(raw, &["mode"]),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IdeToolRequiredEvent {
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Value,
}

impl IdeToolRequiredEvent {
    fn from_raw(raw: &Value) -> Self {
        Self {
            tool_call_id: value_string(raw, &["tool_call_id", "toolCallId"]),
            tool_name: value_string(raw, &["tool_name", "toolName"]),
            args: raw.get("args").cloned().unwrap_or(Value::Null),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserFrameEvent {
    pub tab_id: String,
    pub mime: String,
    pub data: String,
    pub diff_boxes: Vec<Value>,
    pub changed_text: Option<String>,
}

impl BrowserFrameEvent {
    fn from_raw(raw: &Value) -> Self {
        Self {
            tab_id: value_string(raw, &["tab_id", "tabId"]),
            mime: value_string(raw, &["mime"]),
            data: value_string(raw, &["data"]),
            diff_boxes: raw
                .get("diff_boxes")
                .or_else(|| raw.get("diffBoxes"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            changed_text: optional_value_string(raw, &["changed_text", "changedText"]),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserStatusEvent {
    pub snapshot: BrowserSnapshot,
}

impl BrowserStatusEvent {
    fn from_raw(raw: &Value) -> Self {
        Self {
            snapshot: BrowserSnapshot::from_raw(raw),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserClosedEvent {
    pub runtime_id: String,
    pub reason: String,
}

impl BrowserClosedEvent {
    fn from_raw(raw: &Value) -> Self {
        Self {
            runtime_id: value_string(raw, &["runtime_id", "runtimeId"]),
            reason: value_string(raw, &["reason"]),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserTimelineEvent {
    pub events: Vec<Value>,
}

impl BrowserTimelineEvent {
    fn from_raw(raw: &Value) -> Self {
        Self {
            events: raw
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserContextOversizeEvent {
    pub total_bytes: usize,
    pub action_count: usize,
    pub action_bytes: usize,
    pub console_count: usize,
    pub console_bytes: usize,
    pub network_count: usize,
    pub network_bytes: usize,
    pub mutation_bytes: usize,
    pub pending_message_id: String,
}

impl BrowserContextOversizeEvent {
    fn from_raw(raw: &Value) -> Self {
        Self {
            total_bytes: value_usize(raw, &["total_bytes", "totalBytes"]),
            action_count: value_usize(raw, &["action_count", "actionCount"]),
            action_bytes: value_usize(raw, &["action_bytes", "actionBytes"]),
            console_count: value_usize(raw, &["console_count", "consoleCount"]),
            console_bytes: value_usize(raw, &["console_bytes", "consoleBytes"]),
            network_count: value_usize(raw, &["network_count", "networkCount"]),
            network_bytes: value_usize(raw, &["network_bytes", "networkBytes"]),
            mutation_bytes: value_usize(raw, &["mutation_bytes", "mutationBytes"]),
            pending_message_id: value_string(raw, &["pending_message_id", "pendingMessageId"]),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserToolbarActionEvent {
    pub action: String,
}

impl BrowserToolbarActionEvent {
    fn from_raw(raw: &Value) -> Self {
        Self {
            action: value_string(raw, &["action"]),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnknownSseEvent {
    pub kind: String,
    pub raw: Value,
    pub malformed_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InboundEventState {
    background_agents: Vec<BackgroundAgentSummary>,
    browser: Option<BrowserSnapshot>,
    last_background_agent_updated: Option<BackgroundAgentSummary>,
    last_process_completed: Option<ProcessCompletedEvent>,
    ide_tool_required: Option<IdeToolRequiredEvent>,
    last_browser_frame: Option<BrowserFrameEvent>,
    last_browser_status: Option<BrowserStatusEvent>,
    last_browser_closed: Option<BrowserClosedEvent>,
    last_browser_timeline: Option<BrowserTimelineEvent>,
    browser_context_oversize: Option<BrowserContextOversizeEvent>,
    last_browser_toolbar_action: Option<BrowserToolbarActionEvent>,
    unknown_events: Vec<UnknownSseEvent>,
}

impl InboundEventState {
    pub fn background_agents(&self) -> &[BackgroundAgentSummary] {
        &self.background_agents
    }

    pub fn browser(&self) -> Option<&BrowserSnapshot> {
        self.browser.as_ref()
    }

    pub fn last_background_agent_updated(&self) -> Option<&BackgroundAgentSummary> {
        self.last_background_agent_updated.as_ref()
    }

    pub fn last_process_completed(&self) -> Option<&ProcessCompletedEvent> {
        self.last_process_completed.as_ref()
    }

    pub fn ide_tool_required(&self) -> Option<&IdeToolRequiredEvent> {
        self.ide_tool_required.as_ref()
    }

    pub fn last_browser_frame(&self) -> Option<&BrowserFrameEvent> {
        self.last_browser_frame.as_ref()
    }

    pub fn last_browser_status(&self) -> Option<&BrowserStatusEvent> {
        self.last_browser_status.as_ref()
    }

    pub fn last_browser_closed(&self) -> Option<&BrowserClosedEvent> {
        self.last_browser_closed.as_ref()
    }

    pub fn last_browser_timeline(&self) -> Option<&BrowserTimelineEvent> {
        self.last_browser_timeline.as_ref()
    }

    pub fn browser_context_oversize(&self) -> Option<&BrowserContextOversizeEvent> {
        self.browser_context_oversize.as_ref()
    }

    pub fn last_browser_toolbar_action(&self) -> Option<&BrowserToolbarActionEvent> {
        self.last_browser_toolbar_action.as_ref()
    }

    pub fn unknown_events(&self) -> &[UnknownSseEvent] {
        &self.unknown_events
    }

    pub fn apply_snapshot(
        &mut self,
        background_agents: Vec<BackgroundAgentSummary>,
        browser: Option<BrowserSnapshot>,
    ) {
        self.background_agents = background_agents;
        self.browser = browser;
    }

    pub fn update_background_agent(&mut self, agent: BackgroundAgentSummary) {
        if let Some(existing) = self
            .background_agents
            .iter_mut()
            .find(|existing| !agent.agent_id.is_empty() && existing.agent_id == agent.agent_id)
        {
            *existing = agent.clone();
        } else {
            self.background_agents.push(agent.clone());
        }
        self.last_background_agent_updated = Some(agent);
    }

    pub fn set_process_completed(&mut self, event: ProcessCompletedEvent) {
        self.last_process_completed = Some(event);
    }

    pub fn set_ide_tool_required(&mut self, event: IdeToolRequiredEvent) {
        self.ide_tool_required = Some(event);
    }

    pub fn set_browser_frame(&mut self, event: BrowserFrameEvent) {
        self.last_browser_frame = Some(event);
    }

    pub fn set_browser_status(&mut self, event: BrowserStatusEvent) {
        self.browser = Some(event.snapshot.clone());
        self.last_browser_status = Some(event);
    }

    pub fn set_browser_closed(&mut self, event: BrowserClosedEvent) {
        if self
            .browser
            .as_ref()
            .is_some_and(|browser| browser.runtime_id == event.runtime_id)
        {
            self.browser = None;
        }
        self.last_browser_closed = Some(event);
    }

    pub fn set_browser_timeline(&mut self, event: BrowserTimelineEvent) {
        self.last_browser_timeline = Some(event);
    }

    pub fn set_browser_context_oversize(&mut self, event: BrowserContextOversizeEvent) {
        self.browser_context_oversize = Some(event);
    }

    pub fn set_browser_toolbar_action(&mut self, event: BrowserToolbarActionEvent) {
        self.last_browser_toolbar_action = Some(event);
    }

    pub fn record_unknown(&mut self, event: UnknownSseEvent) {
        self.unknown_events.push(event);
    }
}

fn first_value<'a>(raw: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| raw.get(*key))
}

fn value_string(raw: &Value, keys: &[&str]) -> String {
    first_value(raw, keys)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn optional_value_string(raw: &Value, keys: &[&str]) -> Option<String> {
    first_value(raw, keys)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn value_u64(raw: &Value, keys: &[&str]) -> u64 {
    first_value(raw, keys)
        .and_then(Value::as_u64)
        .unwrap_or_default()
}

fn value_usize(raw: &Value, keys: &[&str]) -> usize {
    value_u64(raw, keys) as usize
}

fn value_bool(raw: &Value, keys: &[&str]) -> bool {
    first_value(raw, keys)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn string_values(raw: &Value, keys: &[&str]) -> Vec<String> {
    first_value(raw, keys)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeGoalBudgetSnapshot {
    pub max_turns: Option<u64>,
    pub max_minutes: Option<u64>,
    pub max_tokens: Option<u64>,
    pub max_cost_cents: Option<u64>,
    pub no_progress_turns: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeGoalSnapshot {
    pub active: Option<bool>,
    pub status: Option<String>,
    pub turn_count: Option<u64>,
    pub token_count: Option<u64>,
    pub no_progress_turn_count: Option<u64>,
    pub budget: RuntimeGoalBudgetSnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeCompressionSnapshot {
    pub active: bool,
    pub phase: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeUpdatedEvent {
    pub runtime_state: Option<String>,
    pub goal: RuntimeGoalSnapshot,
    pub compression: RuntimeCompressionSnapshot,
    pub usage: Option<Value>,
}

impl RuntimeUpdatedEvent {
    fn from_raw(raw: &Value) -> Self {
        let goal = raw.get("goal").unwrap_or(raw);
        let budget = goal
            .get("budget")
            .or_else(|| raw.get("goal_budget"))
            .or_else(|| raw.get("budget"));
        let compression = raw.get("compression").unwrap_or(raw);
        let phase = optional_value_string(compression, &["phase", "compression_phase"]);
        let active = first_value(compression, &["active", "compression_active"])
            .or_else(|| raw.get("is_compressing"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && !matches!(phase.as_deref(), Some("applied" | "skipped" | "failed"));
        Self {
            runtime_state: optional_value_string(raw, &["state", "runtime_state"]),
            goal: RuntimeGoalSnapshot {
                active: first_value(goal, &["active", "goal_active"])
                    .or_else(|| raw.get("goal_active"))
                    .and_then(Value::as_bool),
                status: optional_value_string(goal, &["status", "goal_status"])
                    .or_else(|| optional_value_string(raw, &["goal_status"])),
                turn_count: first_value(
                    goal,
                    &["turn_count", "turns", "goal_turn_count", "goal_turns_used"],
                )
                .or_else(|| raw.get("goal_turns_used"))
                .and_then(Value::as_u64),
                token_count: first_value(
                    goal,
                    &[
                        "token_count",
                        "tokens",
                        "goal_token_count",
                        "goal_tokens_used",
                    ],
                )
                .or_else(|| raw.get("goal_tokens_used"))
                .and_then(Value::as_u64),
                no_progress_turn_count: first_value(
                    goal,
                    &[
                        "no_progress_turn_count",
                        "no_progress_turns",
                        "goal_no_progress_turn_count",
                        "goal_no_progress_turns",
                    ],
                )
                .or_else(|| raw.get("goal_no_progress_turns"))
                .and_then(Value::as_u64),
                budget: RuntimeGoalBudgetSnapshot {
                    max_turns: optional_budget_limit(budget, &["max_turns"]),
                    max_minutes: optional_budget_limit(budget, &["max_minutes"]),
                    max_tokens: optional_budget_limit(budget, &["max_tokens"]),
                    max_cost_cents: optional_budget_limit(budget, &["max_cost_cents"]),
                    no_progress_turns: optional_budget_limit(budget, &["no_progress_turns"]),
                },
            },
            compression: RuntimeCompressionSnapshot {
                active,
                phase,
                reason: optional_value_string(compression, &["reason", "compression_reason"]),
            },
            usage: raw
                .get("usage")
                .or_else(|| raw.get("last_usage"))
                .or_else(|| raw.get("token_usage"))
                .cloned(),
        }
    }
}

fn optional_budget_limit(raw: Option<&Value>, keys: &[&str]) -> Option<u64> {
    raw.and_then(|raw| first_value(raw, keys))
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
}

#[derive(Debug, Clone, PartialEq)]
pub enum SseEvent {
    Snapshot {
        thread: Option<Value>,
        runtime: Option<Value>,
        messages: Vec<Value>,
        background_agents: Vec<BackgroundAgentSummary>,
        browser: Option<BrowserSnapshot>,
    },
    BackgroundAgentUpdated {
        agent: BackgroundAgentSummary,
    },
    StreamStarted {
        message_id: Option<String>,
    },
    StreamDelta {
        message_id: Option<String>,
        ops: Vec<DeltaOp>,
    },
    MalformedStreamDelta {
        message_id: Option<String>,
        reason: String,
    },
    StreamFinished {
        message_id: Option<String>,
        usage: Option<Value>,
        finish_reason: Option<Value>,
    },
    RuntimeUpdated {
        runtime: RuntimeUpdatedEvent,
    },
    Ack {
        client_request_id: String,
        accepted: bool,
        result: Option<Value>,
    },
    ProcessCompleted {
        event: ProcessCompletedEvent,
    },
    IdeToolRequired {
        event: IdeToolRequiredEvent,
    },
    QueueUpdated {
        queue_size: usize,
        queued_items: Vec<Value>,
    },
    PauseRequired,
    PauseCleared,
    ThreadUpdated {
        params: Value,
    },
    MessageAdded {
        message: Option<Value>,
        index: Option<usize>,
    },
    MessageUpdated {
        message_id: Option<String>,
        message: Option<Value>,
    },
    MessageRemoved {
        message_id: Option<String>,
    },
    MessagesTruncated {
        from_index: usize,
    },
    SubchatUpdate {
        tool_call_id: String,
        subchat_id: String,
        attached_files: Vec<String>,
        depth: usize,
    },
    BrowserFrame {
        event: BrowserFrameEvent,
    },
    BrowserStatus {
        event: BrowserStatusEvent,
    },
    BrowserClosed {
        event: BrowserClosedEvent,
    },
    BrowserTimeline {
        event: BrowserTimelineEvent,
    },
    BrowserContextOversize {
        event: BrowserContextOversizeEvent,
    },
    BrowserToolbarAction {
        event: BrowserToolbarActionEvent,
    },
    Unknown {
        event: UnknownSseEvent,
    },
}

impl<'de> Deserialize<'de> for SseEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = Value::deserialize(deserializer)?;
        Ok(Self::from_raw(&raw))
    }
}

impl SseEvent {
    pub fn from_raw(raw: &Value) -> Self {
        let kind = raw.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "snapshot" => match raw.get("messages").and_then(Value::as_array) {
                Some(messages) => Self::Snapshot {
                    thread: raw.get("thread").cloned(),
                    runtime: raw.get("runtime").cloned(),
                    messages: messages.clone(),
                    background_agents: raw
                        .get("background_agents")
                        .or_else(|| raw.get("backgroundAgents"))
                        .and_then(Value::as_array)
                        .map(|agents| {
                            agents
                                .iter()
                                .map(BackgroundAgentSummary::from_raw)
                                .collect()
                        })
                        .unwrap_or_default(),
                    browser: raw
                        .get("browser")
                        .filter(|browser| !browser.is_null())
                        .map(BrowserSnapshot::from_raw),
                },
                None => malformed_authoritative_event(kind, raw, "missing or non-array messages"),
            },
            "background_agent_updated" => Self::BackgroundAgentUpdated {
                agent: raw
                    .get("agent")
                    .map(BackgroundAgentSummary::from_raw)
                    .unwrap_or_default(),
            },
            "stream_started" => Self::StreamStarted {
                message_id: message_id(raw),
            },
            "stream_delta" => match raw.get("ops").and_then(Value::as_array) {
                Some(ops) => Self::StreamDelta {
                    message_id: message_id(raw),
                    ops: delta_ops_from_value(&Value::Array(ops.clone())),
                },
                None => Self::MalformedStreamDelta {
                    message_id: message_id(raw),
                    reason: "missing or non-array ops".to_string(),
                },
            },
            "stream_finished" => Self::StreamFinished {
                message_id: message_id(raw),
                usage: raw.get("usage").cloned(),
                finish_reason: raw.get("finish_reason").cloned(),
            },
            "runtime_updated" => Self::RuntimeUpdated {
                runtime: RuntimeUpdatedEvent::from_raw(raw),
            },
            "ack" => match (
                raw.get("client_request_id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty()),
                raw.get("accepted").and_then(Value::as_bool),
            ) {
                (Some(client_request_id), Some(accepted)) => Self::Ack {
                    client_request_id: client_request_id.to_string(),
                    accepted,
                    result: raw.get("result").cloned(),
                },
                (None, _) => {
                    malformed_authoritative_event(kind, raw, "missing or empty client_request_id")
                }
                (_, None) => {
                    malformed_authoritative_event(kind, raw, "missing or non-boolean accepted")
                }
            },
            "process_completed" => Self::ProcessCompleted {
                event: ProcessCompletedEvent::from_raw(raw),
            },
            "ide_tool_required" => Self::IdeToolRequired {
                event: IdeToolRequiredEvent::from_raw(raw),
            },
            "queue_updated" => Self::QueueUpdated {
                queue_size: raw
                    .get("queue_size")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize,
                queued_items: raw
                    .get("queued_items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            },
            "pause_required" => Self::PauseRequired,
            "pause_cleared" => Self::PauseCleared,
            "thread_updated" => Self::ThreadUpdated {
                params: raw.clone(),
            },
            "message_added" => Self::MessageAdded {
                message: raw.get("message").or_else(|| raw.get("msg")).cloned(),
                index: raw
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize),
            },
            "message_updated" => Self::MessageUpdated {
                message_id: message_id(raw),
                message: raw.get("message").or_else(|| raw.get("msg")).cloned(),
            },
            "message_removed" => Self::MessageRemoved {
                message_id: message_id(raw),
            },
            "messages_truncated" => match raw.get("from_index").and_then(Value::as_u64) {
                Some(from_index) => Self::MessagesTruncated {
                    from_index: from_index as usize,
                },
                None => malformed_authoritative_event(
                    kind,
                    raw,
                    "missing or invalid non-negative integer from_index",
                ),
            },
            "subchat_update" => Self::SubchatUpdate {
                tool_call_id: raw
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                subchat_id: raw
                    .get("subchat_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                attached_files: raw
                    .get("attached_files")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                depth: raw
                    .get("depth")
                    .or_else(|| raw.get("subchat_depth"))
                    .and_then(Value::as_u64)
                    .unwrap_or(1) as usize,
            },
            "browser_frame" => Self::BrowserFrame {
                event: BrowserFrameEvent::from_raw(raw),
            },
            "browser_status" => Self::BrowserStatus {
                event: BrowserStatusEvent::from_raw(raw),
            },
            "browser_closed" => Self::BrowserClosed {
                event: BrowserClosedEvent::from_raw(raw),
            },
            "browser_timeline" => Self::BrowserTimeline {
                event: BrowserTimelineEvent::from_raw(raw),
            },
            "browser_context_oversize" => Self::BrowserContextOversize {
                event: BrowserContextOversizeEvent::from_raw(raw),
            },
            "browser_toolbar_action" => Self::BrowserToolbarAction {
                event: BrowserToolbarActionEvent::from_raw(raw),
            },
            _ => Self::Unknown {
                event: UnknownSseEvent {
                    kind: kind.to_string(),
                    raw: raw.clone(),
                    malformed_reason: None,
                },
            },
        }
    }
}

fn malformed_authoritative_event(kind: &str, raw: &Value, reason: &str) -> SseEvent {
    SseEvent::Unknown {
        event: UnknownSseEvent {
            kind: kind.to_string(),
            raw: raw.clone(),
            malformed_reason: Some(reason.to_string()),
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeltaOp {
    AppendContent { text: String },
    AppendReasoning { text: String },
    SetReasoning { text: String },
    SetToolCalls { tool_calls: Vec<Value> },
    SetThinkingBlocks { blocks: Vec<Value> },
    AddCitation { citation: Value },
    AddServerContentBlock { block: Value },
    SetUsage { usage: Value },
    MergeExtra { extra: Map<String, Value> },
    Unknown(UnknownDeltaOp),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnknownDeltaOp {
    pub op: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
enum DeltaOpWire {
    Known(KnownDeltaOp),
    Unknown(RawDeltaOp),
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
enum KnownDeltaOp {
    AppendContent { text: String },
    AppendReasoning { text: String },
    SetReasoning { text: String },
    SetToolCalls { tool_calls: Vec<Value> },
    SetThinkingBlocks { blocks: Vec<Value> },
    AddCitation { citation: Value },
    AddServerContentBlock { block: Value },
    SetUsage { usage: Value },
    MergeExtra { extra: Map<String, Value> },
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct RawDeltaOp {
    #[serde(flatten)]
    fields: Map<String, Value>,
}

impl<'de> Deserialize<'de> for DeltaOp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DeltaOpWire::deserialize(deserializer)?;
        Ok(match wire {
            DeltaOpWire::Known(op) => op.into(),
            DeltaOpWire::Unknown(raw) => Self::Unknown(UnknownDeltaOp::from_fields(raw.fields)),
        })
    }
}

impl From<KnownDeltaOp> for DeltaOp {
    fn from(value: KnownDeltaOp) -> Self {
        match value {
            KnownDeltaOp::AppendContent { text } => Self::AppendContent {
                text: sanitize_tool_text(text),
            },
            KnownDeltaOp::AppendReasoning { text } => Self::AppendReasoning {
                text: sanitize_tool_text(text),
            },
            KnownDeltaOp::SetReasoning { text } => Self::SetReasoning {
                text: sanitize_tool_text(text),
            },
            KnownDeltaOp::SetToolCalls { tool_calls } => Self::SetToolCalls { tool_calls },
            KnownDeltaOp::SetThinkingBlocks { blocks } => Self::SetThinkingBlocks { blocks },
            KnownDeltaOp::AddCitation { citation } => Self::AddCitation { citation },
            KnownDeltaOp::AddServerContentBlock { block } => Self::AddServerContentBlock { block },
            KnownDeltaOp::SetUsage { usage } => Self::SetUsage { usage },
            KnownDeltaOp::MergeExtra { extra } => Self::MergeExtra { extra },
        }
    }
}

impl UnknownDeltaOp {
    fn from_fields(fields: Map<String, Value>) -> Self {
        let op = fields.get("op").and_then(Value::as_str).map(str::to_string);
        Self {
            op,
            raw: Value::Object(fields),
        }
    }

    fn from_raw(raw: Value) -> Self {
        let op = raw.get("op").and_then(Value::as_str).map(str::to_string);
        Self { op, raw }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptRole {
    User,
    Assistant,
    Tool,
    ClientLocalNotice,
    Plan,
    Goal,
    Event,
    System,
    ContextFile,
    Diff,
    PlainText,
    CdInstruction,
    CompressionReport,
    Error,
    Unknown { role: String, raw: Value },
}

impl TranscriptRole {
    pub fn from_wire(role: &str) -> Self {
        Self::from_wire_message(role, Value::Null)
    }

    pub fn from_wire_message(role: &str, raw: Value) -> Self {
        match role {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "tool" => Self::Tool,
            "notice" => Self::ClientLocalNotice,
            "plan" => Self::Plan,
            "goal" => Self::Goal,
            "event" => Self::Event,
            "system" => Self::System,
            "context_file" => Self::ContextFile,
            "diff" => Self::Diff,
            "plain_text" => Self::PlainText,
            "cd_instruction" => Self::CdInstruction,
            "compression_report" => Self::CompressionReport,
            "error" => Self::Error,
            other => Self::Unknown {
                role: other.to_string(),
                raw,
            },
        }
    }

    pub fn is_tool_result(&self) -> bool {
        matches!(self, Self::Tool | Self::Diff)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::ClientLocalNotice => "notice",
            Self::Plan => "plan",
            Self::Goal => "goal",
            Self::Event => "event",
            Self::System => "system",
            Self::ContextFile => "context_file",
            Self::Diff => "diff",
            Self::PlainText => "plain_text",
            Self::CdInstruction => "cd_instruction",
            Self::CompressionReport => "compression_report",
            Self::Error => "error",
            Self::Unknown { role, .. } => role.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptMessage {
    pub message_id: Option<String>,
    pub role: TranscriptRole,
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<Value>,
    pub tool_call_id: Option<String>,
    pub tool_failed: bool,
    pub usage: Option<Value>,
    pub citations: Vec<Value>,
    pub thinking_blocks: Vec<Value>,
    pub server_content_blocks: Vec<Value>,
    pub extra: Map<String, Value>,
    pub unknown_delta_ops: Vec<UnknownDeltaOp>,
    pub stream_finished: bool,
}

impl TranscriptMessage {
    pub fn new(role: TranscriptRole) -> Self {
        Self {
            message_id: None,
            role,
            content: String::new(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_failed: false,
            usage: None,
            citations: Vec::new(),
            thinking_blocks: Vec::new(),
            server_content_blocks: Vec::new(),
            extra: Map::new(),
            unknown_delta_ops: Vec::new(),
            stream_finished: true,
        }
    }

    pub fn assistant(message_id: Option<String>) -> Self {
        let mut message = Self::new(TranscriptRole::Assistant);
        message.message_id = message_id;
        message.stream_finished = false;
        message
    }

    pub fn client_message_id(&self) -> Option<&str> {
        self.extra
            .get(CLIENT_MESSAGE_ID_EXTRA_KEY)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    }

    pub fn from_wire(raw: &Value) -> Self {
        let role = raw
            .get("role")
            .and_then(Value::as_str)
            .map(|role| TranscriptRole::from_wire_message(role, raw.clone()))
            .unwrap_or_else(|| TranscriptRole::Unknown {
                role: "missing".to_string(),
                raw: raw.clone(),
            });
        let mut message = Self::new(role);
        message.message_id = raw
            .get("message_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        message.content = content_text(raw).unwrap_or_default();
        message.reasoning = sanitize_model_text_for_role(
            &message.role,
            raw.get("reasoning_content")
                .or_else(|| raw.get("reasoning"))
                .and_then(value_to_text)
                .unwrap_or_default(),
        );
        message.tool_calls = raw
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        message.tool_call_id = raw
            .get("tool_call_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        message.tool_failed = raw
            .get("tool_failed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        message.usage = raw.get("usage").cloned();
        message.citations = raw
            .get("citations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        message.thinking_blocks = raw
            .get("thinking_blocks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        message.server_content_blocks = raw
            .get("server_content_blocks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        message.stream_finished = match raw.get("stream_finished").and_then(Value::as_bool) {
            Some(stream_finished) => stream_finished,
            None => !matches!(
                message.role,
                TranscriptRole::Assistant | TranscriptRole::Tool | TranscriptRole::Diff
            ),
        };
        message.extra = extra_fields(raw);
        message
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscriptState {
    messages: Vec<TranscriptMessage>,
    active_assistant_id: Option<String>,
    active_assistant_index: Option<usize>,
    usage: Option<Value>,
    unknown_delta_ops: Vec<UnknownDeltaOp>,
}

impl TranscriptState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(&self) -> &[TranscriptMessage] {
        &self.messages
    }

    pub fn messages_mut(&mut self) -> &mut [TranscriptMessage] {
        &mut self.messages
    }

    pub fn usage(&self) -> Option<&Value> {
        self.usage.as_ref()
    }

    pub fn set_usage(&mut self, usage: Value) {
        self.usage = Some(usage);
    }

    pub fn unknown_delta_ops(&self) -> &[UnknownDeltaOp] {
        &self.unknown_delta_ops
    }

    pub fn reset(&mut self) {
        self.messages.clear();
        self.active_assistant_id = None;
        self.active_assistant_index = None;
        self.usage = None;
        self.unknown_delta_ops.clear();
    }

    pub fn reset_from_messages(&mut self, messages: &[Value]) {
        self.reset();
        self.messages = messages.iter().map(TranscriptMessage::from_wire).collect();
        self.refresh_cached_indexes();
    }

    pub fn truncate_messages(&mut self, from_index: usize) {
        self.messages.truncate(from_index.min(self.messages.len()));
        self.refresh_cached_indexes();
    }

    fn refresh_cached_indexes(&mut self) {
        self.usage = self
            .messages
            .iter()
            .rev()
            .find_map(|message| message.usage.clone());
        self.active_assistant_index = self
            .messages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, message)| {
                message.role == TranscriptRole::Assistant && !message.stream_finished
            })
            .map(|(idx, _)| idx);
        self.active_assistant_id = self
            .active_assistant_index
            .and_then(|idx| self.messages[idx].message_id.clone());
    }

    pub fn push_notice(&mut self, text: impl Into<String>) {
        let mut message = TranscriptMessage::new(TranscriptRole::ClientLocalNotice);
        message.content = text.into();
        self.messages.push(message);
    }

    pub fn push_user_message(&mut self, content: impl Into<String>) {
        let mut message = TranscriptMessage::new(TranscriptRole::User);
        message.content = content.into();
        self.messages.push(message);
        self.active_assistant_id = None;
        self.active_assistant_index = None;
    }

    pub fn push_optimistic_user_message(
        &mut self,
        content: impl Into<String>,
        client_message_id: impl Into<String>,
    ) {
        let mut message = TranscriptMessage::new(TranscriptRole::User);
        message.content = content.into();
        message.extra.insert(
            CLIENT_MESSAGE_ID_EXTRA_KEY.to_string(),
            Value::String(client_message_id.into()),
        );
        self.messages.push(message);
        self.active_assistant_id = None;
        self.active_assistant_index = None;
    }

    pub fn replace_optimistic_user_message_at(
        &mut self,
        message: TranscriptMessage,
        index: Option<usize>,
    ) -> bool {
        let Some(client_message_id) = message.client_message_id() else {
            return false;
        };
        let Some(existing_index) = self.messages.iter().position(|existing| {
            existing.role == TranscriptRole::User
                && existing.message_id.is_none()
                && existing.client_message_id() == Some(client_message_id)
        }) else {
            return false;
        };
        if let Some(server_index) = self.message_index_by_id(message.message_id.as_deref()) {
            self.messages.remove(existing_index);
            let server_index = if existing_index < server_index {
                server_index - 1
            } else {
                server_index
            };
            self.replace_message_at(server_index, message, index);
            return true;
        }
        self.replace_message_at(existing_index, message, index);
        true
    }

    pub fn has_optimistic_user_message(&self) -> bool {
        self.messages.iter().any(|message| {
            message.role == TranscriptRole::User
                && message.message_id.is_none()
                && message.client_message_id().is_some()
        })
    }

    pub fn add_message(&mut self, raw: &Value) -> bool {
        self.add_message_at(raw, None)
    }

    pub fn add_message_at(&mut self, raw: &Value, index: Option<usize>) -> bool {
        let message = TranscriptMessage::from_wire(raw);
        self.add_transcript_message(message, index)
    }

    pub fn update_message(&mut self, message_id: Option<&str>, raw: &Value) -> bool {
        let mut message = TranscriptMessage::from_wire(raw);
        if message.message_id.is_none() {
            message.message_id = message_id
                .filter(|value| !value.is_empty())
                .map(str::to_string);
        }
        let lookup_id = message.message_id.as_deref().or(message_id);
        if let Some(idx) = self.message_index_by_id(lookup_id) {
            if let Some(usage) = message.usage.clone() {
                self.usage = Some(usage);
            }
            self.messages[idx] = message;
            self.refresh_cached_indexes();
            true
        } else {
            self.add_transcript_message(message, None)
        }
    }

    pub fn remove_message(&mut self, message_id: Option<&str>) -> bool {
        let Some(idx) = self.message_index_by_id(message_id) else {
            return false;
        };
        self.messages.remove(idx);
        self.refresh_cached_indexes();
        true
    }

    fn add_transcript_message(&mut self, message: TranscriptMessage, index: Option<usize>) -> bool {
        if let Some(idx) = self.message_index_by_id(message.message_id.as_deref()) {
            self.replace_message_at(idx, message, index);
            return false;
        }
        if let Some(usage) = message.usage.clone() {
            self.usage = Some(usage);
        }
        let is_assistant = message.role == TranscriptRole::Assistant;
        let index = index
            .unwrap_or(self.messages.len())
            .min(self.messages.len());
        if self
            .active_assistant_index
            .is_some_and(|active| active >= index)
        {
            self.active_assistant_index = self.active_assistant_index.map(|active| active + 1);
        }
        self.messages.insert(index, message);
        if is_assistant
            && !self
                .messages
                .get(index)
                .is_some_and(|message| message.stream_finished)
        {
            self.active_assistant_id = self.messages[index].message_id.clone();
            self.active_assistant_index = Some(index);
        }
        true
    }

    fn replace_message_at(
        &mut self,
        existing_index: usize,
        message: TranscriptMessage,
        index: Option<usize>,
    ) {
        let active_assistant_id = self.active_assistant_id.clone();
        self.messages.remove(existing_index);
        let index = index.unwrap_or(existing_index).min(self.messages.len());
        self.messages.insert(index, message);
        self.refresh_cached_indexes();
        if let Some(active_assistant_id) = active_assistant_id {
            if let Some(active_assistant_index) = self.messages.iter().position(|message| {
                message.role == TranscriptRole::Assistant
                    && message.message_id.as_deref() == Some(&active_assistant_id)
                    && !message.stream_finished
            }) {
                self.active_assistant_id = Some(active_assistant_id);
                self.active_assistant_index = Some(active_assistant_index);
            }
        }
    }

    fn message_index_by_id(&self, message_id: Option<&str>) -> Option<usize> {
        let message_id = message_id.filter(|value| !value.is_empty())?;
        self.messages
            .iter()
            .position(|message| message.message_id.as_deref() == Some(message_id))
    }

    pub fn start_assistant(&mut self, message_id: Option<&str>) {
        self.ensure_assistant_index(message_id);
    }

    pub fn finish_assistant(&mut self, message_id: Option<&str>, usage: Option<Value>) {
        let should_finish = message_id.filter(|value| !value.is_empty()).is_some()
            || self.active_assistant_index.is_some();
        if !should_finish {
            return;
        }
        let idx = self.ensure_assistant_index(message_id);
        self.messages[idx].stream_finished = true;
        if let Some(usage) = usage {
            self.messages[idx].usage = Some(usage.clone());
            self.usage = Some(usage);
        }
        if self.active_assistant_index == Some(idx) {
            self.active_assistant_index = None;
            self.active_assistant_id = None;
        }
    }

    pub fn apply_delta_ops(&mut self, message_id: Option<&str>, ops: &[DeltaOp]) {
        for op in ops {
            let idx = self.ensure_assistant_index(message_id);
            match op {
                DeltaOp::AppendContent { text } => self.messages[idx].content.push_str(text),
                DeltaOp::AppendReasoning { text } => self.messages[idx].reasoning.push_str(text),
                DeltaOp::SetReasoning { text } => self.messages[idx].reasoning.clone_from(text),
                DeltaOp::SetToolCalls { tool_calls } => {
                    self.messages[idx].tool_calls = tool_calls.clone();
                }
                DeltaOp::SetThinkingBlocks { blocks } => {
                    self.messages[idx].thinking_blocks = blocks.clone();
                }
                DeltaOp::AddCitation { citation } => {
                    self.messages[idx].citations.push(citation.clone());
                }
                DeltaOp::AddServerContentBlock { block } => {
                    self.messages[idx].server_content_blocks.push(block.clone());
                }
                DeltaOp::SetUsage { usage } => {
                    self.messages[idx].usage = Some(usage.clone());
                    self.usage = Some(usage.clone());
                }
                DeltaOp::MergeExtra { extra } => {
                    self.messages[idx].extra.extend(extra.clone());
                }
                DeltaOp::Unknown(unknown) => {
                    self.messages[idx].unknown_delta_ops.push(unknown.clone());
                    self.unknown_delta_ops.push(unknown.clone());
                }
            }
        }
    }

    pub fn citations(&self) -> impl Iterator<Item = &Value> {
        self.messages
            .iter()
            .flat_map(|message| message.citations.iter())
    }

    pub fn server_content_blocks(&self) -> impl Iterator<Item = &Value> {
        self.messages
            .iter()
            .flat_map(|message| message.server_content_blocks.iter())
    }

    fn ensure_assistant_index(&mut self, message_id: Option<&str>) -> usize {
        let normalized_id = message_id.filter(|value| !value.is_empty());
        if let Some(id) = normalized_id {
            if let Some(idx) = self.messages.iter().position(|message| {
                message.role == TranscriptRole::Assistant
                    && message.message_id.as_deref() == Some(id)
            }) {
                self.active_assistant_id = Some(id.to_string());
                self.active_assistant_index = Some(idx);
                self.messages[idx].stream_finished = false;
                return idx;
            }
        }
        if let Some(idx) = self.active_assistant_index {
            if self
                .messages
                .get(idx)
                .is_some_and(|message| message.role == TranscriptRole::Assistant)
            {
                if let Some(id) = normalized_id {
                    if self.messages[idx].message_id.is_none() {
                        self.messages[idx].message_id = Some(id.to_string());
                        self.active_assistant_id = Some(id.to_string());
                        self.messages[idx].stream_finished = false;
                        return idx;
                    }
                } else {
                    self.messages[idx].stream_finished = false;
                    return idx;
                }
            }
        }
        if let Some(id) = normalized_id.or(self.active_assistant_id.as_deref()) {
            if let Some(idx) = self.messages.iter().position(|message| {
                message.role == TranscriptRole::Assistant
                    && message.message_id.as_deref() == Some(id)
            }) {
                self.active_assistant_index = Some(idx);
                self.messages[idx].stream_finished = false;
                return idx;
            }
        }
        if let Some(idx) = self
            .messages
            .iter()
            .rposition(|message| message.role == TranscriptRole::Assistant)
        {
            let reusable_empty = self.messages[idx].message_id.is_none()
                && self.messages[idx].content.is_empty()
                && self.messages[idx].reasoning.is_empty()
                && self.messages[idx].tool_calls.is_empty();
            if reusable_empty {
                if let Some(id) = normalized_id {
                    self.messages[idx].message_id = Some(id.to_string());
                    self.active_assistant_id = Some(id.to_string());
                }
                self.active_assistant_index = Some(idx);
                self.messages[idx].stream_finished = false;
                return idx;
            }
        }
        self.messages.push(TranscriptMessage::assistant(
            normalized_id.map(str::to_string),
        ));
        self.active_assistant_id = normalized_id.map(str::to_string);
        self.active_assistant_index = Some(self.messages.len() - 1);
        self.messages.len() - 1
    }
}

pub fn delta_ops_from_value(value: &Value) -> Vec<DeltaOp> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|raw| {
            serde_json::from_value::<DeltaOp>(raw.clone())
                .unwrap_or_else(|_| DeltaOp::Unknown(UnknownDeltaOp::from_raw(raw.clone())))
        })
        .collect()
}

pub fn content_text(message: &Value) -> Option<String> {
    let content = match message.get("content")? {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(content_part_text)
            .collect::<Vec<_>>()
            .join("\n"),
        value => value_to_compact_string(value),
    };
    Some(sanitize_content_for_wire_role(
        message.get("role").and_then(Value::as_str),
        content,
    ))
}

fn sanitize_content_for_wire_role(role: Option<&str>, content: String) -> String {
    match role {
        Some("user") => sanitize_inbound_user_text(content),
        _ => sanitize_tool_text(content),
    }
}

fn sanitize_inbound_user_text(content: String) -> String {
    let mut sanitized = String::with_capacity(content.len());
    for part in content.split_inclusive('\t') {
        if let Some(text) = part.strip_suffix('\t') {
            sanitized.push_str(&sanitize_tool_text(text));
            sanitized.push('\t');
        } else {
            sanitized.push_str(&sanitize_tool_text(part));
        }
    }
    sanitized
}

fn sanitize_model_text_for_role(role: &TranscriptRole, content: String) -> String {
    match role {
        TranscriptRole::Assistant => sanitize_tool_text(content),
        _ => content,
    }
}

fn content_part_text(part: &Value) -> Option<String> {
    if content_part_is_image(part) {
        return Some(image_placeholder(part));
    }
    if content_part_is_file(part) {
        return Some(file_placeholder(part));
    }
    if content_part_is_audio(part) {
        return Some(audio_placeholder(part));
    }
    if let Some(kind) = content_part_type(part) {
        if !content_part_is_text_or_refusal(kind) {
            return Some(format!(
                "[content: {}] {}",
                sanitize_tool_inline(kind),
                sanitize_tool_text(value_to_compact_string(part))
            ));
        }
    }
    part.get("refusal")
        .or_else(|| {
            part.get("content")
                .filter(|_| content_part_type(part) == Some("refusal"))
        })
        .or_else(|| part.get("text"))
        .or_else(|| {
            part.get("m_content")
                .filter(|_| content_part_type(part) == Some("text"))
        })
        .or_else(|| part.get("input_text"))
        .or_else(|| part.get("output_text"))
        .or_else(|| part.get("content"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            part.get("content")
                .and_then(Value::as_array)
                .map(|parts| content_parts_text(parts))
        })
        .or_else(|| content_part_placeholder(part))
}

fn content_parts_text(parts: &[Value]) -> String {
    parts
        .iter()
        .filter_map(content_part_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn content_part_placeholder(part: &Value) -> Option<String> {
    let kind = content_part_type(part).unwrap_or_default();
    if kind == "text" {
        return None;
    }
    if content_part_is_image(part) {
        return Some(image_placeholder(part));
    }
    if content_part_is_file(part) {
        return Some(file_placeholder(part));
    }
    if content_part_is_audio(part) {
        return Some(audio_placeholder(part));
    }
    if kind.is_empty() {
        None
    } else {
        Some(format!(
            "[content: {}] {}",
            sanitize_tool_inline(kind),
            sanitize_tool_text(value_to_compact_string(part))
        ))
    }
}

fn content_part_type(part: &Value) -> Option<&str> {
    part.get("type")
        .or_else(|| part.get("m_type"))
        .and_then(Value::as_str)
}

fn content_part_is_text_or_refusal(kind: &str) -> bool {
    matches!(kind, "text" | "input_text" | "output_text" | "refusal")
}

fn content_part_is_image(part: &Value) -> bool {
    let kind = content_part_type(part).unwrap_or_default();
    kind.starts_with("image/")
        || matches!(kind, "image" | "image_url" | "input_image" | "output_image")
        || part.get("image_url").is_some()
}

fn content_part_is_file(part: &Value) -> bool {
    let kind = content_part_type(part).unwrap_or_default();
    matches!(kind, "file" | "input_file" | "output_file" | "document")
        || part.get("file_id").is_some()
        || part.get("filename").is_some()
        || part.get("file_name").is_some()
}

fn content_part_is_audio(part: &Value) -> bool {
    let kind = content_part_type(part).unwrap_or_default();
    kind.starts_with("audio/")
        || matches!(kind, "audio" | "audio_url" | "input_audio" | "output_audio")
        || part.get("audio").is_some()
        || part.get("input_audio").is_some()
        || part.get("output_audio").is_some()
}

fn image_placeholder(part: &Value) -> String {
    let mime = part
        .get("mime_type")
        .or_else(|| part.get("media_type"))
        .and_then(Value::as_str)
        .or_else(|| part.get("m_type").and_then(Value::as_str))
        .or_else(|| {
            part.get("source")
                .and_then(|source| source.get("media_type"))
                .and_then(Value::as_str)
        })
        .or_else(|| image_url(part).and_then(mime_from_data_url))
        .unwrap_or("image");
    match image_bytes(part) {
        Some(bytes) => format!("[image: {}, {} bytes]", sanitize_tool_inline(mime), bytes),
        None => format!("[image: {}]", sanitize_tool_inline(mime)),
    }
}

fn file_placeholder(part: &Value) -> String {
    let name = part
        .get("filename")
        .or_else(|| part.get("file_name"))
        .or_else(|| part.get("name"))
        .or_else(|| part.get("file_id"))
        .and_then(Value::as_str)
        .unwrap_or("file");
    let mime = part
        .get("mime_type")
        .or_else(|| part.get("media_type"))
        .and_then(Value::as_str);
    match (mime, file_bytes(part)) {
        (Some(mime), Some(bytes)) => format!(
            "[file: {}, {}, {} bytes]",
            sanitize_tool_inline(name),
            sanitize_tool_inline(mime),
            bytes
        ),
        (Some(mime), None) => format!(
            "[file: {}, {}]",
            sanitize_tool_inline(name),
            sanitize_tool_inline(mime)
        ),
        (None, Some(bytes)) => format!("[file: {}, {} bytes]", sanitize_tool_inline(name), bytes),
        (None, None) => format!("[file: {}]", sanitize_tool_inline(name)),
    }
}

fn audio_placeholder(part: &Value) -> String {
    let audio = part
        .get("audio")
        .or_else(|| part.get("input_audio"))
        .or_else(|| part.get("output_audio"));
    let mime = part
        .get("mime_type")
        .or_else(|| part.get("media_type"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            content_part_type(part)
                .filter(|kind| kind.starts_with("audio/"))
                .map(str::to_string)
        })
        .or_else(|| {
            audio
                .and_then(|audio| audio.get("mime_type").or_else(|| audio.get("media_type")))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            audio
                .and_then(|audio| audio.get("format"))
                .and_then(Value::as_str)
                .map(|format| format!("audio/{format}"))
        })
        .unwrap_or_else(|| "audio".to_string());
    match audio_bytes(part, audio) {
        Some(bytes) => format!("[audio: {}, {} bytes]", sanitize_tool_inline(&mime), bytes),
        None => format!("[audio: {}]", sanitize_tool_inline(&mime)),
    }
}

fn image_url(part: &Value) -> Option<&str> {
    part.get("image_url").and_then(|value| match value {
        Value::String(url) => Some(url.as_str()),
        Value::Object(map) => map.get("url").and_then(Value::as_str),
        _ => None,
    })
}

fn image_bytes(part: &Value) -> Option<usize> {
    image_url(part)
        .and_then(data_url_payload)
        .map(estimated_base64_bytes)
        .or_else(|| {
            part.get("m_content")
                .and_then(Value::as_str)
                .map(estimated_base64_bytes)
        })
        .or_else(|| {
            part.get("data")
                .and_then(Value::as_str)
                .map(estimated_base64_bytes)
        })
        .or_else(|| {
            part.get("source")
                .and_then(|source| source.get("data"))
                .and_then(Value::as_str)
                .map(estimated_base64_bytes)
        })
}

fn file_bytes(part: &Value) -> Option<usize> {
    part.get("bytes")
        .and_then(Value::as_u64)
        .map(|bytes| bytes as usize)
        .or_else(|| part.get("blob").and_then(Value::as_str).map(str::len))
        .or_else(|| part.get("data").and_then(Value::as_str).map(str::len))
}

fn audio_bytes(part: &Value, audio: Option<&Value>) -> Option<usize> {
    part.get("bytes")
        .and_then(Value::as_u64)
        .map(|bytes| bytes as usize)
        .or_else(|| {
            part.get("data")
                .and_then(Value::as_str)
                .map(estimated_base64_bytes)
        })
        .or_else(|| {
            audio
                .and_then(|audio| audio.get("bytes"))
                .and_then(Value::as_u64)
                .map(|bytes| bytes as usize)
        })
        .or_else(|| {
            audio
                .and_then(|audio| audio.get("data"))
                .and_then(Value::as_str)
                .map(estimated_base64_bytes)
        })
}

fn mime_from_data_url(url: &str) -> Option<&str> {
    url.strip_prefix("data:")?
        .split_once(';')
        .map(|(mime, _)| mime)
}

fn data_url_payload(url: &str) -> Option<&str> {
    url.strip_prefix("data:")?
        .split_once(',')
        .map(|(_, data)| data)
}

fn estimated_base64_bytes(data: &str) -> usize {
    let trimmed = data.trim_end_matches('=');
    trimmed.len().saturating_mul(3) / 4
}

fn value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        value => Some(value_to_compact_string(value)),
    }
}

fn value_to_compact_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn message_id(raw: &Value) -> Option<String> {
    raw.get("message_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extra_fields(raw: &Value) -> Map<String, Value> {
    let mut extra = raw.as_object().cloned().unwrap_or_default();
    if let Some(nested_extra) = raw.get("extra").and_then(Value::as_object) {
        extra.extend(nested_extra.clone());
    }
    for key in [
        "message_id",
        "role",
        "content",
        "finish_reason",
        "reasoning_content",
        "reasoning",
        "tool_calls",
        "tool_call_id",
        "tool_failed",
        "preserve",
        "usage",
        "checkpoints",
        "thinking_blocks",
        "citations",
        "server_content_blocks",
        "stream_finished",
        "summarized_range",
        "summarization_tier",
        "summarized_token_estimate",
        "extra",
    ] {
        extra.remove(key);
    }
    extra
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_delta_op_preserves_raw_payload() {
        let op: DeltaOp =
            serde_json::from_value(json!({"op":"future_op","payload":{"x":1}})).unwrap();

        match op {
            DeltaOp::Unknown(unknown) => {
                assert_eq!(unknown.op.as_deref(), Some("future_op"));
                assert_eq!(unknown.raw["payload"]["x"], 1);
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn subchat_update_parses_parent_tool_progress() {
        let event = SseEvent::from_raw(&json!({
            "type": "subchat_update",
            "tool_call_id": "call-1",
            "subchat_id": "1/2: search({})",
            "attached_files": ["src/lib.rs", "src/app.rs"],
            "depth": 7
        }));

        match event {
            SseEvent::SubchatUpdate {
                tool_call_id,
                subchat_id,
                attached_files,
                depth,
            } => {
                assert_eq!(tool_call_id, "call-1");
                assert_eq!(subchat_id, "1/2: search({})");
                assert_eq!(attached_files, vec!["src/lib.rs", "src/app.rs"]);
                assert_eq!(depth, 7);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn ack_parses_delivery_correlation_fields() {
        let event = SseEvent::from_raw(&json!({
            "type": "ack",
            "client_request_id": "request-1",
            "accepted": true,
            "result": {"queued": true}
        }));

        match event {
            SseEvent::Ack {
                client_request_id,
                accepted,
                result,
            } => {
                assert_eq!(client_request_id, "request-1");
                assert!(accepted);
                assert_eq!(result, Some(json!({"queued": true})));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn transcript_state_stores_all_known_delta_fields() {
        let mut state = TranscriptState::new();
        state.start_assistant(Some("a1"));
        let ops = delta_ops_from_value(&json!([
            {"op":"append_content","text":"hello"},
            {"op":"append_reasoning","text":"partial"},
            {"op":"set_reasoning","text":"think"},
            {"op":"set_tool_calls","tool_calls":[{"id":"call-1"}]},
            {"op":"set_thinking_blocks","blocks":[{"type":"thinking","signature":"sig"}]},
            {"op":"add_citation","citation":{"title":"README"}},
            {"op":"add_server_content_block","block":{"type":"web_search_call"}},
            {"op":"set_usage","usage":{"total_tokens":3}},
            {"op":"merge_extra","extra":{"metering":7}}
        ]));

        state.apply_delta_ops(Some("a1"), &ops);

        let message = &state.messages()[0];
        assert_eq!(message.content, "hello");
        assert_eq!(message.reasoning, "think");
        assert_eq!(message.tool_calls[0]["id"], "call-1");
        assert_eq!(message.thinking_blocks[0]["signature"], "sig");
        assert_eq!(message.citations[0]["title"], "README");
        assert_eq!(message.server_content_blocks[0]["type"], "web_search_call");
        assert_eq!(message.usage.as_ref().unwrap()["total_tokens"], 3);
        assert_eq!(message.extra["metering"], 7);
    }

    #[test]
    fn assistant_snapshot_and_stream_deltas_sanitize_terminal_escapes() {
        let injected = injected_model_text();
        let thinking_blocks = vec![json!({
            "type": "thinking",
            "thinking": "keep raw \u{1b}[31m thinking text",
            "signature": "sig\u{1b}]8;;http://signed\u{7}bytes",
        })];
        let raw_assistant = json!({
            "role": "assistant",
            "message_id": "a1",
            "content": injected,
            "reasoning_content": injected,
            "thinking_blocks": thinking_blocks.clone(),
        });

        let message = TranscriptMessage::from_wire(&raw_assistant);

        assert_escape_inert(&message.content);
        assert_escape_inert(&message.reasoning);
        assert_model_text_survives(&message.content);
        assert_model_text_survives(&message.reasoning);
        assert_eq!(message.thinking_blocks, thinking_blocks);

        let ops = delta_ops_from_value(&json!([
            {"op":"append_content","text": injected},
            {"op":"append_reasoning","text": injected},
            {"op":"set_thinking_blocks","blocks": thinking_blocks.clone()}
        ]));
        let mut state = TranscriptState::new();
        state.apply_delta_ops(Some("a2"), &ops);
        let streamed = &state.messages()[0];

        assert_escape_inert(&streamed.content);
        assert_escape_inert(&streamed.reasoning);
        assert_model_text_survives(&streamed.content);
        assert_model_text_survives(&streamed.reasoning);
        assert_eq!(streamed.thinking_blocks, thinking_blocks);
    }

    #[test]
    fn inbound_user_text_is_sanitized_and_preserves_ordinary_content() {
        let injected = injected_model_text();
        let raw_user = json!({
            "role": "user",
            "content": injected,
        });

        let message = TranscriptMessage::from_wire(&raw_user);

        assert_escape_inert(&message.content);
        assert_model_text_survives(&message.content);

        let ordinary = "Hello, café 👋\n\tkeep this tab";
        let ordinary_user = json!({"role": "user", "content": ordinary});
        assert_eq!(
            TranscriptMessage::from_wire(&ordinary_user).content,
            ordinary
        );
    }

    #[test]
    fn locally_typed_user_text_is_not_sanitized() {
        let injected = injected_model_text();
        let mut state = TranscriptState::new();

        state.push_user_message(injected);

        assert_eq!(state.messages()[0].content, injected);
    }

    #[test]
    fn sanitized_inbound_user_replay_deduplicates_by_message_id() {
        let mut state = TranscriptState::new();
        let message = json!({
            "message_id": "u1",
            "role": "user",
            "content": "hello\x1b]0;pwned\x07\x1b[2J world",
        });

        assert!(state.add_message(&message));
        assert!(!state.add_message(&message));
        assert_eq!(state.messages().len(), 1);
        assert_eq!(state.messages()[0].content, "hello world");
    }

    #[test]
    fn optimistic_user_message_reconciles_only_with_matching_client_message_id() {
        let mut state = TranscriptState::new();
        state.push_optimistic_user_message("same", "optimistic-1");
        let matching = TranscriptMessage::from_wire(&json!({
            "message_id": "server-1",
            "role": "user",
            "content": "same",
            "extra": {"client_message_id": "optimistic-1"},
        }));
        let different = TranscriptMessage::from_wire(&json!({
            "message_id": "server-2",
            "role": "user",
            "content": "same",
            "extra": {"client_message_id": "optimistic-2"},
        }));

        assert!(state.replace_optimistic_user_message_at(matching, None));
        assert!(!state.replace_optimistic_user_message_at(different, None));
        assert_eq!(state.messages().len(), 1);
        assert_eq!(state.messages()[0].message_id.as_deref(), Some("server-1"));
    }

    #[test]
    fn authoritative_index_replaces_existing_message_at_start_middle_and_end() {
        let initial = json!([
            {"message_id": "a", "role": "assistant", "content": "a", "stream_finished": true},
            {"message_id": "b", "role": "assistant", "content": "b", "stream_finished": true},
            {"message_id": "c", "role": "assistant", "content": "c", "stream_finished": true},
        ]);

        for (message_id, index, expected) in [
            ("c", 0, ["c", "a", "b"]),
            ("a", 1, ["b", "a", "c"]),
            ("a", 2, ["b", "c", "a"]),
        ] {
            let mut state = TranscriptState::new();
            state.reset_from_messages(initial.as_array().unwrap());
            assert!(!state.add_message_at(
                &json!({
                    "message_id": message_id,
                    "role": "assistant",
                    "content": format!("{message_id} updated"),
                    "stream_finished": true,
                }),
                Some(index),
            ));
            assert_eq!(
                state
                    .messages()
                    .iter()
                    .map(|message| message.message_id.as_deref().unwrap())
                    .collect::<Vec<_>>(),
                expected,
            );
        }
    }

    #[test]
    fn authoritative_index_moves_exact_optimistic_user_match() {
        let mut state = TranscriptState::new();
        state.add_message(&json!({"message_id": "before", "role": "user", "content": "before"}));
        state.push_optimistic_user_message("same", "client-1");
        state.add_message(&json!({"message_id": "after", "role": "user", "content": "after"}));

        assert!(state.replace_optimistic_user_message_at(
            TranscriptMessage::from_wire(&json!({
                "message_id": "server-1",
                "role": "user",
                "content": "same",
                "extra": {"client_message_id": "client-1"},
            })),
            Some(0),
        ));
        assert_eq!(
            state
                .messages()
                .iter()
                .map(|message| message.message_id.as_deref())
                .collect::<Vec<_>>(),
            [Some("server-1"), Some("before"), Some("after")],
        );
    }

    #[test]
    fn optimistic_reconciliation_removes_a_duplicate_server_message() {
        let mut state = TranscriptState::new();
        state.add_message(&json!({"message_id": "server-1", "role": "user", "content": "same"}));
        state.push_optimistic_user_message("same", "client-1");

        assert!(state.replace_optimistic_user_message_at(
            TranscriptMessage::from_wire(&json!({
                "message_id": "server-1",
                "role": "user",
                "content": "same",
                "extra": {"client_message_id": "client-1"},
            })),
            Some(0),
        ));
        assert_eq!(state.messages().len(), 1);
        assert_eq!(state.messages()[0].message_id.as_deref(), Some("server-1"));
        assert_eq!(state.messages()[0].client_message_id(), Some("client-1"));
    }

    #[test]
    fn authoritative_index_keeps_active_assistant_and_out_of_range_reconciliation_safe() {
        let mut state = TranscriptState::new();
        state.reset_from_messages(&[
            json!({"message_id": "a", "role": "assistant", "content": "a", "stream_finished": true}),
            json!({"message_id": "b", "role": "assistant", "content": "b", "stream_finished": false}),
            json!({"message_id": "c", "role": "assistant", "content": "c", "stream_finished": true}),
        ]);

        assert!(!state.add_message_at(
            &json!({
                "message_id": "b",
                "role": "assistant",
                "content": "b",
                "stream_finished": false,
            }),
            Some(0),
        ));
        state.apply_delta_ops(
            Some("b"),
            &[DeltaOp::AppendContent {
                text: " updated".to_string(),
            }],
        );
        assert_eq!(state.messages()[0].message_id.as_deref(), Some("b"));
        assert_eq!(state.messages()[0].content, "b updated");

        assert!(!state.add_message_at(
            &json!({
                "message_id": "a",
                "role": "assistant",
                "content": "a updated",
                "stream_finished": true,
            }),
            Some(99),
        ));
        assert_eq!(
            state.messages().last().unwrap().message_id.as_deref(),
            Some("a")
        );
        assert_eq!(state.messages().len(), 3);
    }

    #[test]
    fn content_text_keeps_multimodal_placeholders_and_text() {
        let message = json!({
            "role": "tool",
            "content": [
                {"type": "text", "text": "found it"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJDRA=="}},
                {"type": "file", "filename": "report.pdf", "mime_type": "application/pdf", "bytes": 1234}
            ]
        });

        let content = content_text(&message).unwrap();

        assert!(content.contains("found it"));
        assert!(content.contains("[image: image/png, 4 bytes]"));
        assert!(content.contains("[file: report.pdf, application/pdf, 1234 bytes]"));
        assert!(!content.contains("(no output)"));
    }

    #[test]
    fn content_text_keeps_sanitized_json_for_unknown_content_parts() {
        let message = json!({
            "role": "tool",
            "content": [{
                "type": "future_content",
                "text": "generic text",
                "refusal": "generic refusal",
                "input_text": "generic input",
                "output_text": "generic output",
                "content": "generic content",
                "nested": {"escape": "\u{1b}[31mred", "bell": "ring\u{7}"}
            }]
        });

        let content = content_text(&message).unwrap();

        assert_eq!(
            content,
            r#"[content: future_content] {"type":"future_content","text":"generic text","refusal":"generic refusal","input_text":"generic input","output_text":"generic output","content":"generic content","nested":{"escape":"\u001b[31mred","bell":"ring\u0007"}}"#
        );
        assert!(!content.contains('\x1b'));
        assert!(!content.contains('\x07'));
    }

    #[test]
    fn content_text_keeps_specialized_text_and_refusal_parts() {
        let message = json!({
            "role": "tool",
            "content": [
                {"type": "text", "text": "ordinary text"},
                {"type": "refusal", "refusal": "cannot comply"}
            ]
        });

        assert_eq!(
            content_text(&message).as_deref(),
            Some("ordinary text\ncannot comply")
        );
    }

    #[test]
    fn tool_content_text_sanitizes_escape_sequences() {
        let message = json!({"role": "tool", "content": "ok\x1b]0;pwned\x07\x1b[2Jdone"});
        let content = content_text(&message).unwrap();

        assert!(!content.contains('\x1b'));
        assert!(!content.contains('\x07'));
        assert!(!content.contains("pwned"));
        assert_eq!(content, "okdone");
    }

    fn injected_model_text() -> &'static str {
        "lead \u{1b}[31mred \u{1b}[2Jclear\u{7} bell \u{009b}31mcsi \u{1b}]8;;http://evil\u{7}TEXT\u{1b}]8;;\u{7} tail"
    }

    fn assert_escape_inert(text: &str) {
        assert!(!text.as_bytes().contains(&0x1b), "raw ESC in {text:?}");
        assert!(!text.as_bytes().contains(&0x07), "raw BEL in {text:?}");
        assert!(!text.as_bytes().contains(&0x9b), "raw CSI byte in {text:?}");
        assert!(!text.contains('\u{009b}'), "raw CSI char in {text:?}");
        assert!(!text.contains("http://evil"), "raw OSC8 URL in {text:?}");
    }

    fn assert_model_text_survives(text: &str) {
        for fragment in ["lead", "red", "clear", "bell", "csi", "TEXT", "tail"] {
            assert!(text.contains(fragment), "missing {fragment:?} in {text:?}");
        }
    }
}
