use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum SseEvent {
    Snapshot {
        thread: Option<Value>,
        runtime: Option<Value>,
        messages: Vec<Value>,
    },
    StreamStarted {
        message_id: Option<String>,
    },
    StreamDelta {
        message_id: Option<String>,
        ops: Vec<DeltaOp>,
    },
    StreamFinished {
        message_id: Option<String>,
        usage: Option<Value>,
        finish_reason: Option<Value>,
    },
    RuntimeUpdated,
    QueueUpdated {
        queue_size: usize,
        queued_items: Vec<Value>,
    },
    PauseRequired,
    PauseCleared,
    MessageAdded {
        message: Option<Value>,
    },
    Unknown {
        kind: String,
        raw: Value,
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
            "snapshot" => Self::Snapshot {
                thread: raw.get("thread").cloned(),
                runtime: raw.get("runtime").cloned(),
                messages: raw
                    .get("messages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            },
            "stream_started" => Self::StreamStarted {
                message_id: message_id(raw),
            },
            "stream_delta" => Self::StreamDelta {
                message_id: message_id(raw),
                ops: delta_ops_from_value(raw.get("ops").unwrap_or(&Value::Null)),
            },
            "stream_finished" => Self::StreamFinished {
                message_id: message_id(raw),
                usage: raw.get("usage").cloned(),
                finish_reason: raw.get("finish_reason").cloned(),
            },
            "runtime_updated" => Self::RuntimeUpdated,
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
            "message_added" => Self::MessageAdded {
                message: raw.get("message").or_else(|| raw.get("msg")).cloned(),
            },
            _ => Self::Unknown {
                kind: kind.to_string(),
                raw: raw.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeltaOp {
    AppendContent { text: String },
    AppendReasoning { text: String },
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
            KnownDeltaOp::AppendContent { text } => Self::AppendContent { text },
            KnownDeltaOp::AppendReasoning { text } => Self::AppendReasoning { text },
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
    Notice,
    Other(String),
}

impl TranscriptRole {
    pub fn from_wire(role: &str) -> Self {
        match role {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "tool" => Self::Tool,
            "notice" => Self::Notice,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Notice => "notice",
            Self::Other(role) => role.as_str(),
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
        }
    }

    pub fn assistant(message_id: Option<String>) -> Self {
        let mut message = Self::new(TranscriptRole::Assistant);
        message.message_id = message_id;
        message
    }

    pub fn from_wire(raw: &Value) -> Self {
        let role = raw
            .get("role")
            .and_then(Value::as_str)
            .map(TranscriptRole::from_wire)
            .unwrap_or_else(|| TranscriptRole::Other(String::new()));
        let mut message = Self::new(role);
        message.message_id = raw
            .get("message_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        message.content = content_text(raw).unwrap_or_default();
        message.reasoning = raw
            .get("reasoning_content")
            .or_else(|| raw.get("reasoning"))
            .and_then(value_to_text)
            .unwrap_or_default();
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
            .find(|(_, message)| message.role == TranscriptRole::Assistant)
            .map(|(idx, _)| idx);
        self.active_assistant_id = self
            .active_assistant_index
            .and_then(|idx| self.messages[idx].message_id.clone());
    }

    pub fn push_notice(&mut self, text: impl Into<String>) {
        let mut message = TranscriptMessage::new(TranscriptRole::Notice);
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

    pub fn add_message(&mut self, raw: &Value) {
        let message = TranscriptMessage::from_wire(raw);
        if let Some(usage) = message.usage.clone() {
            self.usage = Some(usage);
        }
        let is_assistant = message.role == TranscriptRole::Assistant;
        self.messages.push(message);
        if is_assistant {
            let idx = self.messages.len() - 1;
            self.active_assistant_id = self.messages[idx].message_id.clone();
            self.active_assistant_index = Some(idx);
        }
    }

    pub fn start_assistant(&mut self, message_id: Option<&str>) {
        self.ensure_assistant_index(message_id);
    }

    pub fn finish_assistant(&mut self, message_id: Option<&str>, usage: Option<Value>) {
        if let Some(usage) = usage {
            let idx = self.ensure_assistant_index(message_id);
            self.messages[idx].usage = Some(usage.clone());
            self.usage = Some(usage);
        }
    }

    pub fn apply_delta_ops(&mut self, message_id: Option<&str>, ops: &[DeltaOp]) {
        for op in ops {
            let idx = self.ensure_assistant_index(message_id);
            match op {
                DeltaOp::AppendContent { text } => self.messages[idx].content.push_str(text),
                DeltaOp::AppendReasoning { text } => self.messages[idx].reasoning.push_str(text),
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
                    }
                    self.active_assistant_id = Some(id.to_string());
                }
                return idx;
            }
        }
        if let Some(id) = normalized_id.or(self.active_assistant_id.as_deref()) {
            if let Some(idx) = self.messages.iter().position(|message| {
                message.role == TranscriptRole::Assistant
                    && message.message_id.as_deref() == Some(id)
            }) {
                self.active_assistant_index = Some(idx);
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
    match message.get("content")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        value => Some(value_to_compact_string(value)),
    }
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
        "summarized_range",
        "summarization_tier",
        "summarized_token_estimate",
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
    fn transcript_state_stores_all_known_delta_fields() {
        let mut state = TranscriptState::new();
        state.start_assistant(Some("a1"));
        let ops = delta_ops_from_value(&json!([
            {"op":"append_content","text":"hello"},
            {"op":"append_reasoning","text":"think"},
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
}
