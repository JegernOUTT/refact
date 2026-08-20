use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use tokio::task::JoinHandle;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::exec::types::normalize_workspace_path;
use crate::exec::{ExecOutputChunk, ExecOutputStream, ExecProcessId, ExecRegistry};
use crate::files_correction::get_active_project_path;
use crate::global_context::SharedGlobalContext;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::file_edit::auxiliary::active_execution_scope;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::scope::ExecutionScope;

const DEFAULT_MAX_DURATION_MS: u64 = 30_000;
const MIN_MAX_DURATION_MS: u64 = 1_000;
const MAX_MAX_DURATION_MS: u64 = 600_000;
const IDLE_FLUSH_INTERVAL: Duration = Duration::from_millis(100);
const ABORT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const DEFAULT_MAX_EVENTS: usize = 100;
const MAX_MAX_EVENTS: usize = 1_000;
const DEFAULT_MAX_LINE_BYTES: usize = 8_192;
const MAX_MAX_LINE_BYTES: usize = 65_536;
const TRUNCATION_MARKER: &str = "[truncated]";
const MIN_MAX_LINE_BYTES: usize = TRUNCATION_MARKER.len();
const RATE_LIMIT_PER_SECOND: usize = 10;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);
const SUMMARY_PREVIEW_BYTES: usize = 200;

pub struct ToolProcessSubscribe {
    pub config_path: String,
}

struct SubscribeArgs {
    process_id: ExecProcessId,
    regex_filter: Option<Regex>,
    max_duration_ms: u64,
    max_events: usize,
    max_line_bytes: usize,
}

struct SubscriptionConfig {
    regex_filter: Option<Regex>,
    max_duration: Duration,
    max_events: usize,
    max_line_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SubscribeEvent {
    Line(String),
    Summary {
        line_count: usize,
        first_line: String,
        last_line: String,
    },
}

#[async_trait]
impl Tool for ToolProcessSubscribe {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let parsed = parse_subscribe_args(args)?;
        let (gcx, exec_registry, chat_id, execution_scope, abort_flag) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.execution_scope.clone(),
                ccx_lock.abort_flag.clone(),
            )
        };
        let workspace = current_workspace(gcx.clone(), execution_scope.as_ref()).await;
        let snapshot = exec_registry
            .authorize_process_access(&parsed.process_id, &chat_id, workspace.as_deref())
            .await?;
        if snapshot.status.is_terminal() {
            return Err(format!("process is not running: {}", parsed.process_id));
        }

        spawn_process_subscription(
            gcx,
            exec_registry,
            parsed.process_id.clone(),
            chat_id,
            SubscriptionConfig {
                regex_filter: parsed.regex_filter,
                max_duration: Duration::from_millis(parsed.max_duration_ms),
                max_events: parsed.max_events,
                max_line_bytes: parsed.max_line_bytes,
            },
            abort_flag,
        );

        Ok(tool_result(
            tool_call_id,
            json!({
                "subscribed": true,
                "max_duration_ms": parsed.max_duration_ms,
                "max_events": parsed.max_events,
                "max_line_bytes": parsed.max_line_bytes,
                "process_id": parsed.process_id.as_str(),
            })
            .to_string(),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "process_subscribe".to_string(),
            display_name: "Process Subscribe".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Subscribe to a running process's stdout for a bounded window. Each output line that matches the optional regex_filter is injected as a notification event in the chat. Auto-cancels when the process exits or after max_duration_ms. Useful for waiting for 'ready' markers, watching for failures, etc.".to_string(),
            input_schema: process_subscribe_input_schema(),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

fn spawn_process_subscription(
    gcx: SharedGlobalContext,
    exec_registry: Arc<ExecRegistry>,
    process_id: ExecProcessId,
    chat_id: String,
    config: SubscriptionConfig,
    abort_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_process_subscription(gcx, exec_registry, process_id, chat_id, config, abort_flag).await;
    })
}

