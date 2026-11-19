use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use futures::FutureExt;
use refact_chat_api::{
    PythonCallApproval, PythonCallRecord, PythonCallStatus, PythonEngineChange,
    PythonEngineOutcome, PythonEngineRequest,
};
use refact_python_kernel::protocol::HostResult;
use serde_json::{json, Value};

use super::{lookup, suggestions, EngineClass, EngineCtx, EngineError, EngineFn, Preview};
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{get_project_dir_string, run_control_hooks, ControlVerdict, HookPayload};
use crate::python_tool::approvals::{ApprovalAsk, ApprovalOutcome, ENGINE_CHANGE_APPROVAL};
use crate::python_tool::dispatcher::{NestedDispatcher, NestedOutcome};
use crate::python_tool::envelope::preview as summary_preview;

pub const MAX_LISTED_CHANGES: usize = 50;

/// How long a change waits for the script's other calls to finish before it gives up.
#[cfg(not(test))]
const GATE_WAIT: std::time::Duration = std::time::Duration::from_secs(60);
#[cfg(test)]
const GATE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// One engine change at a time across the engine: preview recheck and apply are serialized.
/// The Settings routes take it too, so a person's save cannot interleave with a script's change.
pub(crate) static APPLY: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

/// Functions whose handler runs long and so runs without `APPLY`; each relies on its target's own
/// guard (and on `take_plan`) against a concurrent change of the same target.
fn runs_without_apply_lock(name: &str) -> bool {
    name == "chats.compact" || (cfg!(test) && name == "testing.unlocked")
}

/// How long a stored run message is awaited before an approved shutdown is dropped.
const SHUTDOWN_STORE_WAIT: std::time::Duration = std::time::Duration::from_secs(600);

/// Per-run `engine()` state: the applied changes and a shutdown asked for by `system.shutdown`.
#[derive(Default)]
pub struct EngineRunState {
    changes: Mutex<(Vec<PythonEngineChange>, usize)>,
    shutdown: std::sync::atomic::AtomicBool,
}

impl Drop for EngineRunState {
    fn drop(&mut self) {
        if self.shutdown.load(std::sync::atomic::Ordering::Acquire) {
            tracing::error!(
                "python engine shutdown approved in a run was dropped: the run ended before its result was stored"
            );
        }
    }
}

impl EngineRunState {
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn request_shutdown(&self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Release);
    }
    /// A run that did not complete never shuts the engine down: the request is dropped and
    /// removed from the applied changes. Returns whether one was dropped.
    pub fn drop_requested_shutdown(&self) -> bool {
        if !self
            .shutdown
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            return false;
        }
        let mut changes = self.changes.lock().unwrap();
        let before = changes.0.len();
        changes.0.retain(|c| c.function != "system.shutdown");
        changes.1 -= before - changes.0.len();
        true
    }
    /// Sends a requested shutdown once the chat holds the run's tool message (`extra.python.run_id`
    /// equal to `run_id`), so the shutdown's trajectory flush saves it.
    pub fn send_requested_shutdown_when_stored(
        &self,
        app: &crate::app_state::AppState,
        chat_id: &str,
        run_id: &str,
    ) {
        if !self
            .shutdown
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let (app, chat_id, run_id) = (app.clone(), chat_id.to_string(), run_id.to_string());
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + SHUTDOWN_STORE_WAIT;
            loop {
                let stored = match app.chat.sessions.read().await.get(&chat_id).cloned() {
                    Some(session) => session.lock().await.messages.iter().any(|m| {
                        m.extra
                            .get("python")
                            .and_then(|p| p.get("run_id"))
                            .and_then(Value::as_str)
                            == Some(run_id.as_str())
                    }),
                    None => false,
                };
                if stored {
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    tracing::error!(%chat_id, %run_id, "python engine shutdown dropped: the run's result was not stored within {} s", SHUTDOWN_STORE_WAIT.as_secs());
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            let sent = app
                .gcx
                .ask_shutdown_sender
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .send("python engine function system.shutdown".to_string());
            if sent.is_err() {
                tracing::error!("python engine shutdown request could not be delivered");
            }
        });
    }
    fn record(&self, change: PythonEngineChange) {
        let mut changes = self.changes.lock().unwrap();
        changes.1 += 1;
        changes.0.push(change);
        changes.0.sort_by_key(|c| c.seq);
        changes.0.truncate(MAX_LISTED_CHANGES);
    }
    /// Adds `engine_changes` and `engine_changes_total` to `extra.python` when a change was applied.
    /// Stored values are cut to `STORED_VALUE_MAX_BYTES` each and the list to
    /// `STORED_CHANGES_MAX_BYTES`, keeping the newest; the card and the run's call file have the
    /// full values, and `engine_changes_total` stays the true count.
    pub fn extend_extra(&self, extra: &mut Value) {
        let changes = self.changes.lock().unwrap();
        if changes.1 > 0 {
            let mut stored: Vec<Value> = changes
                .0
                .iter()
                .map(|change| {
                    let mut change = change.clone();
                    for item in &mut change.items {
                        for side in [&mut item.old, &mut item.new] {
                            if let Some(value) = side.as_mut() {
                                if let Some(cut) = cut_stored(value) {
                                    *value = cut;
                                    item.truncated = true;
                                }
                            }
                        }
                    }
                    json!(change)
                })
                .collect();
            let mut size: usize = stored
                .iter()
                .map(|c| serde_json::to_vec(c).map_or(0, |b| b.len()))
                .sum();
            while size > STORED_CHANGES_MAX_BYTES && !stored.is_empty() {
                size -= serde_json::to_vec(&stored.remove(0)).map_or(0, |b| b.len());
            }
            extra["engine_changes"] = Value::Array(stored);
            extra["engine_changes_total"] = json!(changes.1);
        }
    }
}

