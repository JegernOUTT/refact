use std::ffi::OsString;
use std::future::Future;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::daemon::chat_client::{ChatClientError, ProxyChatClient, ToolDecision};
use crate::daemon::client::DaemonClientError;
use crate::daemon::state::DaemonInfo;

pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    Agent,
    Explore,
}

impl RunMode {
    fn as_str(&self) -> &'static str {
        match self {
            RunMode::Agent => "agent",
            RunMode::Explore => "explore",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPolicy {
    Deny,
    Ask,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOptions {
    pub project: Option<PathBuf>,
    pub mode: RunMode,
    pub model: Option<String>,
    pub approve: ApprovalPolicy,
    pub json: bool,
    pub timeout_secs: u64,
    pub prompt: String,
    pub listen_ctrl_c: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunErrorKind {
    Unreachable,
    ProjectOpen,
    Chat,
    ApprovalDenied,
    Timeout,
    Interrupted,
}

impl RunErrorKind {
    fn as_str(self) -> &'static str {
        match self {
            RunErrorKind::Unreachable => "unreachable",
            RunErrorKind::ProjectOpen => "project_open",
            RunErrorKind::Chat => "chat",
            RunErrorKind::ApprovalDenied => "approval_denied",
            RunErrorKind::Timeout => "timeout",
            RunErrorKind::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallSummary {
    pub name: String,
    pub args_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunJsonSummary {
    pub chat_id: String,
    pub final_text: String,
    pub usage: Option<Value>,
    pub tool_calls: Vec<ToolCallSummary>,
}

#[derive(Debug)]
struct RunFailure {
    kind: RunErrorKind,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RunJsonFailure {
    ok: bool,
    error: String,
    kind: String,
    exit_code: i32,
}

pub trait RunIo {
    fn write_stdout(&mut self, text: &str);
    fn write_stderr(&mut self, text: &str);
    fn flush_stdout(&mut self);
    fn stdin_is_tty(&self) -> bool;
    fn read_stdin_line(&mut self) -> Option<String>;
}

pub struct StdRunIo;

impl RunIo for StdRunIo {
    fn write_stdout(&mut self, text: &str) {
        print!("{text}");
        let _ = std::io::stdout().flush();
    }

    fn write_stderr(&mut self, text: &str) {
        eprint!("{text}");
        let _ = std::io::stderr().flush();
    }

    fn flush_stdout(&mut self) {
        let _ = std::io::stdout().flush();
    }

    fn stdin_is_tty(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn read_stdin_line(&mut self) -> Option<String> {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok()?;
        Some(line)
    }
}

struct RunState {
    chat_id: String,
    final_text: String,
    usage: Option<Value>,
    tool_calls: Vec<ToolCallSummary>,
    stream_finished: bool,
    runtime_idle: bool,
    user_sent: bool,
    auto_banner_printed: bool,
}

impl RunState {
    fn new(chat_id: String) -> Self {
        Self {
            chat_id,
            final_text: String::new(),
            usage: None,
            tool_calls: Vec::new(),
            stream_finished: false,
            runtime_idle: false,
            user_sent: false,
            auto_banner_printed: false,
        }
    }

    fn is_complete(&self) -> bool {
        self.user_sent && self.stream_finished && self.runtime_idle
    }

    fn summary(&self) -> RunJsonSummary {
        RunJsonSummary {
            chat_id: self.chat_id.clone(),
            final_text: self.final_text.clone(),
            usage: self.usage.clone(),
            tool_calls: self.tool_calls.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenProjectResponse {
    project_id: String,
}

pub fn parse_run_args(args: &[OsString]) -> Result<RunOptions, String> {
    let mut project = None;
    let mut mode = RunMode::Agent;
    let mut model = None;
    let mut approve = ApprovalPolicy::Deny;
    let mut json = false;
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut prompt_parts = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        let arg = os_to_string(&args[i])?;
        match arg.as_str() {
            "--project" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--project requires a path".to_string())?;
                project = Some(PathBuf::from(os_to_string(value)?));
            }
            "--mode" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--mode requires agent or explore".to_string())?;
                mode = parse_mode(&os_to_string(value)?)?;
            }
            "--model" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--model requires a model id".to_string())?;
                let value = os_to_string(value)?;
                if value.is_empty() {
                    return Err("--model requires a non-empty model id".to_string());
                }
                model = Some(value);
            }
            "--approve" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--approve requires deny, ask, or auto".to_string())?;
                approve = parse_approval(&os_to_string(value)?)?;
            }
            "--json" => json = true,
            "--timeout-secs" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--timeout-secs requires a positive integer".to_string())?;
                timeout_secs = os_to_string(value)?
                    .parse::<u64>()
                    .map_err(|_| "--timeout-secs requires a positive integer".to_string())?;
                if timeout_secs == 0 {
                    return Err("--timeout-secs requires a positive integer".to_string());
                }
            }
            "--" => {
                for value in args.iter().skip(i + 1) {
                    prompt_parts.push(os_to_string(value)?);
                }
                break;
            }
            value if value.starts_with('-') => return Err(format!("unknown run option `{value}`")),
            _ => prompt_parts.push(arg),
        }
        i += 1;
    }
    let prompt = prompt_parts.join(" ").trim().to_string();
    if prompt.is_empty() {
        return Err("run prompt is required".to_string());
    }
    Ok(RunOptions {
        project,
        mode,
        model,
        approve,
        json,
        timeout_secs,
        prompt,
        listen_ctrl_c: true,
    })
}

pub fn exit_code_for(kind: RunErrorKind) -> i32 {
    match kind {
        RunErrorKind::Unreachable => 1,
        RunErrorKind::ProjectOpen => 1,
        RunErrorKind::Chat => 2,
        RunErrorKind::ApprovalDenied => 3,
        RunErrorKind::Timeout => 4,
        RunErrorKind::Interrupted => 130,
    }
}

pub fn approval_decision(
    policy: ApprovalPolicy,
    stdin_is_tty: bool,
    answer: Option<&str>,
) -> ApprovalDecision {
    match policy {
        ApprovalPolicy::Deny => ApprovalDecision::Deny,
        ApprovalPolicy::Auto => ApprovalDecision::Approve,
        ApprovalPolicy::Ask if !stdin_is_tty => ApprovalDecision::Deny,
        ApprovalPolicy::Ask => match answer
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "y" | "yes" => ApprovalDecision::Approve,
            _ => ApprovalDecision::Deny,
        },
    }
}

pub async fn run(options: RunOptions, io: &mut dyn RunIo) -> i32 {
    let json_output = options.json;
    let result = match crate::daemon::client::ensure_daemon_running().await {
        Ok(info) => run_with_daemon_info(options, info, io).await,
        Err(error) => Err(RunFailure {
            kind: RunErrorKind::Unreachable,
            message: format!("daemon unreachable: {error}"),
        }),
    };
    finish_result(result, json_output, io)
}

async fn run_with_daemon_info(
    options: RunOptions,
    daemon: DaemonInfo,
    io: &mut dyn RunIo,
) -> Result<RunJsonSummary, RunFailure> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(options.timeout_secs);
    let root = resolve_project_root(&options)?;
    let project = with_setup_deadline(
        deadline,
        options.timeout_secs,
        "open project",
        open_project(&daemon, &root),
    )
    .await?;
    let chat_id = uuid::Uuid::new_v4().to_string();
    let client =
        ProxyChatClient::from_daemon_info(&daemon, project.project_id).map_err(map_client_error)?;
    drive_chat(&client, &options, chat_id, deadline, io).await
}

fn finish_result(
    result: Result<RunJsonSummary, RunFailure>,
    json_output: bool,
    io: &mut dyn RunIo,
) -> i32 {
    match result {
        Ok(_) => 0,
        Err(error) => {
            let exit_code = exit_code_for(error.kind);
            if json_output {
                write_json_failure(io, &error, exit_code);
            } else if !error.message.is_empty() {
                io.write_stderr(&format!("{}\n", error.message));
            }
            exit_code
        }
    }
}

fn write_json_failure(io: &mut dyn RunIo, error: &RunFailure, exit_code: i32) {
    let failure = RunJsonFailure {
        ok: false,
        error: error.message.clone(),
        kind: error.kind.as_str().to_string(),
        exit_code,
    };
    match serde_json::to_string(&failure) {
        Ok(json) => io.write_stdout(&format!("{json}\n")),
        Err(encode_error) => io.write_stdout(&format!(
            "{{\"ok\":false,\"error\":\"failed to encode run JSON: {encode_error}\",\"kind\":\"chat\",\"exit_code\":2}}\n"
        )),
    }
}

async fn with_setup_deadline<T, F>(
    deadline: tokio::time::Instant,
    timeout_secs: u64,
    step: &str,
    future: F,
) -> Result<T, RunFailure>
where
    F: Future<Output = Result<T, RunFailure>>,
{
    match tokio::time::timeout_at(deadline, future).await {
        Ok(result) => result,
        Err(_) => Err(RunFailure {
            kind: RunErrorKind::Timeout,
            message: format!(
                "refact run setup timed out after {timeout_secs} seconds while {step}"
            ),
        }),
    }
}

async fn with_run_deadline<T, F>(
    deadline: tokio::time::Instant,
    timeout_secs: u64,
    step: &str,
    future: F,
) -> Result<T, RunFailure>
where
    F: Future<Output = Result<T, RunFailure>>,
{
    match tokio::time::timeout_at(deadline, future).await {
        Ok(result) => result,
        Err(_) => Err(RunFailure {
            kind: RunErrorKind::Timeout,
            message: format!("refact run timed out after {timeout_secs} seconds while {step}"),
        }),
    }
}

async fn drive_chat(
    client: &ProxyChatClient,
    options: &RunOptions,
    chat_id: String,
    deadline: tokio::time::Instant,
    io: &mut dyn RunIo,
) -> Result<RunJsonSummary, RunFailure> {
    let mut stream = with_setup_deadline(deadline, options.timeout_secs, "subscribe", async {
        client.subscribe(&chat_id).await.map_err(map_client_error)
    })
    .await?;
    let patch = set_params_patch(options);
    with_setup_deadline(deadline, options.timeout_secs, "send set_params", async {
        client
            .send_set_params(&chat_id, request_id("set-params"), patch)
            .await
            .map_err(map_client_error)
    })
    .await?;
    with_setup_deadline(deadline, options.timeout_secs, "send user message", async {
        client
            .send_user_message(&chat_id, request_id("user-message"), &options.prompt)
            .await
            .map_err(map_client_error)
    })
    .await?;

    let mut state = RunState::new(chat_id.clone());
    state.user_sent = true;
    let mut retried_subscribe = false;

    loop {
        if state.is_complete() {
            io.flush_stdout();
            let summary = state.summary();
            if options.json {
                let json = serde_json::to_string(&summary).map_err(|error| RunFailure {
                    kind: RunErrorKind::Chat,
                    message: format!("failed to encode run JSON: {error}"),
                })?;
                io.write_stdout(&format!("{json}\n"));
            }
            return Ok(summary);
        }

        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                let abort_client = client.clone();
                let abort_chat_id = chat_id.clone();
                tokio::spawn(async move {
                    let _ = abort_client
                        .send_abort(&abort_chat_id, request_id("timeout-abort"))
                        .await;
                });
                return Err(RunFailure {
                    kind: RunErrorKind::Timeout,
                    message: format!("refact run timed out after {} seconds", options.timeout_secs),
                });
            }
            _ = ctrl_c(options.listen_ctrl_c) => {
                let _ = client.send_abort(&chat_id, request_id("ctrl-c-abort")).await;
                return Err(RunFailure {
                    kind: RunErrorKind::Interrupted,
                    message: "interrupted".to_string(),
                });
            }
            event = stream.next() => {
                match event {
                    Some(Ok(event)) => {
                        match handle_event(client, options, &mut state, event, deadline, io).await? {
                            EventOutcome::Continue => {}
                            EventOutcome::Denied => {
                                return Err(RunFailure {
                                    kind: RunErrorKind::ApprovalDenied,
                                    message: "tool approval denied".to_string(),
                                });
                            }
                        }
                    }
                    Some(Err(error)) => {
                        if retried_subscribe {
                            return Err(map_client_error(error));
                        }
                        retried_subscribe = true;
                        stream = with_run_deadline(deadline, options.timeout_secs, "resubscribe", async {
                            client.subscribe(&chat_id).await.map_err(map_client_error)
                        })
                        .await?;
                    }
                    None => {
                        if retried_subscribe {
                            return Err(RunFailure {
                                kind: RunErrorKind::Chat,
                                message: "chat SSE stream ended before completion".to_string(),
                            });
                        }
                        retried_subscribe = true;
                        stream = with_run_deadline(deadline, options.timeout_secs, "resubscribe", async {
                            client.subscribe(&chat_id).await.map_err(map_client_error)
                        })
                        .await?;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventOutcome {
    Continue,
    Denied,
}

async fn handle_event(
    client: &ProxyChatClient,
    options: &RunOptions,
    state: &mut RunState,
    event: Value,
    deadline: tokio::time::Instant,
    io: &mut dyn RunIo,
) -> Result<EventOutcome, RunFailure> {
    match event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "snapshot" => handle_snapshot(state, &event, options, io)?,
        "stream_started" => {
            state.stream_finished = false;
            state.runtime_idle = false;
        }
        "stream_delta" => handle_stream_delta(state, &event, options, io),
        "stream_finished" => {
            if event
                .get("finish_reason")
                .and_then(Value::as_str)
                .is_some_and(|reason| reason == "error")
            {
                return Err(RunFailure {
                    kind: RunErrorKind::Chat,
                    message: "chat stream finished with error".to_string(),
                });
            }
            state.stream_finished = true;
        }
        "runtime_updated" => handle_runtime_updated(state, &event)?,
        "pause_required" => {
            return handle_pause_required(client, options, state, &event, deadline, io).await;
        }
        "ack" => {
            if event.get("accepted").and_then(Value::as_bool) == Some(false) {
                return Err(RunFailure {
                    kind: RunErrorKind::Chat,
                    message: format!("chat command rejected: {event}"),
                });
            }
        }
        _ => {}
    }
    Ok(EventOutcome::Continue)
}

fn handle_snapshot(
    state: &mut RunState,
    event: &Value,
    options: &RunOptions,
    io: &mut dyn RunIo,
) -> Result<(), RunFailure> {
    if let Some(text) = latest_assistant_text(event) {
        let old = state.final_text.clone();
        if text.starts_with(&old) {
            let suffix = &text[old.len()..];
            if !options.json && !suffix.is_empty() {
                io.write_stdout(suffix);
            }
        } else if !options.json {
            io.write_stdout(&format!("\n--- reconnect ---\n{text}"));
        }
        state.final_text = text;
    }
    if let Some(runtime) = event.get("runtime") {
        if let Some(error) = runtime.get("error").and_then(Value::as_str) {
            if !error.is_empty() {
                return Err(RunFailure {
                    kind: RunErrorKind::Chat,
                    message: error.to_string(),
                });
            }
        }
        let idle = runtime.get("state").and_then(Value::as_str) == Some("idle");
        state.runtime_idle = idle;
        if state.user_sent && idle && !state.final_text.is_empty() {
            state.stream_finished = true;
        }
    }
    Ok(())
}

fn handle_runtime_updated(state: &mut RunState, event: &Value) -> Result<(), RunFailure> {
    if let Some(error) = event.get("error").and_then(Value::as_str) {
        if !error.is_empty() {
            return Err(RunFailure {
                kind: RunErrorKind::Chat,
                message: error.to_string(),
            });
        }
    }
    match event
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "idle" => state.runtime_idle = true,
        "error" => {
            return Err(RunFailure {
                kind: RunErrorKind::Chat,
                message: event
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("chat runtime error")
                    .to_string(),
            })
        }
        _ => state.runtime_idle = false,
    }
    Ok(())
}

fn handle_stream_delta(
    state: &mut RunState,
    event: &Value,
    options: &RunOptions,
    io: &mut dyn RunIo,
) {
    let Some(ops) = event.get("ops").and_then(Value::as_array) else {
        return;
    };
    for op in ops {
        match op.get("op").and_then(Value::as_str).unwrap_or_default() {
            "append_content" => {
                if let Some(text) = op.get("text").and_then(Value::as_str) {
                    state.final_text.push_str(text);
                    if !options.json {
                        io.write_stdout(text);
                    }
                }
            }
            "set_usage" => {
                if let Some(usage) = op.get("usage") {
                    state.usage = Some(usage.clone());
                }
            }
            "set_tool_calls" => {
                if let Some(calls) = op.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        let summary = summarize_tool_call(call);
                        if !options.json {
                            io.write_stderr(&format!(
                                "→ {}({})\n",
                                summary.name, summary.args_preview
                            ));
                        }
                        state.tool_calls.push(summary);
                    }
                }
            }
            _ => {}
        }
    }
}

async fn handle_pause_required(
    client: &ProxyChatClient,
    options: &RunOptions,
    state: &mut RunState,
    event: &Value,
    deadline: tokio::time::Instant,
    io: &mut dyn RunIo,
) -> Result<EventOutcome, RunFailure> {
    let reasons = event
        .get("reasons")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let tty = io.stdin_is_tty();
    let answer = if matches!(options.approve, ApprovalPolicy::Ask) && tty && !options.json {
        io.write_stderr(&format!("Approve {} tool call(s)? [y/N] ", reasons.len()));
        io.read_stdin_line()
    } else {
        None
    };
    match approval_decision(options.approve, tty, answer.as_deref()) {
        ApprovalDecision::Approve => {
            if matches!(options.approve, ApprovalPolicy::Auto)
                && !state.auto_banner_printed
                && !options.json
            {
                io.write_stderr("warning: --approve auto approves all requested tools\n");
                state.auto_banner_printed = true;
            }
            let decisions = tool_decisions(&reasons, true);
            with_run_deadline(
                deadline,
                options.timeout_secs,
                "send tool approval",
                async {
                    client
                        .send_tool_decisions(&state.chat_id, request_id("approve-tools"), decisions)
                        .await
                        .map_err(map_client_error)
                },
            )
            .await?;
            Ok(EventOutcome::Continue)
        }
        ApprovalDecision::Deny => {
            if !options.json {
                for reason in &reasons {
                    io.write_stderr(&format!(
                        "denied {}: {}\n",
                        tool_name(reason),
                        deny_reason(reason)
                    ));
                }
            }
            let decisions = tool_decisions(&reasons, false);
            if !decisions.is_empty() {
                with_run_deadline(deadline, options.timeout_secs, "send tool denial", async {
                    client
                        .send_tool_decisions(&state.chat_id, request_id("deny-tools"), decisions)
                        .await
                        .map_err(map_client_error)
                })
                .await?;
            }
            Ok(EventOutcome::Denied)
        }
    }
}

fn tool_decisions(reasons: &[Value], accepted: bool) -> Vec<ToolDecision> {
    reasons
        .iter()
        .filter_map(|reason| {
            reason
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(|tool_call_id| ToolDecision {
                    tool_call_id: tool_call_id.to_string(),
                    accepted,
                })
        })
        .collect()
}

async fn open_project(
    daemon: &DaemonInfo,
    root: &PathBuf,
) -> Result<OpenProjectResponse, RunFailure> {
    crate::daemon::client::post_json(
        daemon,
        "/daemon/v1/projects/open",
        &json!({"root": root.to_string_lossy()}),
    )
    .await
    .map_err(project_open_error)
}

fn project_open_error(error: DaemonClientError) -> RunFailure {
    let message = match error {
        DaemonClientError::Http(message) => format!(
            "daemon project open failed: {}",
            message
                .strip_prefix("failed to contact daemon: ")
                .unwrap_or(&message)
        ),
        DaemonClientError::Status { status, body } => {
            format!("daemon project open failed with status {status}: {body}")
        }
        DaemonClientError::Json(message) => {
            format!("daemon project open returned invalid JSON: {message}")
        }
    };
    RunFailure {
        kind: RunErrorKind::ProjectOpen,
        message,
    }
}

fn resolve_project_root(options: &RunOptions) -> Result<PathBuf, RunFailure> {
    let root = match &options.project {
        Some(path) => path.clone(),
        None => std::env::current_dir().map_err(|error| RunFailure {
            kind: RunErrorKind::Unreachable,
            message: format!("failed to read current directory: {error}"),
        })?,
    };
    Ok(root)
}

fn set_params_patch(options: &RunOptions) -> Value {
    let mut patch = serde_json::Map::new();
    patch.insert("mode".to_string(), json!(options.mode.as_str()));
    patch.insert("tool_use".to_string(), json!(options.mode.as_str()));
    if let Some(model) = &options.model {
        patch.insert("model".to_string(), json!(model));
    }
    Value::Object(patch)
}

fn request_id(prefix: &str) -> String {
    format!("run-{prefix}-{}", uuid::Uuid::new_v4())
}

fn ctrl_c(enabled: bool) -> impl std::future::Future<Output = ()> {
    async move {
        if enabled {
            let _ = tokio::signal::ctrl_c().await;
        } else {
            std::future::pending::<()>().await;
        }
    }
}

fn map_client_error(error: ChatClientError) -> RunFailure {
    let kind = if error.is_unreachable() {
        RunErrorKind::Unreachable
    } else {
        RunErrorKind::Chat
    };
    RunFailure {
        kind,
        message: error.to_string(),
    }
}

fn os_to_string(value: &OsString) -> Result<String, String> {
    value
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| "arguments must be valid UTF-8".to_string())
}

fn parse_mode(value: &str) -> Result<RunMode, String> {
    match value {
        "agent" => Ok(RunMode::Agent),
        "explore" => Ok(RunMode::Explore),
        _ => Err("--mode must be agent or explore".to_string()),
    }
}

fn parse_approval(value: &str) -> Result<ApprovalPolicy, String> {
    match value {
        "deny" => Ok(ApprovalPolicy::Deny),
        "ask" => Ok(ApprovalPolicy::Ask),
        "auto" => Ok(ApprovalPolicy::Auto),
        _ => Err("--approve must be deny, ask, or auto".to_string()),
    }
}

fn summarize_tool_call(call: &Value) -> ToolCallSummary {
    let name = call
        .get("function")
        .and_then(|function| function.get("name"))
        .or_else(|| call.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_string();
    let args = call
        .get("function")
        .and_then(|function| function.get("arguments"))
        .or_else(|| call.get("arguments"))
        .or_else(|| call.get("args"))
        .or_else(|| call.get("input"));
    ToolCallSummary {
        name,
        args_preview: args_preview(args),
    }
}

fn args_preview(args: Option<&Value>) -> String {
    let raw = match args {
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
        None => String::new(),
    };
    truncate_chars(raw.replace('\n', " "), 120)
}

fn latest_assistant_text(event: &Value) -> Option<String> {
    event
        .get("messages")
        .and_then(Value::as_array)?
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .and_then(|message| message.get("content"))
        .map(message_content_text)
}

fn message_content_text(content: &Value) -> String {
    match content {
        Value::String(value) => value.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn tool_name(reason: &Value) -> String {
    reason
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_string()
}

fn deny_reason(reason: &Value) -> String {
    reason
        .get("rule")
        .and_then(Value::as_str)
        .or_else(|| reason.get("command").and_then(Value::as_str))
        .unwrap_or("approval denied")
        .to_string()
}

fn truncate_chars(value: String, limit: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= limit {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::config::DaemonConfig;
    use crate::daemon::RuntimePaths;
    use serial_test::serial;
    use tempfile::{tempdir, TempDir};

    struct BufferIo {
        stdout: String,
        stderr: String,
        tty: bool,
        line: Option<String>,
    }

    impl BufferIo {
        fn new() -> Self {
            Self {
                stdout: String::new(),
                stderr: String::new(),
                tty: false,
                line: None,
            }
        }
    }

    impl RunIo for BufferIo {
        fn write_stdout(&mut self, text: &str) {
            self.stdout.push_str(text);
        }

        fn write_stderr(&mut self, text: &str) {
            self.stderr.push_str(text);
        }

        fn flush_stdout(&mut self) {}

        fn stdin_is_tty(&self) -> bool {
            self.tty
        }

        fn read_stdin_line(&mut self) -> Option<String> {
            self.line.take()
        }
    }

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(script: &str) -> Option<Self> {
            let python = std::env::var("PYTHON3").unwrap_or_else(|_| "python3".to_string());
            if std::process::Command::new(&python)
                .arg("--version")
                .output()
                .is_err()
            {
                return None;
            }
            let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fake_worker.py");
            let keys = vec![
                (
                    "REFACT_DAEMON_WORKER_CMD",
                    std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
                ),
                (
                    "REFACT_DAEMON_SUPERVISOR_BACKOFF_MS",
                    std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS").ok(),
                ),
                ("FAKE_WORKER_CRASH", std::env::var("FAKE_WORKER_CRASH").ok()),
                (
                    "FAKE_WORKER_CHAT_SCRIPT",
                    std::env::var("FAKE_WORKER_CHAT_SCRIPT").ok(),
                ),
            ];
            std::env::set_var(
                "REFACT_DAEMON_WORKER_CMD",
                shell_words::join([python.as_str(), worker.to_string_lossy().as_ref()]),
            );
            std::env::set_var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", "1");
            std::env::set_var("FAKE_WORKER_CHAT_SCRIPT", script);
            std::env::remove_var("FAKE_WORKER_CRASH");
            Some(Self { keys })
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.keys.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    async fn start_daemon(
        dir: &TempDir,
    ) -> (DaemonInfo, tokio::task::JoinHandle<i32>, RuntimePaths) {
        let paths = RuntimePaths::in_dir(dir.path().join("daemon").as_path());
        let config = DaemonConfig {
            bind: "127.0.0.1".to_string(),
            port: 0,
            ..DaemonConfig::default()
        };
        let task_paths = paths.clone();
        let task = tokio::spawn(async move {
            crate::daemon::run_daemon_entry_with_paths(config, task_paths, false, false).await
        });
        let info = wait_for_info(&paths.daemon_json_path).await;
        (info, task, paths)
    }

    async fn wait_for_info(path: &std::path::Path) -> DaemonInfo {
        for _ in 0..100 {
            if let Some(info) = crate::daemon::state::read_daemon_info(path).await.unwrap() {
                return info;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("daemon info not written");
    }

    async fn stop_daemon(info: &DaemonInfo, task: tokio::task::JoinHandle<i32>) {
        let client = reqwest::Client::new();
        let _ = client
            .post(format!("http://127.0.0.1:{}/daemon/v1/shutdown", info.port))
            .json(&json!({"reason": "test"}))
            .send()
            .await;
        let _ = task.await;
    }

    fn options(project: PathBuf, approve: ApprovalPolicy, json: bool) -> RunOptions {
        RunOptions {
            project: Some(project),
            mode: RunMode::Agent,
            model: Some("test/model".to_string()),
            approve,
            json,
            timeout_secs: 10,
            prompt: "say hi".to_string(),
            listen_ctrl_c: false,
        }
    }

    fn options_with_timeout(
        project: PathBuf,
        approve: ApprovalPolicy,
        json: bool,
        timeout_secs: u64,
    ) -> RunOptions {
        RunOptions {
            timeout_secs,
            ..options(project, approve, json)
        }
    }

    #[test]
    fn parse_run_args_full_matrix() {
        let opts = parse_run_args(&[
            OsString::from("--project"),
            OsString::from("/tmp/project"),
            OsString::from("--mode"),
            OsString::from("explore"),
            OsString::from("--model"),
            OsString::from("m"),
            OsString::from("--approve"),
            OsString::from("auto"),
            OsString::from("--json"),
            OsString::from("--timeout-secs"),
            OsString::from("12"),
            OsString::from("do"),
            OsString::from("x"),
        ])
        .unwrap();
        assert_eq!(opts.project, Some(PathBuf::from("/tmp/project")));
        assert_eq!(opts.mode, RunMode::Explore);
        assert_eq!(opts.model.as_deref(), Some("m"));
        assert_eq!(opts.approve, ApprovalPolicy::Auto);
        assert!(opts.json);
        assert_eq!(opts.timeout_secs, 12);
        assert_eq!(opts.prompt, "do x");
    }

    #[test]
    fn parse_run_args_defaults_to_deny_agent() {
        let opts = parse_run_args(&[OsString::from("hello")]).unwrap();
        assert_eq!(opts.mode, RunMode::Agent);
        assert_eq!(opts.approve, ApprovalPolicy::Deny);
        assert_eq!(opts.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert_eq!(opts.prompt, "hello");
    }

    #[test]
    fn parse_run_args_rejects_invalid_mode() {
        let err = parse_run_args(&[
            OsString::from("--mode"),
            OsString::from("debug"),
            OsString::from("hello"),
        ])
        .unwrap_err();
        assert!(err.contains("--mode"));
    }

    #[test]
    fn parse_run_args_requires_prompt() {
        let err = parse_run_args(&[OsString::from("--json")]).unwrap_err();
        assert!(err.contains("prompt"));
    }

    #[test]
    fn run_error_kind_json_names_are_stable() {
        assert_eq!(RunErrorKind::Unreachable.as_str(), "unreachable");
        assert_eq!(RunErrorKind::ProjectOpen.as_str(), "project_open");
        assert_eq!(RunErrorKind::Chat.as_str(), "chat");
        assert_eq!(RunErrorKind::ApprovalDenied.as_str(), "approval_denied");
        assert_eq!(RunErrorKind::Timeout.as_str(), "timeout");
        assert_eq!(RunErrorKind::Interrupted.as_str(), "interrupted");
    }

    #[test]
    fn exit_code_mapping_table() {
        assert_eq!(exit_code_for(RunErrorKind::Unreachable), 1);
        assert_eq!(exit_code_for(RunErrorKind::Chat), 2);
        assert_eq!(exit_code_for(RunErrorKind::ApprovalDenied), 3);
        assert_eq!(exit_code_for(RunErrorKind::Timeout), 4);
        assert_eq!(exit_code_for(RunErrorKind::Interrupted), 130);
    }

    #[test]
    fn run_json_failures_emit_parseable_contract() {
        for kind in [
            RunErrorKind::Unreachable,
            RunErrorKind::ProjectOpen,
            RunErrorKind::Chat,
            RunErrorKind::ApprovalDenied,
            RunErrorKind::Timeout,
            RunErrorKind::Interrupted,
        ] {
            let mut io = BufferIo::new();
            let exit_code = finish_result(
                Err(RunFailure {
                    kind,
                    message: format!("{} failure", kind.as_str()),
                }),
                true,
                &mut io,
            );
            assert_eq!(exit_code, exit_code_for(kind));
            assert!(io.stderr.is_empty());
            let failure: RunJsonFailure = serde_json::from_str(io.stdout.trim()).unwrap();
            assert!(!failure.ok);
            assert_eq!(failure.kind, kind.as_str());
            assert_eq!(failure.exit_code, exit_code_for(kind));
            assert!(failure.error.contains("failure"));
        }
    }

    #[test]
    fn approval_policy_decisions() {
        assert_eq!(
            approval_decision(ApprovalPolicy::Deny, true, Some("y")),
            ApprovalDecision::Deny
        );
        assert_eq!(
            approval_decision(ApprovalPolicy::Ask, false, Some("y")),
            ApprovalDecision::Deny
        );
        assert_eq!(
            approval_decision(ApprovalPolicy::Ask, true, Some("y")),
            ApprovalDecision::Approve
        );
        assert_eq!(
            approval_decision(ApprovalPolicy::Ask, true, Some("")),
            ApprovalDecision::Deny
        );
        assert_eq!(
            approval_decision(ApprovalPolicy::Auto, false, None),
            ApprovalDecision::Approve
        );
    }

    #[test]
    fn summarize_tool_call_handles_openai_shape() {
        let summary = summarize_tool_call(&json!({
            "function": {"name": "shell", "arguments": "{\"command\":\"echo hi\"}"}
        }));
        assert_eq!(summary.name, "shell");
        assert!(summary.args_preview.contains("echo hi"));
    }

    #[test]
    fn reconnect_snapshot_prefix_prints_only_suffix() {
        let mut state = RunState::new("chat".to_string());
        state.final_text = "hello".to_string();
        let mut io = BufferIo::new();
        let opts = options(PathBuf::from("/tmp/project"), ApprovalPolicy::Deny, false);
        handle_snapshot(
            &mut state,
            &json!({
                "messages": [{"role": "assistant", "content": "hello world"}]
            }),
            &opts,
            &mut io,
        )
        .unwrap();
        assert_eq!(io.stdout, " world");
        assert_eq!(state.final_text, "hello world");
    }

    #[test]
    fn reconnect_snapshot_different_text_prints_separator_and_full_text() {
        let mut state = RunState::new("chat".to_string());
        state.final_text = "hello".to_string();
        let mut io = BufferIo::new();
        let opts = options(PathBuf::from("/tmp/project"), ApprovalPolicy::Deny, false);
        handle_snapshot(
            &mut state,
            &json!({
                "messages": [{"role": "assistant", "content": "goodbye"}]
            }),
            &opts,
            &mut io,
        )
        .unwrap();
        assert_eq!(io.stdout, "\n--- reconnect ---\ngoodbye");
        assert_eq!(state.final_text, "goodbye");
    }

    #[test]
    fn reconnect_snapshot_json_mode_updates_state_without_streaming() {
        let mut state = RunState::new("chat".to_string());
        state.final_text = "hello".to_string();
        let mut io = BufferIo::new();
        let opts = options(PathBuf::from("/tmp/project"), ApprovalPolicy::Deny, true);
        handle_snapshot(
            &mut state,
            &json!({
                "messages": [{"role": "assistant", "content": "hello world"}]
            }),
            &opts,
            &mut io,
        )
        .unwrap();
        assert!(io.stdout.is_empty());
        assert_eq!(state.final_text, "hello world");
    }

    #[tokio::test(start_paused = true)]
    async fn setup_deadline_returns_timeout_for_pending_step() {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let task = tokio::spawn(with_setup_deadline(
            deadline,
            5,
            "subscribe",
            std::future::pending::<Result<(), RunFailure>>(),
        ));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(5)).await;
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.kind, RunErrorKind::Timeout);
        assert!(error.message.contains("setup timed out after 5 seconds"));
        assert!(error.message.contains("subscribe"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn run_streams_answer_against_fake_worker() {
        let Some(_env) = EnvGuard::set("ok") else {
            return;
        };
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let (info, task, _paths) = start_daemon(&dir).await;
        let mut io = BufferIo::new();
        let summary = run_with_daemon_info(
            options(project, ApprovalPolicy::Deny, false),
            info.clone(),
            &mut io,
        )
        .await
        .unwrap();
        assert_eq!(summary.final_text, "hello world");
        assert_eq!(io.stdout, "hello world");
        assert_eq!(summary.tool_calls[0].name, "fake_tool");
        assert!(io.stderr.contains("→ fake_tool"));
        stop_daemon(&info, task).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn run_pause_required_deny_exits_three() {
        let Some(_env) = EnvGuard::set("pause") else {
            return;
        };
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let (info, task, _paths) = start_daemon(&dir).await;
        let mut io = BufferIo::new();
        let result = run_with_daemon_info(
            options(project, ApprovalPolicy::Deny, false),
            info.clone(),
            &mut io,
        )
        .await
        .unwrap_err();
        assert_eq!(exit_code_for(result.kind), 3);
        assert!(io.stderr.contains("denied fake_tool"));
        stop_daemon(&info, task).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn run_pause_required_auto_approves_and_json_summarizes() {
        let Some(_env) = EnvGuard::set("pause") else {
            return;
        };
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let (info, task, _paths) = start_daemon(&dir).await;
        let mut io = BufferIo::new();
        let summary = run_with_daemon_info(
            options(project, ApprovalPolicy::Auto, true),
            info.clone(),
            &mut io,
        )
        .await
        .unwrap();
        assert_eq!(summary.final_text, "approved path");
        assert!(io.stderr.is_empty());
        let emitted: RunJsonSummary = serde_json::from_str(io.stdout.trim()).unwrap();
        assert_eq!(emitted.final_text, "approved path");
        stop_daemon(&info, task).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn run_json_stalled_stream_times_out_with_failure_contract() {
        let Some(_env) = EnvGuard::set("stall") else {
            return;
        };
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let (info, task, _paths) = start_daemon(&dir).await;
        let mut io = BufferIo::new();
        let code = finish_result(
            run_with_daemon_info(
                options_with_timeout(project, ApprovalPolicy::Deny, true, 3),
                info.clone(),
                &mut io,
            )
            .await,
            true,
            &mut io,
        );

        assert_eq!(code, exit_code_for(RunErrorKind::Timeout));
        assert!(io.stderr.is_empty());
        let failure: RunJsonFailure = serde_json::from_str(io.stdout.trim()).unwrap();
        assert!(!failure.ok);
        assert_eq!(failure.kind, "timeout");
        assert_eq!(failure.exit_code, exit_code_for(RunErrorKind::Timeout));
        stop_daemon(&info, task).await;
    }
}