async fn run_process_subscription(
    gcx: SharedGlobalContext,
    exec_registry: Arc<ExecRegistry>,
    process_id: ExecProcessId,
    chat_id: String,
    config: SubscriptionConfig,
    abort_flag: Arc<AtomicBool>,
) {
    let mut output_rx = exec_registry.subscribe_output();
    let mut state = SubscriptionState::new(
        config.regex_filter,
        config.max_events,
        config.max_line_bytes,
    );
    let timeout = tokio::time::sleep(config.max_duration);
    tokio::pin!(timeout);
    let wait_process_id = process_id.clone();
    let process_finished = exec_registry.wait(&wait_process_id);
    tokio::pin!(process_finished);
    let mut input_finished = false;
    loop {
        if gcx.shutdown_flag.load(Ordering::Relaxed) || abort_flag.load(Ordering::Relaxed) {
            break;
        }
        if input_finished {
            if state.needs_timer() {
                state.finalize_ready_summary(Instant::now());
                if !try_flush_events_when_idle(
                    gcx.clone(),
                    &chat_id,
                    &process_id,
                    &mut state.events,
                    abort_flag.clone(),
                )
                .await
                {
                    break;
                }
            } else {
                break;
            }
        }
        tokio::select! {
            biased;
            _ = &mut timeout, if !input_finished => {
                let now = Instant::now();
                state.flush_pending_line(now);
                state.finalize_summaries();
                if !try_flush_events_when_idle(
                    gcx.clone(),
                    &chat_id,
                    &process_id,
                    &mut state.events,
                    abort_flag.clone(),
                ).await {
                    break;
                }
                input_finished = true;
            },
            _ = wait_for_abort(abort_flag.clone()) => break,
            received = output_rx.recv(), if !input_finished => match received {
                Ok(chunk) => {
                    if should_process_chunk(&chunk, &process_id) {
                        if state.push_text(&chunk.text, Instant::now()) {
                            state.finalize_summaries();
                            input_finished = true;
                        }
                        if !try_flush_events_when_idle(
                            gcx.clone(),
                            &chat_id,
                            &process_id,
                            &mut state.events,
                            abort_flag.clone(),
                        ).await {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!("process subscription lagged by {count} output chunk(s)");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            result = &mut process_finished, if !input_finished => {
                if let Err(error) = result {
                    tracing::debug!("process subscription wait ended for {process_id}: {error}");
                }
                let now = Instant::now();
                state.flush_pending_line(now);
                state.finalize_summaries();
                input_finished = true;
            },
            _ = tokio::time::sleep(IDLE_FLUSH_INTERVAL), if state.needs_timer() => {
                state.finalize_ready_summary(Instant::now());
                if !try_flush_events_when_idle(
                    gcx.clone(),
                    &chat_id,
                    &process_id,
                    &mut state.events,
                    abort_flag.clone(),
                ).await {
                    break;
                }
            }
        }
    }
}

async fn wait_for_abort(abort_flag: Arc<AtomicBool>) {
    while !abort_flag.load(Ordering::Relaxed) {
        tokio::time::sleep(ABORT_POLL_INTERVAL).await;
    }
}

fn should_process_chunk(chunk: &ExecOutputChunk, process_id: &ExecProcessId) -> bool {
    chunk.process_id == *process_id
        && matches!(
            chunk.stream,
            ExecOutputStream::Stdout | ExecOutputStream::Combined
        )
}

struct SubscriptionState {
    regex_filter: Option<Regex>,
    max_events: usize,
    max_line_bytes: usize,
    pending: String,
    pending_truncated: bool,
    events: VecDeque<SubscribeEvent>,
    event_count: usize,
    recent_event_times: VecDeque<Instant>,
    active_summary: Option<SummaryBuilder>,
}

impl SubscriptionState {
    fn new(regex_filter: Option<Regex>, max_events: usize, max_line_bytes: usize) -> Self {
        Self {
            regex_filter,
            max_events,
            max_line_bytes: max_line_bytes.clamp(MIN_MAX_LINE_BYTES, MAX_MAX_LINE_BYTES),
            pending: String::new(),
            pending_truncated: false,
            events: VecDeque::new(),
            event_count: 0,
            recent_event_times: VecDeque::new(),
            active_summary: None,
        }
    }

    fn push_text(&mut self, text: &str, now: Instant) -> bool {
        if self.reached_cap() {
            return true;
        }
        for segment in text.split_inclusive('\n') {
            if let Some(line_part) = segment.strip_suffix('\n') {
                self.append_pending(line_part);
                let mut line = std::mem::take(&mut self.pending);
                self.pending_truncated = false;
                if line.ends_with('\r') {
                    line.pop();
                }
                if self.enqueue_line(line, now) {
                    return true;
                }
            } else {
                self.append_pending(segment);
            }
        }
        false
    }

    fn append_pending(&mut self, text: &str) {
        if text.is_empty() || self.pending_truncated {
            return;
        }
        if self.pending.len() + text.len() <= self.max_line_bytes {
            self.pending.push_str(text);
            return;
        }

        let prefix_limit = self.max_line_bytes.saturating_sub(TRUNCATION_MARKER.len());
        truncate_string_to_byte_limit(&mut self.pending, prefix_limit);
        let remaining = prefix_limit.saturating_sub(self.pending.len());
        if remaining > 0 {
            self.pending.push_str(prefix_by_bytes(text, remaining));
        }
        self.pending.push_str(TRUNCATION_MARKER);
        self.pending_truncated = true;
    }

    fn flush_pending_line(&mut self, now: Instant) -> bool {
        if self.reached_cap() {
            return true;
        }
        if self.pending.is_empty() {
            return false;
        }
        let mut line = std::mem::take(&mut self.pending);
        self.pending_truncated = false;
        if line.ends_with('\r') {
            line.pop();
        }
        self.enqueue_line(line, now)
    }

    fn enqueue_line(&mut self, line: String, now: Instant) -> bool {
        if !line_matches(self.regex_filter.as_ref(), &line) {
            return false;
        }
        self.finalize_ready_summary(now);
        self.drop_old_event_times(now);
        if self.event_count >= self.max_events {
            return true;
        }
        if self.active_summary.is_some() || self.recent_event_times.len() >= RATE_LIMIT_PER_SECOND {
            self.add_summary_line(line, now);
            return self.event_count >= self.max_events;
        }
        self.events.push_back(SubscribeEvent::Line(line));
        self.event_count += 1;
        self.recent_event_times.push_back(now);
        self.event_count >= self.max_events
    }

    fn add_summary_line(&mut self, line: String, now: Instant) {
        if let Some(summary) = &mut self.active_summary {
            summary.add_line(line);
            return;
        }
        if self.event_count >= self.max_events {
            return;
        }
        self.active_summary = Some(SummaryBuilder::new(now, line));
        self.event_count += 1;
    }

    fn finalize_ready_summary(&mut self, now: Instant) {
        let should_finalize = self
            .active_summary
            .as_ref()
            .is_some_and(|summary| duration_since(now, summary.started_at) >= RATE_LIMIT_WINDOW);
        if should_finalize {
            self.finalize_summaries();
        }
    }

    fn finalize_summaries(&mut self) {
        if let Some(summary) = self.active_summary.take() {
            self.events.push_back(summary.into_event());
        }
    }

    fn drop_old_event_times(&mut self, now: Instant) {
        while self
            .recent_event_times
            .front()
            .is_some_and(|time| duration_since(now, *time) >= RATE_LIMIT_WINDOW)
        {
            self.recent_event_times.pop_front();
        }
    }

    fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    fn needs_timer(&self) -> bool {
        self.has_events() || self.active_summary.is_some()
    }

    fn reached_cap(&self) -> bool {
        self.event_count >= self.max_events
    }
}

struct SummaryBuilder {
    started_at: Instant,
    line_count: usize,
    first_line: String,
    last_line: String,
}

impl SummaryBuilder {
    fn new(started_at: Instant, line: String) -> Self {
        let preview = preview_line(&line);
        Self {
            started_at,
            line_count: 1,
            first_line: preview.clone(),
            last_line: preview,
        }
    }

    fn add_line(&mut self, line: String) {
        self.line_count += 1;
        self.last_line = preview_line(&line);
    }

    fn into_event(self) -> SubscribeEvent {
        SubscribeEvent::Summary {
            line_count: self.line_count,
            first_line: self.first_line,
            last_line: self.last_line,
        }
    }
}

fn duration_since(now: Instant, earlier: Instant) -> Duration {
    now.checked_duration_since(earlier).unwrap_or_default()
}

fn prefix_by_bytes(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn truncate_string_to_byte_limit(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

fn preview_line(line: &str) -> String {
    prefix_by_bytes(line, SUMMARY_PREVIEW_BYTES).to_string()
}

fn line_matches(regex_filter: Option<&Regex>, line: &str) -> bool {
    regex_filter
        .map(|regex| regex.is_match(line))
        .unwrap_or(true)
}

async fn try_flush_events_when_idle(
    gcx: SharedGlobalContext,
    chat_id: &str,
    process_id: &ExecProcessId,
    events: &mut VecDeque<SubscribeEvent>,
    abort_flag: Arc<AtomicBool>,
) -> bool {
    if events.is_empty() {
        return true;
    }
    if abort_flag.load(Ordering::Relaxed) {
        return false;
    }
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return false;
    };
    if gcx.shutdown_flag.load(Ordering::Relaxed) || abort_flag.load(Ordering::Relaxed) {
        return false;
    }
    let mut session = session_arc.lock().await;
    if session.closed {
        return false;
    }
    if !session.is_idle() {
        return true;
    }
    while let Some(event) = events.pop_front() {
        session.add_message(subscription_message(process_id.clone(), event));
    }
    true
}

fn subscription_message(process_id: ExecProcessId, event_value: SubscribeEvent) -> ChatMessage {
    match event_value {
        SubscribeEvent::Line(line) => event(
            EventSubkind::SystemNotice,
            "exec.subscribe",
            json!({"process_id": process_id, "line": line}),
            line,
        ),
        SubscribeEvent::Summary {
            line_count,
            first_line,
            last_line,
        } => {
            let content = format!(
                "Process {} matched {line_count} additional lines; first: {first_line}; last: {last_line}",
                process_id.as_str()
            );
            event(
                EventSubkind::SystemNotice,
                "exec.subscribe",
                json!({
                    "process_id": process_id,
                    "summary": true,
                    "line_count": line_count,
                    "first_line": first_line,
                    "last_line": last_line,
                }),
                content,
            )
        }
    }
}

async fn current_workspace(
    gcx: SharedGlobalContext,
    execution_scope: Option<&ExecutionScope>,
) -> Option<std::path::PathBuf> {
    if let Some(scope) = active_execution_scope(execution_scope) {
        return Some(normalize_workspace_path(scope.effective_root()));
    }
    get_active_project_path(gcx)
        .await
        .map(|path| normalize_workspace_path(&path))
}

fn process_subscribe_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "process_id": { "type": "string" },
            "regex_filter": { "type": "string", "description": "Optional regex. Only matching lines fire notifications. Empty = all lines." },
            "max_duration_ms": { "type": "integer", "default": DEFAULT_MAX_DURATION_MS, "minimum": MIN_MAX_DURATION_MS, "maximum": MAX_MAX_DURATION_MS },
            "max_events": { "type": "integer", "default": DEFAULT_MAX_EVENTS, "minimum": 1, "maximum": MAX_MAX_EVENTS },
            "max_line_bytes": { "type": "integer", "default": DEFAULT_MAX_LINE_BYTES, "minimum": MIN_MAX_LINE_BYTES, "maximum": MAX_MAX_LINE_BYTES }
        },
        "required": ["process_id"]
    })
}

fn tool_result(tool_call_id: &String, content: String) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content),
            tool_call_id: tool_call_id.clone(),
            output_filter: Some(OutputFilter::no_limits()),
            ..Default::default()
        })],
    )
}

