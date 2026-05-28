use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use tokio::task::JoinHandle;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::types::SessionState;
use crate::exec::{ExecOutputChunk, ExecOutputStream, ExecProcessId, ExecRegistry};
use crate::global_context::SharedGlobalContext;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const DEFAULT_MAX_DURATION_MS: u64 = 30_000;
const MIN_MAX_DURATION_MS: u64 = 1_000;
const MAX_MAX_DURATION_MS: u64 = 600_000;
const IDLE_WAIT_TIMEOUT: Duration = Duration::from_secs(1);
const ABORT_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub struct ToolProcessSubscribe {
    pub config_path: String,
}

struct SubscribeArgs {
    process_id: ExecProcessId,
    regex_filter: Option<Regex>,
    max_duration_ms: u64,
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
        let (gcx, exec_registry, chat_id, abort_flag) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.abort_flag.clone(),
            )
        };
        let snapshot = exec_registry
            .get(&parsed.process_id)
            .await
            .ok_or_else(|| format!("process not found: {}", parsed.process_id))?;
        if snapshot.status.is_terminal() {
            return Err(format!("process is not running: {}", parsed.process_id));
        }

        spawn_process_subscription(
            gcx,
            exec_registry,
            parsed.process_id.clone(),
            chat_id,
            parsed.regex_filter,
            Duration::from_millis(parsed.max_duration_ms),
            abort_flag,
        );

        Ok(tool_result(
            tool_call_id,
            json!({
                "subscribed": true,
                "max_duration_ms": parsed.max_duration_ms,
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
    regex_filter: Option<Regex>,
    max_duration: Duration,
    abort_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_process_subscription(
            gcx,
            exec_registry,
            process_id,
            chat_id,
            regex_filter,
            max_duration,
            abort_flag,
        )
        .await;
    })
}

async fn run_process_subscription(
    gcx: SharedGlobalContext,
    exec_registry: Arc<ExecRegistry>,
    process_id: ExecProcessId,
    chat_id: String,
    regex_filter: Option<Regex>,
    max_duration: Duration,
    abort_flag: Arc<AtomicBool>,
) {
    let mut output_rx = exec_registry.subscribe_output();
    let mut pending = String::new();
    let timeout = tokio::time::sleep(max_duration);
    tokio::pin!(timeout);
    let wait_process_id = process_id.clone();
    let process_finished = exec_registry.wait(&wait_process_id);
    tokio::pin!(process_finished);
    loop {
        if abort_flag.load(Ordering::Relaxed) {
            break;
        }
        tokio::select! {
            biased;
            _ = &mut timeout => break,
            _ = wait_for_abort(abort_flag.clone()) => break,
            received = output_rx.recv() => match received {
                Ok(chunk) => {
                    if should_process_chunk(&chunk, &process_id) {
                        emit_matching_lines(
                            gcx.clone(),
                            chat_id.clone(),
                            process_id.clone(),
                            &regex_filter,
                            abort_flag.clone(),
                            &mut pending,
                            &chunk.text,
                        );
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!("process subscription lagged by {count} output chunk(s)");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            result = &mut process_finished => {
                if let Err(error) = result {
                    tracing::debug!("process subscription wait ended for {process_id}: {error}");
                }
                break;
            }
        }
    }
    flush_pending_line(
        gcx,
        chat_id,
        process_id,
        &regex_filter,
        abort_flag,
        &mut pending,
    );
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

fn emit_matching_lines(
    gcx: SharedGlobalContext,
    chat_id: String,
    process_id: ExecProcessId,
    regex_filter: &Option<Regex>,
    abort_flag: Arc<AtomicBool>,
    pending: &mut String,
    text: &str,
) {
    pending.push_str(text);
    while let Some(newline_index) = pending.find('\n') {
        let mut line: String = pending.drain(..=newline_index).collect();
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
        emit_if_matches(
            gcx.clone(),
            chat_id.clone(),
            process_id.clone(),
            regex_filter,
            abort_flag.clone(),
            line,
        );
    }
}

fn flush_pending_line(
    gcx: SharedGlobalContext,
    chat_id: String,
    process_id: ExecProcessId,
    regex_filter: &Option<Regex>,
    abort_flag: Arc<AtomicBool>,
    pending: &mut String,
) {
    if pending.is_empty() {
        return;
    }
    let mut line = std::mem::take(pending);
    if line.ends_with('\r') {
        line.pop();
    }
    emit_if_matches(gcx, chat_id, process_id, regex_filter, abort_flag, line);
}

fn emit_if_matches(
    gcx: SharedGlobalContext,
    chat_id: String,
    process_id: ExecProcessId,
    regex_filter: &Option<Regex>,
    abort_flag: Arc<AtomicBool>,
    line: String,
) {
    if line_matches(regex_filter.as_ref(), &line) {
        tokio::spawn(async move {
            inject_line_when_idle(gcx, chat_id, process_id, line, abort_flag).await;
        });
    }
}

fn line_matches(regex_filter: Option<&Regex>, line: &str) -> bool {
    regex_filter
        .map(|regex| {
            regex
                .find(line)
                .map(|matched| matched.start() == 0 && matched.end() == line.len())
                .unwrap_or(false)
        })
        .unwrap_or(true)
}

async fn inject_line_when_idle(
    gcx: SharedGlobalContext,
    chat_id: String,
    process_id: ExecProcessId,
    line: String,
    abort_flag: Arc<AtomicBool>,
) {
    if abort_flag.load(Ordering::Relaxed) {
        return;
    }
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(&chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return;
    };
    loop {
        if gcx.shutdown_flag.load(Ordering::Relaxed) {
            return;
        }
        if abort_flag.load(Ordering::Relaxed) {
            return;
        }
        let notify = {
            let mut session = session_arc.lock().await;
            if session.closed {
                return;
            }
            if is_stream_busy(session.runtime.state) {
                session.queue_notify.clone()
            } else {
                session.add_message(subscription_message(process_id, line));
                return;
            }
        };
        let _ = tokio::time::timeout(IDLE_WAIT_TIMEOUT, notify.notified()).await;
    }
}

fn is_stream_busy(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Generating | SessionState::ExecutingTools
    )
}

fn subscription_message(process_id: ExecProcessId, line: String) -> ChatMessage {
    event(
        EventSubkind::SystemNotice,
        "exec.subscribe",
        json!({"process_id": process_id, "line": line}),
        line,
    )
}

fn process_subscribe_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "process_id": { "type": "string" },
            "regex_filter": { "type": "string", "description": "Optional regex. Only matching lines fire notifications. Empty = all lines." },
            "max_duration_ms": { "type": "integer", "default": DEFAULT_MAX_DURATION_MS, "minimum": MIN_MAX_DURATION_MS, "maximum": MAX_MAX_DURATION_MS }
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
    Ok(SubscribeArgs {
        process_id,
        regex_filter,
        max_duration_ms,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::internal_roles::EVENT_ROLE;
    use crate::chat::types::ChatSession;
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
            "Start-Sleep -Milliseconds 100; [Console]::Out.Write(\"READY`nNOT_READY`nREADY`n\")"
                .to_string()
        } else {
            "sleep 0.1; printf 'READY\nNOT_READY\nREADY\n'".to_string()
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
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
                ("regex_filter", json!("READY")),
                ("max_duration_ms", json!(5_000)),
            ]),
        )
        .await;
        assert_eq!(
            serde_json::from_str::<Value>(&message.content.content_text_only()).unwrap(),
            json!({"subscribed": true, "max_duration_ms": 5_000, "process_id": process_id.as_str()})
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
            None,
            Duration::from_secs(10),
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
            None,
            Duration::from_millis(50),
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
