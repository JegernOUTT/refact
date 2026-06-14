use serde::Deserialize;
use serde_json::{Map, Value};

use crate::text_safety::sanitize_tool_inline;

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
    ThreadUpdated {
        params: Value,
    },
    MessageAdded {
        message: Option<Value>,
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
            "thread_updated" => Self::ThreadUpdated {
                params: raw.clone(),
            },
            "message_added" => Self::MessageAdded {
                message: raw.get("message").or_else(|| raw.get("msg")).cloned(),
            },
            "message_updated" => Self::MessageUpdated {
                message_id: message_id(raw),
                message: raw.get("message").or_else(|| raw.get("msg")).cloned(),
            },
            "message_removed" => Self::MessageRemoved {
                message_id: message_id(raw),
            },
            "messages_truncated" => Self::MessagesTruncated {
                from_index: raw
                    .get("from_index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .unwrap_or(usize::MAX),
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
    Plan,
    Event,
    Other(String),
}

impl TranscriptRole {
    pub fn from_wire(role: &str) -> Self {
        match role {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "tool" => Self::Tool,
            "notice" => Self::Notice,
            "plan" => Self::Plan,
            "event" => Self::Event,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Notice => "notice",
            Self::Plan => "plan",
            Self::Event => "event",
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
        message.stream_finished = raw
            .get("stream_finished")
            .and_then(Value::as_bool)
            .unwrap_or(true);
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

    pub fn add_message(&mut self, raw: &Value) -> bool {
        let mut message = TranscriptMessage::from_wire(raw);
        if matches!(
            message.role,
            TranscriptRole::Assistant | TranscriptRole::Tool
        ) && raw
            .get("stream_finished")
            .and_then(Value::as_bool)
            .is_none()
        {
            message.stream_finished = true;
        }
        self.add_transcript_message(message)
    }

    pub fn update_message(&mut self, message_id: Option<&str>, raw: &Value) -> bool {
        let mut message = TranscriptMessage::from_wire(raw);
        if message.message_id.is_none() {
            message.message_id = message_id
                .filter(|value| !value.is_empty())
                .map(str::to_string);
        }
        if matches!(
            message.role,
            TranscriptRole::Assistant | TranscriptRole::Tool
        ) && raw
            .get("stream_finished")
            .and_then(Value::as_bool)
            .is_none()
        {
            message.stream_finished = true;
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
            self.add_transcript_message(message)
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

    fn add_transcript_message(&mut self, message: TranscriptMessage) -> bool {
        if let Some(idx) = self.message_index_by_id(message.message_id.as_deref()) {
            if let Some(usage) = message.usage.clone() {
                self.usage = Some(usage);
            }
            self.messages[idx] = message;
            self.refresh_cached_indexes();
            return false;
        }
        if let Some(usage) = message.usage.clone() {
            self.usage = Some(usage);
        }
        let is_assistant = message.role == TranscriptRole::Assistant;
        self.messages.push(message);
        if is_assistant
            && !self
                .messages
                .last()
                .is_some_and(|message| message.stream_finished)
        {
            let idx = self.messages.len() - 1;
            self.active_assistant_id = self.messages[idx].message_id.clone();
            self.active_assistant_index = Some(idx);
        }
        true
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
                    }
                    self.active_assistant_id = Some(id.to_string());
                }
                self.messages[idx].stream_finished = false;
                return idx;
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
    if message.get("role").and_then(Value::as_str) == Some("tool") {
        Some(crate::text_safety::sanitize_tool_text(content))
    } else {
        Some(content)
    }
}

fn content_part_text(part: &Value) -> Option<String> {
    if content_part_is_image(part) {
        return Some(image_placeholder(part));
    }
    if content_part_is_file(part) {
        return Some(file_placeholder(part));
    }
    part.get("text")
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
    if kind.is_empty() {
        None
    } else {
        Some(format!("[{}]", sanitize_tool_inline(kind)))
    }
}

fn content_part_type(part: &Value) -> Option<&str> {
    part.get("type")
        .or_else(|| part.get("m_type"))
        .and_then(Value::as_str)
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
    fn tool_content_text_sanitizes_escape_sequences() {
        let message = json!({"role": "tool", "content": "ok\x1b]0;pwned\x07\x1b[2Jdone"});
        let content = content_text(&message).unwrap();

        assert!(!content.contains('\x1b'));
        assert!(!content.contains('\x07'));
        assert!(!content.contains("pwned"));
        assert_eq!(content, "okdone");
    }
}