fn parse_subscribe_args(args: &HashMap<String, Value>) -> Result<SubscribeArgs, String> {
    let process_id = parse_process_id(args)?;
    let regex_filter = parse_optional_string(args, "regex_filter")?
        .map(|filter| Regex::new(&filter).map_err(|error| format!("invalid regex_filter: {error}")))
        .transpose()?;
    let max_duration_ms = parse_max_duration_ms(args)?;
    let max_events = parse_usize_arg(args, "max_events", DEFAULT_MAX_EVENTS, 1, MAX_MAX_EVENTS)?;
    let max_line_bytes = parse_usize_arg(
        args,
        "max_line_bytes",
        DEFAULT_MAX_LINE_BYTES,
        MIN_MAX_LINE_BYTES,
        MAX_MAX_LINE_BYTES,
    )?;
    Ok(SubscribeArgs {
        process_id,
        regex_filter,
        max_duration_ms,
        max_events,
        max_line_bytes,
    })
}

fn parse_process_id(args: &HashMap<String, Value>) -> Result<ExecProcessId, String> {
    let process_id = match args.get("process_id") {
        Some(Value::String(value)) if !value.trim().is_empty() => value.trim().to_string(),
        Some(Value::String(_)) => return Err("Argument `process_id` cannot be empty".to_string()),
        Some(value) => return Err(format!("argument `process_id` is not a string: {value:?}")),
        None => return Err("Missing argument `process_id`".to_string()),
    };
    if !process_id.starts_with("exec_") {
        return Err("process_id must be a runtime-owned exec_* ID".to_string());
    }
    Ok(ExecProcessId(process_id))
}