const STORED_VALUE_MAX_BYTES: usize = 2 * 1024;
const STORED_CHANGES_MAX_BYTES: usize = 256 * 1024;

fn cut_stored(value: &Value) -> Option<Value> {
    let text = match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    if serde_json::to_vec(value).map_or(0, |b| b.len()) <= STORED_VALUE_MAX_BYTES {
        return None;
    }
    let mut end = STORED_VALUE_MAX_BYTES.min(text.len()) - 16;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(Value::String(format!("{}…", &text[..end])))
}

type Raise = (&'static str, String, bool);

fn raise(error: EngineError) -> Result<Value, Raise> {
    match error {
        EngineError::Type(message) => Err(("TypeError", message, false)),
        EngineError::Permission(message) => Err(("PermissionError", message, false)),
        EngineError::Failed(message) => Ok(failed_envelope(&message)),
    }
}

pub(crate) fn failed_envelope(message: &str) -> Value {
    let message = super::mask::scrub_text(message);
    json!({"ok":false,"data":null,"text":"","files":[],"diffs":[],"images":[],"images_dropped":0,"error":message,"truncated":false})
}

/// An envelope the script can receive whole. Over the cap a read returns ok:false saying why;
/// an applied change keeps ok:true with `data` null and `truncated` true.
pub(crate) fn fit_for_script(
    name: &str,
    envelope: Value,
    max_bytes: usize,
    applied: bool,
) -> Value {
    use refact_python_kernel::convert::{fits_result, json_size, result_nodes_for, too_large_message};
    let size = json_size(&envelope);
    let max_bytes = max_bytes as u64;
    let max_nodes = result_nodes_for(max_bytes);
    if fits_result(&size, max_nodes, max_bytes) {
        return envelope;
    }
    let message = format!(
        "{}; the limit is set by python_max_nested_result_bytes",
        too_large_message(
            &format!("{name} returned a result"),
            &size,
            max_nodes,
            max_bytes,
        )
    );
    if applied {
        let text = summary_preview(&format!("the change was applied; {message}"));
        return json!({"ok":true,"data":null,"text":text,"files":[],"diffs":[],"images":[],"images_dropped":0,"error":null,"truncated":true});
    }
    failed_envelope(&message)
}

fn validate(schema: &Value, args: &Value) -> Result<(), Raise> {
    refact_tool_api::schema_validate::validate(schema, args).map_err(|errors| {
        (
            "TypeError",
            errors
                .iter()
                .map(|e| {
                    if e.path.is_empty() {
                        e.message.clone()
                    } else {
                        format!("{}: {}", e.path, e.message)
                    }
                })
                .collect::<Vec<_>>()
                .join("; "),
            false,
        )
    })
}

fn switch_refusal(
    settings: &refact_work_core::CoordinationSettings,
    name: &str,
    class: EngineClass,
) -> Option<String> {
    match class {
        EngineClass::Read | EngineClass::RedactedRead => None,
        EngineClass::Change if !settings.python_engine_change_enabled => Some(format!(
            "engine changes are switched off (python_engine_change_enabled=false); {name} was not run"
        )),
        EngineClass::Risky | EngineClass::Destructive if !settings.python_engine_risky_enabled => Some(format!(
            "risky and destructive engine changes are switched off (python_engine_risky_enabled=false); {name} was not run"
        )),
        _ => None,
    }
}

/// A chat that runs commands without asking could approve its own change.
pub(crate) fn auto_commands_refusal(
    settings: &refact_work_core::CoordinationSettings,
    thread: &refact_chat_api::ThreadParams,
    name: &str,
) -> Option<String> {
    (thread.auto_approve_dangerous_commands && !settings.python_engine_change_with_auto_commands).then(|| format!(
        "engine changes are refused in a chat that runs commands without asking (auto_approve_dangerous_commands), because a command of this chat could approve the change itself; turn that off for this chat, or set python_engine_change_with_auto_commands=true in Settings; {name} was not run"
    ))
}

async fn current_thread(d: &NestedDispatcher) -> refact_chat_api::ThreadParams {
    let c = &d.config;
    match c.app.chat.sessions.read().await.get(&c.chat_id).cloned() {
        Some(session) => session.lock().await.thread.clone(),
        None => c.thread.clone(),
    }
}

/// Why no person can answer a change in this chat, or `None` for a root chat a person watches.
pub(crate) async fn unattended_reason(d: &NestedDispatcher) -> Option<&'static str> {
    let c = &d.config;
    let (agent, task_agent) = {
        let context = c.ccx.lock().await;
        (
            context.background_agent_id.is_some(),
            context
                .task_meta
                .as_ref()
                .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents")),
        )
    };
    let thread = current_thread(d).await;
    let agent = agent
        || c.app
            .agents
            .find_agent_id_by_child_chat_id(&c.chat_id)
            .await
            .is_some();
    unattended_kind(agent, task_agent, &thread, &c.chat_id)
}

/// Child, task-agent, scheduled or automation chats and `autonomous_no_confirm` chats have no person to ask.
pub(crate) fn unattended_kind(
    background_agent: bool,
    task_agent_context: bool,
    thread: &refact_chat_api::ThreadParams,
    chat_id: &str,
) -> Option<&'static str> {
    if background_agent {
        return Some("a background agent chat");
    }
    if task_agent_context
        || thread
            .task_meta
            .as_ref()
            .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
    {
        return Some("a task-agent chat");
    }
    if matches!(
        thread.link_type.as_deref(),
        Some("subagent" | "delegate" | "task_agent")
    ) {
        return Some("a child chat");
    }
    if chat_id.starts_with("cron_")
        || chat_id.starts_with("subchat-")
        || thread
            .buddy_meta
            .as_ref()
            .is_some_and(|meta| meta.workflow_id.is_some())
    {
        return Some("a scheduled or automation chat");
    }
    if thread.autonomous_no_confirm {
        return Some("a chat running with autonomous_no_confirm");
    }
    None
}

