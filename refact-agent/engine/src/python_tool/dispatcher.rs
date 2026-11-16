use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Instant;

use refact_chat_api::{PythonCallApproval, PythonCallRecord, PythonCallStatus, ThreadParams};
use refact_python_catalog::PythonCatalog;
use refact_python_kernel::protocol::HostResult;
use refact_runtime_api::{ToolCatalogSnapshot, TurnToolPool};
use serde_json::{json, Value};
use tokio::sync::{Mutex as AMutex, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatToolCall, MultimodalElement};
use crate::chat::tools::{
    check_tools_confirmation_reporting_auto, execute_tools, is_editing_tool, ExecuteToolsOptions,
};
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{
    get_project_dir_string, revalidate_rewritten_tool_call, run_control_hooks, ControlVerdict,
    HookPayload,
};
use super::approvals::{ApprovalAsk, ApprovalOutcome};
use super::envelope::{build_envelope, preview};

pub trait CallSink: Send + Sync {
    fn call_started(&self, record: &PythonCallRecord);
    fn call_finished(&self, record: &PythonCallRecord);
}

/// Supply the outer tool's trusted context and the session's thread/catalog snapshot.
/// Approvals use `ccx`; execution freezes its call-specific approval into an abort-isolated context.
pub struct NestedDispatchConfig {
    pub app: AppState,
    pub ccx: Arc<AMutex<AtCommandsContext>>,
    pub chat_id: String,
    pub outer_tool_call_id: String,
    pub run_id: String,
    pub catalog: Arc<PythonCatalog>,
    pub thread: ThreadParams,
    pub mode_id: String,
    pub model_id: Option<String>,
    pub allowed_tools: Vec<String>,
    pub tool_catalog: Option<Arc<ToolCatalogSnapshot>>,
    pub turn_tool_pool: Option<TurnToolPool>,
    pub max_nested_calls: usize,
    pub max_parallel_calls: usize,
    pub max_nested_result_bytes: usize,
    pub run_dir: PathBuf,
    pub cancel: CancellationToken,
    pub sink: Arc<dyn CallSink>,
    /// `llm()` budget of this run; `None` when the helper is disabled.
    pub llm: Option<Arc<super::llm::LlmBudget>>,
    /// Session taint, nested results' and direct reads' privacy collected so far in this run.
    pub run_privacy: Arc<std::sync::Mutex<super::llm::RunPrivacy>>,
    /// `engine()` state of this run; `None` when engine reads are switched off.
    pub engine: Option<Arc<super::engine_fns::EngineRunState>>,
}

pub struct NestedDispatcher {
    pub(crate) config: NestedDispatchConfig,
    charged: AtomicUsize,
    images: AtomicUsize,
    diff_bytes: AtomicUsize,
    pub(crate) parallel: Semaphore,
    /// Every nested call of the run holds a read guard while it runs; an engine change holds
    /// the write guard from before its preview until its handler ended (C15, "nothing in flight").
    pub(crate) gate: Arc<tokio::sync::RwLock<()>>,
}

pub struct NestedOutcome {
    pub result: HostResult,
    pub record: PythonCallRecord,
    pub privacy: Vec<Value>,
    pub images: Vec<MultimodalElement>,
    pub stop_run: bool,
}

impl NestedDispatcher {
    pub fn new(config: NestedDispatchConfig) -> Self {
        Self {
            parallel: Semaphore::new(config.max_parallel_calls.max(1)),
            config,
            charged: AtomicUsize::new(0),
            images: AtomicUsize::new(0),
            diff_bytes: AtomicUsize::new(0),
            gate: Arc::new(tokio::sync::RwLock::new(())),
        }
    }

    /// Waits while an engine change of this run is pending; `None` when the run was cancelled first.
    pub(crate) async fn enter_call(&self) -> Option<tokio::sync::OwnedRwLockReadGuard<()>> {
        tokio::select! {
            biased;
            _ = self.config.cancel.cancelled() => None,
            guard = self.gate.clone().read_owned() => Some(guard),
        }
    }