fn parse_optional_string(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    match args.get(name) {
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!("argument `{name}` is not a string: {value:?}")),
    }
}

fn parse_max_duration_ms(args: &HashMap<String, Value>) -> Result<u64, String> {
    let value = match args.get("max_duration_ms") {
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| "argument `max_duration_ms` must be an integer".to_string())?,
        Some(Value::String(value)) if value.trim().is_empty() => DEFAULT_MAX_DURATION_MS,
        Some(Value::String(value)) => value
            .trim()
            .parse::<u64>()
            .map_err(|_| "argument `max_duration_ms` must be an integer".to_string())?,
        Some(Value::Null) | None => DEFAULT_MAX_DURATION_MS,
        Some(value) => {
            return Err(format!(
                "argument `max_duration_ms` is not an integer: {value:?}"
            ));
        }
    };
    if !(MIN_MAX_DURATION_MS..=MAX_MAX_DURATION_MS).contains(&value) {
        return Err(format!(
            "argument `max_duration_ms` must be between {MIN_MAX_DURATION_MS} and {MAX_MAX_DURATION_MS}"
        ));
    }
    Ok(value)
}

fn parse_usize_arg(
    args: &HashMap<String, Value>,
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, String> {
    let value = match args.get(name) {
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| format!("argument `{name}` must be an integer"))?,
        Some(Value::String(value)) if value.trim().is_empty() => default as u64,
        Some(Value::String(value)) => value
            .trim()
            .parse::<u64>()
            .map_err(|_| format!("argument `{name}` must be an integer"))?,
        Some(Value::Null) | None => default as u64,
        Some(value) => return Err(format!("argument `{name}` is not an integer: {value:?}")),
    };
    let value = usize::try_from(value)
        .map_err(|_| format!("argument `{name}` must be between {min} and {max}"))?;
    if !(min..=max).contains(&value) {
        return Err(format!("argument `{name}` must be between {min} and {max}"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::internal_roles::EVENT_ROLE;
    use crate::chat::types::{ChatSession, SessionState};
    use crate::exec::{ExecMode, ExecOwnerMeta, ExecSpawnRequest};
    use crate::global_context::GlobalContext;

    async fn test_context(
        chat_id: &str,
    ) -> (
        Arc<GlobalContext>,
        Arc<AMutex<AtCommandsContext>>,
        Arc<AMutex<ChatSession>>,
    ) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let session = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        gcx.chat_sessions
            .write()
            .await
            .insert(chat_id.to_string(), session.clone());
        let ccx = AtCommandsContext::new_with_abort(
            AppState::from_gcx(gcx.clone()).await,
            4096,
            20,
            false,
            Vec::new(),
            chat_id.to_string(),
            None,
            "model".to_string(),
            None,
            None,
            None,
        )
        .await;
        (gcx, Arc::new(AMutex::new(ccx)), session)
    }

    fn args(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    async fn run_tool(
        tool: &mut ToolProcessSubscribe,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: HashMap<String, Value>,
    ) -> ChatMessage {
        let (_, messages) = tool
            .tool_execute(ccx, &"tool_call".to_string(), &args)
            .await
            .unwrap();
        match messages.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => message,
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        }
    }

    async fn run_tool_error(
        tool: &mut ToolProcessSubscribe,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: HashMap<String, Value>,
    ) -> String {
        tool.tool_execute(ccx, &"tool_call".to_string(), &args)
            .await
            .unwrap_err()
    }

    fn owner(chat_id: &str) -> ExecOwnerMeta {
        ExecOwnerMeta {
            chat_id: Some(chat_id.to_string()),
            tool_call_id: Some("tool-call".to_string()),
            service_name: None,
            workspace: None,
        }
    }

    async fn spawn_background(
        gcx: &Arc<GlobalContext>,
        chat_id: &str,
        command: String,
    ) -> ExecProcessId {
        gcx.exec_registry
            .spawn(
                ExecSpawnRequest::new(ExecMode::Background, command)
                    .with_owner(owner(chat_id))
                    .with_short_description("test process"),
            )
            .await
            .unwrap()
            .snapshot
            .meta
            .process_id
    }

    fn ready_command() -> String {
        if cfg!(target_os = "windows") {
            "Start-Sleep -Milliseconds 100; [Console]::Out.Write('READY' + [char]10 + 'NOT_READY' + [char]10 + 'READY' + [char]10)"
                .to_string()
        } else {
            "sleep 0.1; printf 'READY\nNOT_READY\nREADY\n'".to_string()
        }
    }

    fn subscribe_duration_ms() -> u64 {
        if cfg!(target_os = "windows") {
            60_000
        } else {
            5_000
        }
    }

    fn short_sleep_command() -> String {
        if cfg!(target_os = "windows") {
            "Start-Sleep -Milliseconds 50".to_string()
        } else {
            "sleep 0.05".to_string()
        }
    }

    fn long_sleep_command() -> String {
        if cfg!(target_os = "windows") {
            "Start-Sleep -Seconds 30".to_string()
        } else {
            "sleep 30".to_string()
        }
    }

    async fn subscribe_events(session: &Arc<AMutex<ChatSession>>) -> Vec<ChatMessage> {
        let session = session.lock().await;
        session
            .messages
            .iter()
            .filter(|message| is_subscribe_event(message))
            .cloned()
            .collect()
    }

    fn is_subscribe_event(message: &ChatMessage) -> bool {
        message.role == EVENT_ROLE
            && message
                .extra
                .get("event")
                .and_then(|event| event.get("subkind"))
                .and_then(Value::as_str)
                == Some("system_notice")
            && message
                .extra
                .get("event")
                .and_then(|event| event.get("source"))
                .and_then(Value::as_str)
                == Some("exec.subscribe")
    }

    async fn wait_for_subscribe_event_count(
        session: &Arc<AMutex<ChatSession>>,
        expected_count: usize,
    ) -> Vec<ChatMessage> {
        let wait_secs = if cfg!(target_os = "windows") { 60 } else { 2 };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_secs);
        loop {
            let events = subscribe_events(session).await;
            if events.len() >= expected_count {
                return events;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "process subscribe events not injected"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_subscribe_event_count_for(
        session: &Arc<AMutex<ChatSession>>,
        expected_count: usize,
        timeout: Duration,
    ) -> Option<Vec<ChatMessage>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let events = subscribe_events(session).await;
            if events.len() >= expected_count {
                return Some(events);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn subscribe_injects_matching_line() {
        let chat_id = "subscribe-injects-matching-line";
        let (gcx, ccx, session) = test_context(chat_id).await;
        let process_id = spawn_background(&gcx, chat_id, ready_command()).await;
        let mut tool = ToolProcessSubscribe {
            config_path: String::new(),
        };

        let message = run_tool(
            &mut tool,
            ccx,
            args(vec![
                ("process_id", json!(process_id.as_str())),
                ("regex_filter", json!("^READY$")),
                ("max_duration_ms", json!(subscribe_duration_ms())),
            ]),
        )
        .await;
        assert_eq!(
            serde_json::from_str::<Value>(&message.content.content_text_only()).unwrap(),
            json!({
                "subscribed": true,
                "max_duration_ms": subscribe_duration_ms(),
                "max_events": DEFAULT_MAX_EVENTS,
                "max_line_bytes": DEFAULT_MAX_LINE_BYTES,
                "process_id": process_id.as_str()
            })
        );
        let _ = gcx.exec_registry.wait(&process_id).await.unwrap();

        wait_for_subscribe_event_count(&session, 2).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let events = subscribe_events(&session).await;
        assert_eq!(events.len(), 2);
        assert_eq!(
            events
                .iter()
                .map(|message| message.extra["event"]["payload"]["line"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["READY", "READY"]
        );
        assert!(events
            .iter()
            .all(|message| message.extra["event"]["payload"]["process_id"] == json!(process_id)));
    }

    #[test]
    fn regex_filter_uses_substring_match() {
        let regex = Regex::new("error").unwrap();

        assert!(line_matches(Some(&regex), "fatal error: bind failed"));
    }

    #[test]
    fn pending_no_newline_buffer_is_truncated() {
        let mut state = SubscriptionState::new(None, DEFAULT_MAX_EVENTS, 32);

        state.push_text(&"x".repeat(1_000), Instant::now());

        assert_eq!(state.pending.len(), 32);
        assert!(state.pending.ends_with(TRUNCATION_MARKER));
    }

    #[test]
    fn max_events_caps_injections() {
        let mut state = SubscriptionState::new(None, 3, DEFAULT_MAX_LINE_BYTES);

        let reached_cap = state.push_text("one\ntwo\nthree\nfour\n", Instant::now());

        assert!(reached_cap);
        assert_eq!(state.event_count, 3);
        assert_eq!(state.events.len(), 3);
    }

    #[test]
    fn high_rate_matches_are_coalesced() {
        let mut state = SubscriptionState::new(None, DEFAULT_MAX_EVENTS, DEFAULT_MAX_LINE_BYTES);

        state.push_text(
            &format!(
                "{}\n",
                (0..12)
                    .map(|i| format!("line-{i}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            Instant::now(),
        );
        state.finalize_summaries();

        assert_eq!(state.events.len(), RATE_LIMIT_PER_SECOND + 1);
        assert!(matches!(
            state.events.back().unwrap(),
            SubscribeEvent::Summary { line_count: 2, .. }
        ));
    }

    #[tokio::test]
    async fn paused_session_gets_no_injections_until_idle() {
        let chat_id = "paused-session-gets-no-injections-until-idle";
        let (gcx, ccx, session) = test_context(chat_id).await;
        {
            let mut session = session.lock().await;
            session.set_runtime_state(SessionState::Paused, None);
        }
        let process_id = spawn_background(&gcx, chat_id, ready_command()).await;
        let mut tool = ToolProcessSubscribe {
            config_path: String::new(),
        };

        let _ = run_tool(
            &mut tool,
            ccx,
            args(vec![
                ("process_id", json!(process_id.as_str())),
                ("regex_filter", json!("READY")),
                ("max_duration_ms", json!(subscribe_duration_ms())),
            ]),
        )
        .await;
        let _ = gcx.exec_registry.wait(&process_id).await.unwrap();
        assert!(
            wait_for_subscribe_event_count_for(&session, 1, Duration::from_millis(200))
                .await
                .is_none()
        );

        {
            let mut session = session.lock().await;
            session.set_runtime_state(SessionState::Idle, None);
        }

        wait_for_subscribe_event_count(&session, 2).await;
    }

    #[tokio::test]
    async fn cross_chat_process_subscribe_denied() {
        let (gcx, _owner_ccx, _owner_session) = test_context("subscribe-owner-chat").await;
        let (_other_gcx, other_ccx, _other_session) = test_context("subscribe-other-chat").await;
        {
            let mut ccx_lock = other_ccx.lock().await;
            ccx_lock.app = AppState::from_gcx(gcx.clone()).await;
            ccx_lock.global_context = gcx.clone();
        }
        let process_id = spawn_background(&gcx, "subscribe-owner-chat", long_sleep_command()).await;
        let mut tool = ToolProcessSubscribe {
            config_path: String::new(),
        };

        let err = run_tool_error(
            &mut tool,
            other_ccx,
            args(vec![
                ("process_id", json!(process_id.as_str())),
                ("max_duration_ms", json!(1_000)),
            ]),
        )
        .await;

        assert_eq!(err, format!("process access denied: {process_id}"));
        gcx.exec_registry.kill(&process_id).await.unwrap();
    }

    #[tokio::test]
    async fn auto_cancel_on_process_exit() {
        let chat_id = "auto-cancel-on-process-exit";
        let (gcx, _ccx, _session) = test_context(chat_id).await;
        let process_id = spawn_background(&gcx, chat_id, short_sleep_command()).await;
        let handle = spawn_process_subscription(
            gcx.clone(),
            gcx.exec_registry.clone(),
            process_id.clone(),
            chat_id.to_string(),
            SubscriptionConfig {
                regex_filter: None,
                max_duration: Duration::from_secs(10),
                max_events: DEFAULT_MAX_EVENTS,
                max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            },
            Arc::new(AtomicBool::new(false)),
        );

        let _ = gcx.exec_registry.wait(&process_id).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn auto_cancel_on_max_duration() {
        let chat_id = "auto-cancel-on-max-duration";
        let (gcx, _ccx, _session) = test_context(chat_id).await;
        let process_id = spawn_background(&gcx, chat_id, long_sleep_command()).await;
        let handle = spawn_process_subscription(
            gcx.clone(),
            gcx.exec_registry.clone(),
            process_id.clone(),
            chat_id.to_string(),
            SubscriptionConfig {
                regex_filter: None,
                max_duration: Duration::from_millis(50),
                max_events: DEFAULT_MAX_EVENTS,
                max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            },
            Arc::new(AtomicBool::new(false)),
        );

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
        let _ = gcx.exec_registry.kill(&process_id).await;
    }

    #[test]
    fn process_subscribe_is_registered() {
        let names = crate::tools::tools_list::builtin_system_tools(String::new())
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"process_subscribe".to_string()));
    }
}