/// Why this run may not raise engine change cards, from the origin its chat latched when the
/// current turn started (A2); never from the transcript.
async fn latched_origin_refusal(d: &NestedDispatcher) -> Option<String> {
    let c = &d.config;
    let session = c.app.chat.sessions.read().await.get(&c.chat_id).cloned();
    let (origin, started) = match session {
        Some(session) => {
            let s = session.lock().await;
            (
                s.turn_origin.clone(),
                s.runtime
                    .python_runs
                    .iter()
                    .find(|run| run.run_id == c.run_id)
                    .map(|run| run.started_at_ms),
            )
        }
        None => (None, None),
    };
    refact_chat_history::history_gates::engine_change_origin_refusal(origin.as_ref(), started)
}

/// Masked copy of call arguments for every record, file, hook payload and approval request.
pub(crate) fn masked(args: &Value) -> Value {
    let mut copy = args.clone();
    super::mask::scrub(&mut copy);
    copy
}

/// A hook saw masked arguments: wherever its rewrite still holds the masked value we sent it,
/// it did not change that field, so the real value goes back instead of the mask.
pub(crate) fn unmask_rewrite(rewritten: &mut Value, sent: &Value, original: &Value) {
    match (rewritten, sent, original) {
        (Value::Object(new), Value::Object(sent), Value::Object(original)) => {
            for (key, value) in new.iter_mut() {
                if let (Some(sent), Some(original)) = (sent.get(key), original.get(key)) {
                    unmask_rewrite(value, sent, original);
                }
            }
        }
        (Value::Array(new), Value::Array(sent), Value::Array(original))
            if sent.len() == original.len() =>
        {
            for (i, value) in new.iter_mut().enumerate() {
                if let (Some(sent), Some(original)) = (sent.get(i), original.get(i)) {
                    unmask_rewrite(value, sent, original);
                }
            }
        }
        (value, sent, original) if *value == *sent && sent != original => {
            *value = original.clone();
        }
        _ => {}
    }
}

/// Privacy records a preview added to the run (P1): taken out of the run's taint so that they
/// travel once, with this call's records, whatever happens to the call afterwards.
fn taint_since(
    run: &std::sync::Mutex<crate::python_tool::llm::RunPrivacy>,
    start: usize,
) -> Vec<Value> {
    let mut run = run.lock().unwrap_or_else(|e| e.into_inner());
    if run.taint.len() > start {
        run.taint.split_off(start)
    } else {
        Vec::new()
    }
}

/// Functions whose approval card shows message text of another chat (P2): the call summary, the
/// call file and the stored change items keep their labels, counts and flags, never that text.
fn card_shows_other_chat_text(name: &str) -> bool {
    matches!(
        name,
        "chats.patch"
            | "chats.edit_message"
            | "chats.remove_message"
            | "chats.retry_from"
            | "chats.cancel_queued"
    )
}

/// The arguments the call file keeps (P2): a `chats.patch` `find` fragment is text of the other
/// chat, so only its size is stored.
fn stored_kwargs(name: &str, args: &Value) -> Value {
    let mut kwargs = masked(args);
    if name == "chats.patch" {
        for op in kwargs["ops"].as_array_mut().into_iter().flatten() {
            if let Some(bytes) = op.get("find").and_then(Value::as_str).map(str::len) {
                op["find"] = json!(format!("({bytes} bytes, shown on the approval card only)"));
            }
        }
    }
    kwargs
}

fn without_card_text(
    items: &[refact_chat_api::PythonEngineItem],
) -> Vec<refact_chat_api::PythonEngineItem> {
    let keep = |value: &Option<Value>| {
        value.as_ref().map(|value| match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
            other => json!(format!(
                "({} bytes, shown on the approval card only)",
                serde_json::to_vec(other).map_or(0, |bytes| bytes.len())
            )),
        })
    };
    items
        .iter()
        .map(|item| refact_chat_api::PythonEngineItem {
            label: item.label.clone(),
            old: keep(&item.old),
            new: keep(&item.new),
            truncated: false,
            label_is_name: item.label_is_name,
        })
        .collect()
}

/// How a change ended after its handler started.
enum Applied {
    No,
    Yes,
    Unknown,
}