    pub(crate) fn charged_calls(&self) -> usize {
        self.charged.load(Ordering::Relaxed)
    }

    /// Charges one call against `python_max_nested_calls`; false when exhausted.
    pub(crate) fn charge_nested_call(&self) -> bool {
        self.charged
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < self.config.max_nested_calls).then(|| n + 1)
            })
            .is_ok()
    }

    pub fn help_text(&self, name: &str) -> Option<String> {
        self.config.catalog.help_text(name)
    }
    pub fn tool_names(&self) -> Vec<String> {
        self.config
            .catalog
            .functions
            .iter()
            .chain(&self.config.catalog.helpers)
            .map(|f| f.py_name.clone())
            .collect()
    }

    pub async fn dispatch(
        &self,
        seq: u32,
        function: &str,
        kwargs: Value,
        kwargs_truncated: bool,
    ) -> NestedOutcome {
        self.dispatch_at(seq, function, kwargs, kwargs_truncated, (None, None))
            .await
    }

    pub async fn dispatch_at(
        &self,
        seq: u32,
        function: &str,
        kwargs: Value,
        kwargs_truncated: bool,
        site: (Option<u32>, Option<u32>),
    ) -> NestedOutcome {
        let start = Instant::now();
        let mut record = PythonCallRecord {
            seq,
            call_id: format!("py_{}_{}", self.config.run_id, seq),
            tool: function.into(),
            args_preview: super::approvals::args_preview(&kwargs.to_string(), 1000, 200),
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
        self.config.sink.call_started(&record);
        let awaiting_approval = AtomicBool::new(false);
        let preflight = tokio::select! {
            biased;
            _ = self.config.cancel.cancelled() => Err(("RuntimeError", "python run cancelled".into(), true)),
            result = self.dispatch_inner(function, &kwargs, kwargs_truncated, &mut record, &awaiting_approval) => result,
        };
        if awaiting_approval.load(Ordering::Acquire) && preflight.is_err() {
            record.approval = PythonCallApproval::Cancelled;
        }
        let outcome = match preflight {
            Ok((call, messages, thread)) => self.execute(call, messages, thread).await,
            Err(error) => Err(error),
        };
        let (result, privacy, images, stop_run) = match outcome {
            Ok((messages, stopped)) => {
                let built =
                    build_envelope(&messages, self.config.max_nested_result_bytes, &self.images);
                record.status = if built.envelope["ok"] == true {
                    PythonCallStatus::Ok
                } else {
                    PythonCallStatus::Failed
                };
                record.summary = built.summary;
                let bytes = serde_json::to_vec(&built.diffs).map_or(usize::MAX, |b| b.len());
                if self
                    .diff_bytes
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                        used.checked_add(bytes).filter(|n| *n <= 256 * 1024)
                    })
                    .is_ok()
                {
                    record.diffs = built.diffs;
                } else {
                    record.summary.push_str(" [diffs dropped: message budget]");
                }
                (
                    HostResult::Ok(built.envelope),
                    built.privacy,
                    built.images,
                    stopped,
                )
            }
            Err((exc_type, message, stopped)) => {
                record.status = if stopped {
                    PythonCallStatus::Cancelled
                } else if exc_type == "PermissionError" {
                    PythonCallStatus::Denied
                } else {
                    PythonCallStatus::Failed
                };
                record.summary = preview(&message);
                (
                    HostResult::Err {
                        exc_type: exc_type.into(),
                        message,
                    },
                    Vec::new(),
                    Vec::new(),
                    stopped,
                )
            }
        };
        if !privacy.is_empty() {
            self.config
                .run_privacy
                .lock()
                .unwrap()
                .taint
                .extend(privacy.iter().cloned());
        }
        record.duration_ms = Some(start.elapsed().as_millis().min(u64::MAX as u128) as u64);
        let envelope = match &result {
            HostResult::Ok(value) => value.clone(),
            HostResult::Err {
                exc_type,
                message,
            } => json!({"exception":exc_type,"message":message}),
        };
        let artifact = json!({"seq":seq,"call_id":record.call_id,"tool":record.tool,"kwargs":kwargs,"envelope":envelope,"record":record});
        let name: String = record
            .tool
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dir = self.config.run_dir.join("calls");
        if let Err(error) = async {
            tokio::fs::create_dir_all(&dir).await?;
            tokio::fs::write(
                dir.join(format!("{seq}_{name}.json")),
                serde_json::to_vec(&artifact)?,
            )
            .await
        }
        .await
        {
            tracing::warn!(%error, "cannot persist nested tool call");
        }
        self.config.sink.call_finished(&record);
        NestedOutcome {
            result,
            record,
            privacy,
            images,
            stop_run,
        }
    }

    async fn dispatch_inner(
        &self,
        function: &str,
        kwargs: &Value,
        truncated: bool,
        record: &mut PythonCallRecord,
        awaiting_approval: &AtomicBool,
    ) -> Result<(ChatToolCall, Vec<ChatMessage>, ThreadParams), (&'static str, String, bool)> {
        let c = &self.config;
        let entry = c
            .catalog
            .resolve(function)
            .filter(|f| f.is_async)
            .ok_or_else(|| {
                (
                    "RuntimeError",
                    format!("unknown function: {function}"),
                    false,
                )
            })?;
        let tool = if function == "mcp_search" {
            "mcp_tool_search"
        } else {
            &entry.tool_name
        };
        record.tool = tool.into();
        if c.tool_catalog
            .as_ref()
            .is_some_and(|catalog| !catalog.index.tools.iter().any(|entry| entry.name == tool))
        {
            return Err((
                "PermissionError",
                "tool is outside chat tool set".into(),
                false,
            ));
        }
        if truncated {
            return Err(("ValueError", "arguments too large".into(), false));
        }
        let normalized = normalize_optional_nulls(&entry.input_schema, kwargs);
        let kwargs = &normalized;
        validate(&entry.input_schema, kwargs)?;
        if !self.charge_nested_call() {
            return Err(("RuntimeError", "nested call budget exhausted".into(), false));
        }
        if !c.allowed_tools.is_empty()
            && !c.allowed_tools.iter().any(|name| {
                crate::agents::tool_authority::canonical(name)
                    == crate::agents::tool_authority::canonical(tool)
            })
        {
            return Err((
                "PermissionError",
                "tool is outside allowed tools".into(),
                false,
            ));
        }
        if c.ccx
            .lock()
            .await
            .child_tool_ceiling
            .as_ref()
            .is_some_and(|ceiling| !crate::agents::tool_authority::allows_dispatch(ceiling, tool))
        {
            return Err((
                "PermissionError",
                "outside accepted child tool authority".into(),
                false,
            ));
        }
        let mut args = kwargs.clone();
        crate::agents::tool_authority::ensure_context_tool(&c.ccx, tool)
            .await
            .map_err(|error| ("PermissionError", error, false))?;
        let mut payload = HookPayload::new(
            &HookEvent::PreToolUse,
            c.chat_id.clone(),
            get_project_dir_string(c.app.clone()).await,
        )
        .with_tool(tool, Some(args.clone()));
        payload
            .extra
            .insert("python_run_id".into(), json!(c.run_id));
        payload.extra.insert(
            "python_parent_tool_call_id".into(),
            json!(c.outer_tool_call_id),
        );
        payload
            .extra
            .insert("tool_call_id".into(), json!(record.call_id));
        match run_control_hooks(c.app.clone(), HookEvent::PreToolUse, payload).await {
            ControlVerdict::Allow => {}
            ControlVerdict::Deny(reason) => return Err(("PermissionError", reason, false)),
            ControlVerdict::Rewrite(value) => {
                let authority = (!c.allowed_tools.is_empty()).then(|| {
                    crate::subchat::ToolsPolicy::Only(
                        c.allowed_tools
                            .iter()
                            .map(|s| crate::agents::tool_authority::canonical(s))
                            .collect(),
                    )
                });
                args = Value::Object(
                    revalidate_rewritten_tool_call(tool, &value, authority.as_ref())
                        .map_err(|e| ("PermissionError", e.to_string(), false))?,
                );
                validate(&entry.input_schema, &args)?;
                record.args_preview = super::approvals::args_preview(&args.to_string(), 1000, 200);
            }
        }
        let call: ChatToolCall = serde_json::from_value(json!({"id":record.call_id,"type":"function","function":{"name":tool,"arguments":args.to_string()}})).map_err(|e| ("RuntimeError", e.to_string(), false))?;
        let messages = c.ccx.lock().await.messages.clone();
        let thread =
            if let Some(session) = c.app.chat.sessions.read().await.get(&c.chat_id).cloned() {
                session.lock().await.thread.clone()
            } else {
                c.thread.clone()
            };
        let (asks, denials, auto_suppressed) = check_tools_confirmation_reporting_auto(
            c.app.clone(),
            std::slice::from_ref(&call),
            &messages,
            &thread,
            c.tool_catalog.as_deref(),
            &c.mode_id,
            c.model_id.as_deref(),
            &c.allowed_tools,
            "python",
            c.turn_tool_pool.as_ref(),
        )
        .await;
        if let Some(denial) = denials.first() {
            return Err(("PermissionError", denial.rule.clone(), false));
        }
        if asks.is_empty() && auto_suppressed.contains(&call.id) {
            record.approval = PythonCallApproval::Auto;
        }
        for ask in asks {
            awaiting_approval.store(true, Ordering::Release);
            let outcome = c
                .app
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
                        command: (!ask.command.is_empty() && ask.command != tool)
                            .then_some(ask.command),
                        rule: ask.rule,
                        editing_tool: is_editing_tool(tool),
                        engine: None,
                    },
                    &c.ccx,
                    &call,
                )
                .await;
            awaiting_approval.store(false, Ordering::Release);
            match outcome {
                ApprovalOutcome::Allowed {
                    for_chat,
                } => {
                    record.approval = if for_chat {
                        PythonCallApproval::AllowedForChat
                    } else {
                        PythonCallApproval::Allowed
                    }
                }
                ApprovalOutcome::AllowedForRun => {
                    record.approval = PythonCallApproval::AllowedForRun
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
        }
        Ok((call, messages, thread))
    }

    async fn execute(
        &self,
        call: ChatToolCall,
        messages: Vec<ChatMessage>,
        thread: ThreadParams,
    ) -> Result<(Vec<ChatMessage>, bool), (&'static str, String, bool)> {
        let c = &self.config;
        let _permit = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => return Err(("RuntimeError", "python run cancelled".into(), true)),
            permit = self.parallel.acquire() => permit.map_err(|_| ("RuntimeError", "dispatcher closed".into(), true))?,
        };
        let (subchat_tool_parameters, postprocess_settings) = {
            let context = c.ccx.lock().await;
            (
                context.subchat_tool_parameters.clone(),
                context.postprocess_parameters.clone(),
            )
        };
        let native_abort = Arc::new(AtomicBool::new(false));
        let (nested_context, outer_abort) = {
            let mut context = c.ccx.lock().await;
            let outer_abort = context.abort_flag.clone();
            let mut nested = context
                .freeze_tool_job(&call.id, native_abort.clone())
                .map_err(|e| ("RuntimeError", e, false))?;
            nested.execution_abort = native_abort.clone();
            (Arc::new(AMutex::new(nested)), outer_abort)
        };
        let calls = [call];
        let app = c.app.clone();
        let mode_id = c.mode_id.clone();
        let model_id = c.model_id.clone();
        let options = ExecuteToolsOptions {
            structured_output: true,
            wait_until_terminal: true,
            native_abort: Some(native_abort.clone()),
            execution_context: Some(nested_context),
            allowed_tools: c.allowed_tools.clone(),
            catalog: c.tool_catalog.clone(),
            turn_tool_pool: c.turn_tool_pool.clone(),
            subchat_tool_parameters: Some(subchat_tool_parameters),
            postprocess_settings: Some(postprocess_settings),
            results_for_script: true,
            ..Default::default()
        };
        // The drainer owns the foreground claim until nested execution is terminal.
        struct ParentRegistration {
            app: AppState,
            key: (String, String),
        }
        impl Drop for ParentRegistration {
            fn drop(&mut self) {
                self.app
                    .python_tool
                    .nested_parents
                    .lock()
                    .unwrap()
                    .remove(&self.key);
            }
        }
        let key = (c.chat_id.clone(), calls[0].id.clone());
        c.app
            .python_tool
            .nested_parents
            .lock()
            .unwrap()
            .insert(key.clone(), c.outer_tool_call_id.clone());
        let registration = ParentRegistration {
            app: c.app.clone(),
            key,
        };
        let mut execution = tokio::spawn(async move {
            let _registration = registration;
            execute_tools(
                app,
                &calls,
                &messages,
                &thread,
                &mode_id,
                model_id.as_deref(),
                options,
            )
            .await
        });
        tokio::select! {
            result = &mut execution => result.map(|r| (r.0, false)).map_err(|e| ("RuntimeError", e.to_string(), true)),
            _ = async {
                loop {
                    if outer_abort.load(Ordering::Acquire) { break; }
                    tokio::select! {
                        _ = c.cancel.cancelled() => break,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
                    }
                }
            } => {
                native_abort.store(true, Ordering::Release);
                // Cancellation is a request, not proof of terminal execution. Keep the
                // outer owner (and semaphore permit) alive until the drainer settles;
                // otherwise a scope mutation could pass a still-running nested tool.
                let warning = tokio::time::sleep(std::time::Duration::from_secs(5));
                tokio::pin!(warning);
                let mut warned = false;
                loop {
                    tokio::select! {
                        _ = &mut execution => break,
                        _ = &mut warning, if !warned => {
                            warned = true;
                            tracing::warn!(chat_id = %c.chat_id, run_id = %c.run_id,
                                "cancelled Python run waiting for nested execution to settle");
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                            // Only engine shutdown may escape this drain. Admission is
                            // permanently closed then; the spawned drainer still owns
                            // ParentRegistration until its execution actually ends.
                            if crate::tools::tool_python::engine_shutting_down(&c.app) {
                                break;
                            }
                        }
                    }
                }
                Err(("RuntimeError", "python run cancelled".into(), true))
            }
        }
    }
}