impl NestedDispatcher {
    /// `await engine("area.function", **kwargs)`: the C15 pipeline.
    pub async fn dispatch_engine(
        &self,
        seq: u32,
        kwargs: Value,
        too_large: Option<String>,
        site: (Option<u32>, Option<u32>),
    ) -> NestedOutcome {
        let c = &self.config;
        let start = Instant::now();
        let mut args = kwargs;
        let name = match args
            .as_object_mut()
            .and_then(|a| a.remove(super::FUNCTION_KEY))
        {
            Some(Value::String(name)) => name,
            _ => String::new(),
        };
        let mut record = PythonCallRecord {
            seq,
            call_id: format!("py_{}_{}", c.run_id, seq),
            tool: format!("engine.{name}"),
            args_preview: crate::python_tool::approvals::args_preview(
                &stored_kwargs(&name, &args).to_string(),
                1000,
                200,
            ),
            status: PythonCallStatus::Running,
            summary: String::new(),
            started_at_ms: crate::agents::approval::now_ms(),
            duration_ms: None,
            approval: PythonCallApproval::NotNeeded,
            diffs: Vec::new(),
            line: site.0,
            col: site.1,
            model: None,
            tokens_in: None,
            tokens_out: None,
        };
        c.sink.call_started(&record);
        let mut log = json!({"seq":seq,"call_id":record.call_id,"tool":record.tool,"kwargs":stored_kwargs(&name, &args)});
        let mut privacy = Vec::new();
        let awaiting = std::sync::atomic::AtomicBool::new(false);
        let handler_started = std::sync::atomic::AtomicBool::new(false);
        let outcome = {
            let inner = self.engine_inner(
                &name,
                args,
                too_large.as_deref(),
                &mut record,
                &mut log,
                &mut privacy,
                &awaiting,
                &handler_started,
            );
            tokio::pin!(inner);
            tokio::select! {
                biased;
                result = &mut inner => result,
                _ = c.cancel.cancelled() => {
                    if handler_started.load(std::sync::atomic::Ordering::Acquire) {
                        inner.await
                    } else {
                        Err(("RuntimeError", "python run cancelled".to_string(), true))
                    }
                }
            }
        };
        let stopped_change = matches!(outcome, Err((_, _, true)))
            && record.approval == PythonCallApproval::NotNeeded
            && super::lookup(&name).is_some_and(|f| !f.class.is_read());
        if (awaiting.load(std::sync::atomic::Ordering::Acquire) && outcome.is_err())
            || stopped_change
        {
            record.approval = PythonCallApproval::Cancelled;
        }
        let (result, stop_run) = match outcome {
            Ok(envelope) => {
                record.status = if envelope["ok"] == true {
                    PythonCallStatus::Ok
                } else {
                    PythonCallStatus::Failed
                };
                if record.summary.is_empty() {
                    record.summary = summary_preview(
                        envelope["error"]
                            .as_str()
                            .or(envelope["text"].as_str())
                            .unwrap_or_default(),
                    );
                }
                log["envelope"] = envelope.clone();
                (HostResult::Ok(envelope), false)
            }
            Err((exc_type, message, stopped)) => {
                let message = super::mask::scrub_text(&message);
                record.status = if stopped {
                    PythonCallStatus::Cancelled
                } else if exc_type == "PermissionError" {
                    PythonCallStatus::Denied
                } else {
                    PythonCallStatus::Failed
                };
                record.summary = summary_preview(&message);
                log["envelope"] = json!({"exception":exc_type,"message":message});
                (
                    HostResult::Err {
                        exc_type: exc_type.into(),
                        message,
                    },
                    stopped,
                )
            }
        };
        if !privacy.is_empty() {
            c.run_privacy
                .lock()
                .unwrap()
                .taint
                .extend(privacy.iter().cloned());
        }
        record.duration_ms = Some(start.elapsed().as_millis().min(u64::MAX as u128) as u64);
        log["record"] = json!(record);
        let dir = c.run_dir.join("calls");
        if let Err(error) = async {
            tokio::fs::create_dir_all(&dir).await?;
            tokio::fs::write(
                dir.join(format!("{seq}_engine.json")),
                serde_json::to_vec(&log)?,
            )
            .await
        }
        .await
        {
            tracing::warn!(%error, "cannot persist python engine call");
        }
        c.sink.call_finished(&record);
        NestedOutcome {
            result,
            record,
            privacy,
            images: Vec::new(),
            stop_run,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn engine_inner(
        &self,
        name: &str,
        args: Value,
        too_large: Option<&str>,
        record: &mut PythonCallRecord,
        log: &mut Value,
        privacy: &mut Vec<Value>,
        awaiting: &std::sync::atomic::AtomicBool,
        handler_started: &std::sync::atomic::AtomicBool,
    ) -> Result<Value, Raise> {
        let c = &self.config;
        let state = c.engine.clone().ok_or_else(|| {
            (
                "NameError",
                "name 'engine' is not defined".to_string(),
                false,
            )
        })?;
        if name.is_empty() {
            return Err((
                "TypeError",
                "engine() missing required argument: 'name' (a string such as \"settings.get\")"
                    .into(),
                false,
            ));
        }
        let Some(function) = lookup(name) else {
            let close = suggestions(name);
            let hint = if close.is_empty() {
                "help(\"engine\") lists the areas and functions".to_string()
            } else {
                format!("did you mean {}?", close.join(", "))
            };
            return Err((
                "TypeError",
                format!("unknown engine function '{name}'; {hint}"),
                false,
            ));
        };
        let change = !function.class.is_read();
        let refused = |message: String, record: &mut PythonCallRecord| {
            if change {
                record.approval = PythonCallApproval::Denied;
            }
            Err(("PermissionError", message, false))
        };
        if let Some(size) = too_large {
            return Err((
                "TypeError",
                format!("engine() arguments are too large to send: {size}; pass smaller values"),
                false,
            ));
        }
        let schema = (function.input)();
        let mut args = crate::python_tool::dispatcher::normalize_optional_nulls(&schema, &args);
        validate(&schema, &args)?;
        if !self.charge_nested_call() {
            return Err(("RuntimeError", "nested call budget exhausted".into(), false));
        }
        let mut ctx = self.engine_ctx().await?;
        if let Some(refusal) = switch_refusal(&ctx.settings, name, function.class) {
            return refused(refusal, record);
        }
        if change {
            if let Some(reason) = unattended_reason(self).await {
                return refused(format!("engine changes need a person to approve them and this is {reason}; {name} was not run"), record);
            }
            let thread = current_thread(self).await;
            if let Some(refusal) = auto_commands_refusal(&ctx.settings, &thread, name) {
                return refused(refusal, record);
            }
            if let Some(reason) = latched_origin_refusal(self).await {
                return refused(format!("engine changes run only in a turn a person started; {reason}; {name} was not run"), record);
            }
        }
        let tool = format!("engine.{name}");
        match self.engine_pre_hook(&tool, &args, &record.call_id).await {
            Ok(Some(mut rewritten)) => {
                unmask_rewrite(&mut rewritten, &masked(&args), &args);
                validate(&schema, &rewritten)?;
                args = rewritten;
                record.args_preview = crate::python_tool::approvals::args_preview(
                    &stored_kwargs(name, &args).to_string(),
                    1000,
                    200,
                );
                log["kwargs"] = stored_kwargs(name, &args);
            }
            Ok(None) => {}
            Err((_, message, _)) => return refused(message, record),
        }
        if let Some(rule) = self.engine_deny_rule(&tool).await {
            return refused(rule, record);
        }
        if !change {
            let _call = self
                .enter_call()
                .await
                .ok_or_else(|| ("RuntimeError", "python run cancelled".to_string(), true))?;
            let result = self.engine_execute(function, &ctx, &args).await?;
            return self.finish_read(name, result, record, privacy);
        }
        let _exclusive = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => return Err(("RuntimeError", "python run cancelled".into(), true)),
            guard = tokio::time::timeout(GATE_WAIT, self.gate.clone().write_owned()) => match guard {
                Ok(guard) => guard,
                Err(_) => {
                    record.approval = PythonCallApproval::Denied;
                    return Err((
                        "RuntimeError",
                        format!(
                            "{name}: other calls of this script were still running after {} s; a change runs only when nothing else of the script is in flight, so await the other calls first and call it again; nothing was run",
                            GATE_WAIT.as_secs()
                        ),
                        false,
                    ));
                }
            },
        };
        let taint_before = c.run_privacy.lock().unwrap().taint.len();
        let previewed = self.engine_preview(function, &ctx, &args).await;
        privacy.extend(taint_since(&c.run_privacy, taint_before));
        let mut preview = match previewed {
            Ok(preview) => preview,
            Err((exc, message, stopped)) => {
                record.approval = PythonCallApproval::Denied;
                return Err((exc, message, stopped));
            }
        };
        if preview.plan.is_nothing() {
            record.summary = "nothing to change".into();
            return Ok(
                json!({"ok":true,"data":null,"text":"nothing to change","files":[],"diffs":[],"images":[],"images_dropped":0,"error":null,"truncated":false}),
            );
        }
        if let Some(refusal) = switch_refusal(&ctx.settings, name, preview.class) {
            return refused(refusal, record);
        }
        awaiting.store(true, std::sync::atomic::Ordering::Release);
        let outcome = self.engine_ask(&tool, &args, record, &preview).await;
        awaiting.store(false, std::sync::atomic::Ordering::Release);
        match outcome {
            ApprovalOutcome::Allowed {
                for_chat: false,
            } => {
                record.approval = PythonCallApproval::Allowed;
            }
            ApprovalOutcome::Allowed {
                for_chat: true,
            }
            | ApprovalOutcome::AllowedForRun => {
                record.approval = PythonCallApproval::Denied;
                return Err((
                    "PermissionError",
                    "an engine change is approved only once by a person".into(),
                    false,
                ));
            }
            ApprovalOutcome::Denied => {
                record.approval = PythonCallApproval::Denied;
                return Err(("PermissionError", "rejected by the user".into(), false));
            }
            ApprovalOutcome::Stopped | ApprovalOutcome::Cancelled => {
                record.approval = PythonCallApproval::Cancelled;
                return Err(("RuntimeError", "python run stopped".into(), true));
            }
        }
        let audit = card_shows_other_chat_text(name).then(|| Preview {
            title: preview.title.clone(),
            items: without_card_text(&preview.items),
            scope: preview.scope.clone(),
            class: preview.class,
            warning: preview.warning.clone(),
            plan: super::Plan::default(),
        });
        log["preview"] = json!(request(name, audit.as_ref().unwrap_or(&preview)));
        let _apply = if runs_without_apply_lock(name) {
            None
        } else {
            tokio::select! {
                biased;
                _ = c.cancel.cancelled() => return Err(("RuntimeError", "python run stopped".into(), true)),
                guard = APPLY.lock() => Some(guard),
            }
        };
        if c.cancel.is_cancelled() {
            return Err(("RuntimeError", "python run stopped".into(), true));
        }
        let plan = std::mem::take(&mut preview.plan);
        *ctx.plan.get_mut().unwrap_or_else(|e| e.into_inner()) = Some(plan);
        handler_started.store(true, std::sync::atomic::Ordering::Release);
        let (mut result, mut applied, hook_denied) = self.engine_apply(function, &ctx, &args).await;
        if result.is_ok() && ctx.plan.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            applied = Applied::Unknown;
            result = Err(EngineError::Failed(format!(
                "internal error: {name} did not apply the approved plan; whether it changed anything is unknown"
            )));
        }
        let stopped = result
            .as_ref()
            .ok()
            .and_then(|output| output.stopped.as_ref())
            .map(|stopped| stopped.applied.clone());
        if stopped.as_ref().is_some_and(Vec::is_empty) {
            applied = Applied::No;
        }
        if !matches!(applied, Applied::No) {
            let shown = audit.as_ref().unwrap_or(&preview);
            let part = stopped.map(|items| Preview {
                title: shown.title.clone(),
                items,
                scope: shown.scope.clone(),
                class: shown.class,
                warning: shown.warning.clone(),
                plan: super::Plan::default(),
            });
            self.record_change(
                name,
                record,
                part.as_ref().unwrap_or(shown),
                &applied,
                &state,
            );
        }
        if let Some(reason) = hook_denied {
            let message = format!(
                "the PostToolUse hook rejected the result ({reason}); the change is in place"
            );
            record.summary = summary_preview(&super::mask::scrub_text(&message));
            return Ok(failed_envelope(&message));
        }
        let output = match result {
            Ok(output) => output,
            Err(error) => return raise(error),
        };
        let envelope = self.finish_output(output, record, privacy);
        let ok = envelope["ok"] == true;
        Ok(fit_for_script(
            name,
            envelope,
            c.max_nested_result_bytes,
            ok,
        ))
    }

    fn finish_read(
        &self,
        name: &str,
        result: Result<super::EngineOutput, EngineError>,
        record: &mut PythonCallRecord,
        privacy: &mut Vec<Value>,
    ) -> Result<Value, Raise> {
        let output = match result {
            Ok(output) => output,
            Err(error) => return raise(error),
        };
        let envelope = self.finish_output(output, record, privacy);
        Ok(fit_for_script(
            name,
            envelope,
            self.config.max_nested_result_bytes,
            false,
        ))
    }

    fn finish_output(
        &self,
        mut output: super::EngineOutput,
        record: &mut PythonCallRecord,
        privacy: &mut Vec<Value>,
    ) -> Value {
        super::mask::scrub(&mut output.data);
        let text = super::mask::scrub_text(&output.text);
        let error = output
            .stopped
            .as_ref()
            .map(|stopped| super::mask::scrub_text(&stopped.error));
        if record.summary.is_empty() {
            record.summary = summary_preview(error.as_deref().unwrap_or(&text));
        }
        privacy.extend(output.privacy);
        json!({"ok":error.is_none(),"data":output.data,"text":text,"files":[],"diffs":[],"images":[],"images_dropped":0,"error":error,"truncated":false})
    }

    fn record_change(
        &self,
        name: &str,
        record: &mut PythonCallRecord,
        preview: &Preview,
        applied: &Applied,
        state: &super::EngineRunState,
    ) {
        let c = &self.config;
        let unknown = matches!(applied, Applied::Unknown);
        record.summary = change_summary(preview);
        if unknown {
            record.summary = summary_preview(&format!("outcome unknown: {}", record.summary));
        }
        tracing::info!(
            function = %name,
            target = %preview.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>().join(","),
            scope = %preview.scope.as_deref().unwrap_or(""),
            outcome = if unknown { "unknown" } else { "applied" },
            chat_id = %c.chat_id,
            run_id = %c.run_id,
            "python engine change applied"
        );
        state.record(PythonEngineChange {
            seq: record.seq,
            function: name.into(),
            class: preview
                .class
                .wire()
                .unwrap_or(refact_chat_api::PythonEngineClass::Change),
            title: preview.title.clone(),
            items: preview.items.clone(),
            scope: preview.scope.clone(),
            at_ms: crate::agents::approval::now_ms(),
            outcome: unknown.then_some(PythonEngineOutcome::Unknown),
        });
    }