fn validate(schema: &Value, args: &Value) -> Result<(), (&'static str, String, bool)> {
    refact_tool_api::schema_validate::validate(schema, args).map_err(|errors| {
        (
            "TypeError",
            errors
                .iter()
                .map(|e| e.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
            false,
        )
    })
}

#[cfg(test)]
#[path = "dispatcher_tests.rs"]
mod tests;

#[cfg(test)]
mod optional_null_tests {
    use super::*;
    #[test]
    fn optional_nonnullable_null_is_omitted_but_required_and_nullable_survive() {
        let schema = json!({"type":"object","required":["required"],"properties":{"optional":{"type":"integer"},"required":{"type":"integer"},"nullable":{"type":["integer","null"]}}});
        let normalized = normalize_optional_nulls(
            &schema,
            &json!({"optional":null,"required":null,"nullable":null}),
        );
        assert!(normalized.get("optional").is_none());
        assert!(normalized.get("required").is_some());
        assert!(normalized.get("nullable").is_some());
        assert!(validate(&schema, &normalized).is_err());
    }
}

pub(crate) fn normalize_optional_nulls(schema: &Value, kwargs: &Value) -> Value {
    let mut kwargs = kwargs.clone();
    if let Some(object) = kwargs.as_object_mut() {
        object.retain(|name, value| {
            if !value.is_null() {
                return true;
            }
            let required = schema["required"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|key| key.as_str() == Some(name.as_str())));
            required || validate(&property_schema(schema, name), &Value::Null).is_ok()
        });
    }
    kwargs
}

fn property_schema(root: &Value, name: &str) -> Value {
    let mut property = root["properties"][name].clone();
    if let Some(object) = property.as_object_mut() {
        for key in ["$defs", "definitions"] {
            if let Some(defs) = root.get(key) {
                object.entry(key).or_insert_with(|| defs.clone());
            }
        }
    }
    property
}