    async fn engine_ctx(&self) -> Result<EngineCtx, Raise> {
        let c = &self.config;
        let scope_root = c
            .ccx
            .lock()
            .await
            .execution_scope
            .as_ref()
            .map(|scope| scope.effective_root().to_path_buf());
        let project_root = match scope_root {
            Some(root) => Some(root),
            None => crate::files_correction::get_active_project_path(c.app.gcx.clone()).await,
        };
        let settings = match &project_root {
            Some(root) => crate::runtime_settings::coordination_for_project(root)
                .await
                .map_err(|e| ("RuntimeError", e, false))?,
            None => crate::runtime_settings::current().coordination,
        };
        Ok(EngineCtx {
            app: c.app.clone(),
            ccx: c.ccx.clone(),
            chat_id: c.chat_id.clone(),
            run_id: c.run_id.clone(),
            outer_tool_call_id: c.outer_tool_call_id.clone(),
            project_root,
            settings,
            llm: c.llm.clone(),
            run_privacy: c.run_privacy.clone(),
            nested: super::NestedCalls {
                used: self.charged_calls(),
                limit: c.max_nested_calls,
                parallel_limit: c.max_parallel_calls,
            },
            plan: std::sync::Mutex::new(None),
            run: c.engine.clone(),
            completing: std::sync::Mutex::new(None),
        })
    }

    async fn engine_pre_hook(
        &self,
        tool: &str,
        args: &Value,
        call_id: &str,
    ) -> Result<Option<Value>, Raise> {
        match self.pre_tool_use(tool, masked(args), call_id).await {
            ControlVerdict::Allow => Ok(None),
            ControlVerdict::Deny(reason) => Err(("PermissionError", reason, false)),
            ControlVerdict::Rewrite(value) if value.is_object() => Ok(Some(value)),
            ControlVerdict::Rewrite(_) => Err((
                "PermissionError",
                "hook rewrite must be a JSON object of engine function arguments".into(),
                false,
            )),
        }
    }

    /// Mode `tool_confirm` rules matching `tool` (`engine.<name>`, `git`): deny refuses; `auto`
    /// never skips a change's ask.
    pub(crate) async fn engine_deny_rule(&self, tool: &str) -> Option<String> {
        let c = &self.config;
        let rules = crate::tools::tool_python::mode_confirm_rules(
            &c.app,
            &c.mode_id,
            c.model_id.as_deref(),
        )
        .await;
        let action =
            crate::yaml_configs::customization_registry::match_tool_confirm_action(&rules, tool)?;
        (action == "deny").then(|| format!("mode policy: deny ({tool})"))
    }

    async fn engine_preview(
        &self,
        function: &EngineFn,
        ctx: &EngineCtx,
        args: &Value,
    ) -> Result<Preview, Raise> {
        let preview_fn = function.preview.ok_or_else(|| {
            (
                "RuntimeError",
                format!("{} has no change preview", function.name),
                false,
            )
        })?;
        let mut preview = match preview_fn(ctx, args).await {
            Ok(preview) => preview,
            Err(EngineError::Type(m)) => return Err(("TypeError", m, false)),
            Err(EngineError::Permission(m)) => return Err(("PermissionError", m, false)),
            Err(EngineError::Failed(m)) => return Err(("RuntimeError", m, false)),
        };
        if preview.plan.is_nothing() {
            return Ok(preview);
        }
        if preview.items.is_empty() {
            return Err((
                "RuntimeError",
                format!(
                    "internal error: {} planned a change without listing what it changes; nothing was run",
                    function.name
                ),
                false,
            ));
        }
        if let Some(refusal) = preview
            .items
            .iter()
            .find_map(super::mask::new_value_refusal)
        {
            return Err((
                "PermissionError",
                format!("{refusal}; {} was not run", function.name),
                false,
            ));
        }
        preview.class = preview.class.max(function.class);
        if preview.class == EngineClass::Change
            && preview
                .warning
                .as_deref()
                .is_some_and(|w| !w.trim().is_empty())
        {
            preview.class = EngineClass::Risky;
        }
        if matches!(preview.class, EngineClass::Risky | EngineClass::Destructive)
            && preview
                .warning
                .as_deref()
                .is_none_or(|w| w.trim().is_empty())
        {
            return Err((
                "RuntimeError",
                format!(
                    "{} produced a {} change without a warning",
                    function.name,
                    preview.class.as_str()
                ),
                false,
            ));
        }
        if let Some(warning) = &preview.warning {
            preview.warning = Some(super::mask::scrub_text(warning));
        }
        Ok(preview)
    }

    /// Forced approval: always a card, never an auto-approve flag, rule or permit.
    async fn engine_ask(
        &self,
        tool: &str,
        args: &Value,
        record: &PythonCallRecord,
        preview: &Preview,
    ) -> ApprovalOutcome {
        let c = &self.config;
        let call: crate::call_validation::ChatToolCall = match serde_json::from_value(
            json!({"id":record.call_id,"type":"function","function":{"name":tool,"arguments":masked(args).to_string()}}),
        ) {
            Ok(call) => call,
            Err(_) => return ApprovalOutcome::Denied,
        };
        let name = tool.trim_start_matches("engine.");
        c.app
            .python_tool
            .approvals
            .request_with_context(
                &c.app,
                ApprovalAsk {
                    chat_id: c.chat_id.clone(),
                    tool_call_id: c.outer_tool_call_id.clone(),
                    run_id: c.run_id.clone(),
                    call_id: call.id.clone(),
                    tool_name: tool.into(),
                    args_preview: call.function.arguments.clone(),
                    command: None,
                    rule: ENGINE_CHANGE_APPROVAL.into(),
                    editing_tool: false,
                    engine: Some(request(name, preview)),
                },
                &c.ccx,
                &call,
            )
            .await
    }

    /// A read: under the nested-call semaphore, ended by Stop, then PostToolUse.
    async fn engine_execute(
        &self,
        function: &EngineFn,
        ctx: &EngineCtx,
        args: &Value,
    ) -> Result<Result<super::EngineOutput, EngineError>, Raise> {
        let c = &self.config;
        let _permit = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => return Err(("RuntimeError", "python run cancelled".into(), true)),
            permit = self.parallel.acquire() => permit.map_err(|_| ("RuntimeError", "dispatcher closed".to_string(), true))?,
        };
        let (result, _) = run_handler(function, ctx, args).await;
        if let Some(reason) = self.engine_post_hook(function, args, &result).await {
            return Ok(Err(EngineError::Failed(format!(
                "Blocked by hook: {reason}"
            ))));
        }
        Ok(result)
    }

    /// A change after approval: runs to its end or its own timeout whatever Stop does; returns the
    /// handler result, whether the change is in place, and a PostToolUse denial.
    async fn engine_apply(
        &self,
        function: &EngineFn,
        ctx: &EngineCtx,
        args: &Value,
    ) -> (
        Result<super::EngineOutput, EngineError>,
        Applied,
        Option<String>,
    ) {
        let (result, unknown) = run_handler(function, ctx, args).await;
        let applied = match (&result, unknown) {
            (_, true) => Applied::Unknown,
            (Ok(_), false) => Applied::Yes,
            (Err(_), false) => Applied::No,
        };
        let denied = self.engine_post_hook(function, args, &result).await;
        (result, applied, denied)
    }

    async fn engine_post_hook(
        &self,
        function: &EngineFn,
        args: &Value,
        result: &Result<super::EngineOutput, EngineError>,
    ) -> Option<String> {
        let c = &self.config;
        let output_text = match result {
            Ok(output) => {
                let mut data = output.data.clone();
                super::mask::scrub(&mut data);
                data.to_string()
            }
            Err(error) => format!("{error:?}"),
        };
        let payload = HookPayload::new(
            &HookEvent::PostToolUse,
            c.chat_id.clone(),
            get_project_dir_string(c.app.clone()).await,
        )
        .with_tool(&format!("engine.{}", function.name), Some(masked(args)))
        .with_tool_output(crate::python_tool::render::cut(&output_text, 64 * 1024));
        match run_control_hooks(c.app.clone(), HookEvent::PostToolUse, payload).await {
            ControlVerdict::Deny(reason) => Some(reason),
            _ => None,
        }
    }
}

/// Runs a handler with its timeout; `true` when it timed out or panicked, so its effect is unknown.
async fn run_handler(
    function: &EngineFn,
    ctx: &EngineCtx,
    args: &Value,
) -> (Result<super::EngineOutput, EngineError>, bool) {
    let timeout = function.timeout.unwrap_or(super::DEFAULT_TIMEOUT);
    let handler = std::panic::AssertUnwindSafe((function.handler)(ctx, args)).catch_unwind();
    match tokio::time::timeout(timeout, handler).await {
        Ok(Ok(result)) => (result, false),
        Ok(Err(_)) => (
            Err(EngineError::Failed(format!(
                "{} failed unexpectedly; whether it changed anything is unknown",
                function.name
            ))),
            true,
        ),
        Err(_) => {
            let completing = ctx
                .completing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            (
                Err(EngineError::Failed(format!(
                    "{} timed out after {} s{}",
                    function.name,
                    timeout.as_secs_f64(),
                    completing
                        .map(|note| format!("; {note}"))
                        .unwrap_or_default()
                ))),
                true,
            )
        }
    }
}

pub(crate) fn request(name: &str, preview: &Preview) -> PythonEngineRequest {
    PythonEngineRequest {
        function: name.into(),
        class: preview
            .class
            .wire()
            .unwrap_or(refact_chat_api::PythonEngineClass::Change),
        title: preview.title.clone(),
        items: preview.items.clone(),
        scope: preview.scope.clone(),
        warning: preview.warning.clone(),
    }
}

/// `label: old → new` per item, joined.
pub(crate) fn change_summary(preview: &Preview) -> String {
    let show = |value: &Option<Value>, absent: &str| match value {
        None => absent.to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    };
    let parts: Vec<String> = preview
        .items
        .iter()
        .map(|item| {
            format!(
                "{}: {} → {}",
                item.label,
                show(&item.old, "(not set)"),
                show(&item.new, "(removed)")
            )
        })
        .collect();
    let joined = if parts.is_empty() {
        preview.title.clone()
    } else {
        parts.join("; ")
    };
    summary_preview(&joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(data: Value) -> Value {
        json!({"ok":true,"data":data,"text":"t","files":[],"diffs":[],"images":[],"images_dropped":0,"error":null,"truncated":false})
    }

    #[test]
    fn stored_engine_changes_are_bounded_and_keep_the_newest_and_the_true_total() {
        let state = EngineRunState::new();
        let big = "v".repeat(16 * 1024);
        for seq in 0..50u32 {
            state.record(PythonEngineChange {
                seq,
                function: "settings.set".into(),
                class: refact_chat_api::PythonEngineClass::Change,
                title: "change engine settings".into(),
                items: (0..5)
                    .map(|i| {
                        super::super::mask::item(
                            &format!("k{i}"),
                            Some(json!(big)),
                            Some(json!({"v": big})),
                        )
                    })
                    .chain([super::super::mask::item(
                        "small",
                        Some(json!(1)),
                        Some(json!(2)),
                    )])
                    .collect(),
                scope: None,
                at_ms: 1,
                outcome: None,
            });
        }
        let mut extra = json!({});
        state.extend_extra(&mut extra);
        let stored = extra["engine_changes"].as_array().unwrap();
        assert!(serde_json::to_vec(stored).unwrap().len() <= 256 * 1024);
        assert!(!stored.is_empty() && stored.len() < 50);
        assert_eq!(stored.last().unwrap()["seq"], 49);
        assert_eq!(extra["engine_changes_total"], 50);
        let items = stored[0]["items"].as_array().unwrap();
        for item in &items[..5] {
            assert_eq!(item["truncated"], true, "{item}");
            for side in ["old", "new"] {
                assert!(
                    serde_json::to_vec(&item[side]).unwrap().len() <= 2 * 1024 + 8,
                    "{item}"
                );
            }
        }
        assert_eq!(items[5]["new"], 2);
        assert!(items[5].get("truncated").is_none());
    }

    #[test]
    fn engine_result_of_20000_nodes_reaches_the_script() {
        let items: Vec<Value> = (0..10_000).map(|i| json!({"i": i})).collect();
        let fitted = fit_for_script(
            "testing.many",
            envelope(json!({"items": items})),
            16 * 1024 * 1024,
            false,
        );
        assert_eq!(fitted["ok"], true, "{}", fitted["error"]);
        assert_eq!(fitted["data"]["items"].as_array().unwrap().len(), 10_000);
    }

    #[test]
    fn engine_result_over_the_cap_is_a_clear_failure() {
        let fitted = fit_for_script(
            "testing.big",
            envelope(json!("x".repeat(70_000))),
            65_536,
            false,
        );
        assert_eq!(fitted["ok"], false);
        assert_eq!(fitted["data"], Value::Null);
        let error = fitted["error"].as_str().unwrap();
        assert!(
            error.contains("testing.big") && error.contains("65536"),
            "{error}"
        );
        assert!(error.contains("narrow"), "{error}");
        assert!(error.contains("python_max_nested_result_bytes"), "{error}");
        let mut deep = json!(0);
        for _ in 0..40 {
            deep = json!([deep]);
        }
        let fitted = fit_for_script("testing.deep", envelope(deep), 65_536, false);
        assert_eq!(fitted["ok"], false);
        assert!(
            fitted["error"].as_str().unwrap().contains("levels deep"),
            "{fitted}"
        );
    }
}
