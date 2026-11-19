//! Script-only engine functions behind `await engine("area.function", **kwargs)`.
//! Each area file exports `functions()`; `registry()` aggregates them, one line per area.

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::future::BoxFuture;
use refact_chat_api::{PythonEngineClass, PythonEngineItem};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;

mod agents;
mod apps;
mod chat;
mod chats;
mod common;
mod dispatch;
mod gui;
mod hooks;
mod index;
mod integrations;
mod knowledge;
mod mask;
mod models;
mod modes;
mod previewed;
mod privacy;
mod processes;
mod providers;
mod scheduler;
mod session;
mod settings;
mod stats;
mod system;
#[cfg(test)]
mod test_fns;
#[cfg(test)]
mod tests;
mod tools;

pub use dispatch::EngineRunState;
pub(crate) use dispatch::APPLY as ENGINE_CHANGE_LOCK;
pub(crate) use chat::apply_queued_patch;
pub(crate) use dispatch::{failed_envelope, fit_for_script};

/// Key under which the kernel passes the positional function name of `engine("area.fn", ...)`;
/// not a Python identifier, so it never collides with a keyword argument such as `name`.
pub const FUNCTION_KEY: &str = "#function";
pub use mask::scrub;

fn registry() -> Vec<EngineFn> {
    let mut all = Vec::new();
    all.extend(settings::functions());
    all.extend(system::functions());
    all.extend(session::functions());
    all.extend(gui::functions());
    all.extend(stats::functions());
    all.extend(scheduler::functions());
    all.extend(index::functions());
    all.extend(knowledge::functions());
    all.extend(integrations::functions());
    all.extend(processes::functions());
    all.extend(apps::functions());
    all.extend(agents::functions());
    all.extend(chat::functions());
    all.extend(chats::functions());
    all.extend(models::functions());
    all.extend(providers::functions());
    all.extend(modes::functions());
    all.extend(tools::functions());
    all.extend(hooks::functions());
    all.extend(privacy::functions());
    #[cfg(test)]
    all.extend(test_fns::functions());
    all.sort_by(|a, b| a.name.cmp(b.name));
    all
}

static REGISTRY: LazyLock<Vec<EngineFn>> = LazyLock::new(registry);

pub fn functions() -> &'static [EngineFn] {
    &REGISTRY
}

pub fn lookup(name: &str) -> Option<&'static EngineFn> {
    functions().iter().find(|f| f.name == name)
}

/// Plain documentation handed to the catalog crate.
pub fn docs() -> Vec<refact_python_catalog::EngineFunctionDoc> {
    functions()
        .iter()
        .map(|f| refact_python_catalog::EngineFunctionDoc {
            name: f.name.into(),
            area: f.area().into(),
            class: f.class.as_str().into(),
            summary: f.summary.into(),
            input_schema: (f.input)(),
            output_schema: (f.output)(),
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EngineClass {
    Read,
    RedactedRead,
    Change,
    Risky,
    Destructive,
}

impl EngineClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::RedactedRead => "redacted_read",
            Self::Change => "change",
            Self::Risky => "risky",
            Self::Destructive => "destructive",
        }
    }
    pub fn is_read(self) -> bool {
        matches!(self, Self::Read | Self::RedactedRead)
    }
    pub fn wire(self) -> Option<PythonEngineClass> {
        match self {
            Self::Read | Self::RedactedRead => None,
            Self::Change => Some(PythonEngineClass::Change),
            Self::Risky => Some(PythonEngineClass::Risky),
            Self::Destructive => Some(PythonEngineClass::Destructive),
        }
    }
}

/// What the user approves: the change preview of one call.
#[derive(Debug)]
pub struct Preview {
    /// Lower-case verb phrase without a full stop, e.g. "change engine settings".
    pub title: String,
    pub items: Vec<PythonEngineItem>,
    pub scope: Option<String>,
    /// May raise the function's base class for this call, never lower it.
    pub class: EngineClass,
    /// One plain sentence; required for risky and destructive.
    pub warning: Option<String>,
    /// Never shown and never on the wire: what the handler applies after approval.
    pub plan: Plan,
}

/// The opaque plan of a preview: the state it was computed from and what will be applied.
#[derive(Default)]
pub struct Plan {
    state: Option<Value>,
    payload: Option<Arc<dyn std::any::Any + Send + Sync>>,
    nothing: bool,
}

impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plan")
            .field("state", &self.state)
            .field("nothing", &self.nothing)
            .finish()
    }
}

impl Plan {
    /// A plan computed from `state` that applies `payload`.
    pub fn new(state: Value, payload: impl std::any::Any + Send + Sync) -> Self {
        Self {
            state: Some(state),
            payload: Some(Arc::new(payload)),
            nothing: false,
        }
    }
    /// The call would change nothing: no card, ok true with "nothing to change".
    pub fn nothing() -> Self {
        Self {
            nothing: true,
            ..Default::default()
        }
    }
    pub fn is_nothing(&self) -> bool {
        self.nothing
    }
}

/// Run facts an engine function may need besides the chat context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NestedCalls {
    /// Nested calls charged so far in this run, this call included.
    pub used: usize,
    pub limit: usize,
    pub parallel_limit: usize,
}

/// Typed output of a function; `privacy` holds privacy records (`{"files": [...]}`) of chat or file content it returns.
#[derive(Clone, Debug, Default)]
pub struct EngineOutput {
    pub data: Value,
    pub text: String,
    pub privacy: Vec<Value>,
    /// A change that stopped part of the way: the result is ok false with its data.
    pub stopped: Option<Stopped>,
}

/// Why a change stopped, and the part of its card that was applied before it stopped; the change
/// is recorded with those items only, and not at all when there are none.
#[derive(Clone, Debug)]
pub struct Stopped {
    pub error: String,
    pub applied: Vec<PythonEngineItem>,
}

impl EngineOutput {
    pub fn new(data: Value, text: impl Into<String>) -> Self {
        Self {
            data,
            text: text.into(),
            privacy: Vec::new(),
            stopped: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// Bad arguments: raises TypeError.
    Type(String),
    /// Refused: raises PermissionError.
    Permission(String),
    /// The function failed: returns ok=false.
    Failed(String),
}

pub struct EngineCtx {
    pub app: AppState,
    pub ccx: Arc<AMutex<AtCommandsContext>>,
    pub chat_id: String,
    pub run_id: String,
    /// Tool call id of the outer `python` call.
    pub outer_tool_call_id: String,
    pub project_root: Option<PathBuf>,
    /// Coordination settings effective for this project at call time.
    pub settings: refact_work_core::CoordinationSettings,
    /// The run's `llm()` budget; `None` when the helper is switched off.
    pub llm: Option<Arc<crate::python_tool::llm::LlmBudget>>,
    /// The run's privacy records (session taint, nested results, direct reads).
    pub run_privacy: Arc<std::sync::Mutex<crate::python_tool::llm::RunPrivacy>>,
    pub nested: NestedCalls,
    /// The approved plan, handed over by the pipeline after the person allowed the change.
    pub plan: std::sync::Mutex<Option<Plan>>,
    /// This run's engine state; `system.shutdown` asks for a shutdown after the run ends.
    pub run: Option<Arc<EngineRunState>>,
    /// Set by a handler whose approved change a task now completes even if the call ends: a
    /// timeout of the call then says so instead of leaving the outcome unexplained.
    pub completing: std::sync::Mutex<Option<String>>,
}

impl EngineCtx {
    /// The approved plan's payload when the state read now equals the state the preview was computed from.
    pub fn take_plan<T: Clone + 'static>(
        &self,
        function: &str,
        current: &Value,
    ) -> Result<T, EngineError> {
        let plan = self
            .plan
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .filter(|plan| plan.state.is_some())
            .ok_or_else(|| {
                EngineError::Failed(format!(
                    "{function}: no preview of this change was recorded; nothing was applied"
                ))
            })?;
        let shown = plan.state.unwrap_or_default();
        if shown != *current {
            return Err(EngineError::Failed(format!(
                "{function}: changed since the preview ({}); nothing was applied, call it again to see a fresh preview",
                moved_parts(&shown, current)
            )));
        }
        plan.payload
            .and_then(|payload| payload.downcast_ref::<T>().cloned())
            .ok_or_else(|| {
                EngineError::Failed(format!(
                    "{function}: the approved plan has an unexpected shape; nothing was applied"
                ))
            })
    }

    /// Runs the privacy egress check for `destination` over this chat's records, the session
    /// taint and the run's records, with the decision function `llm()` uses.
    pub async fn clear_for(&self, destination: &refact_privacy::Destination) -> Result<(), String> {
        let (taint, reads) = {
            let run = self.run_privacy.lock().unwrap_or_else(|e| e.into_inner());
            (run.taint.clone(), run.reads.clone())
        };
        let (history, derived) = {
            let context = self.ccx.lock().await;
            (
                context.messages.clone(),
                context.derived_privacy_zones.clone(),
            )
        };
        let mut carrier = crate::call_validation::ChatMessage::default();
        crate::privacy::records::carry_records_into(&mut carrier, &history)
            .map_err(|e| e.to_string())?;
        crate::python_tool::run_output::merge_taint(&mut carrier, &taint)?;
        let direct = crate::privacy::records::observed_file_records_with_derived(
            &self.app.gcx,
            reads.iter().map(PathBuf::from),
            &derived,
        )?;
        crate::privacy::records::merge_records(&mut carrier, direct);
        struct Carrier<'a>(&'a [crate::call_validation::ChatMessage]);
        impl refact_privacy::PrivacyAudited for Carrier<'_> {
            fn privacy_records(
                &self,
            ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>
            {
                refact_privacy::records_from_messages(self.0)
            }
        }
        crate::privacy::destinations::clear_current(
            &self.app.gcx,
            Carrier(std::slice::from_ref(&carrier)),
            destination,
        )
        .map(|_| ())
        .map_err(|refusal| refusal.model_facing())
    }
}

/// Names what moved between two plan states without quoting any value: the differing top-level
/// keys when both are objects, otherwise only that the state differs.
fn moved_parts(shown: &Value, now: &Value) -> String {
    let (Value::Object(a), Value::Object(b)) = (shown, now) else {
        return "its state differs".into();
    };
    let mut keys: Vec<&str> = a
        .keys()
        .chain(b.keys())
        .filter(|k| a.get(*k) != b.get(*k))
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let total = keys.len();
    let mut text = keys.into_iter().take(10).collect::<Vec<_>>().join(", ");
    if total > 10 {
        text.push_str(&format!(" and {} more", total - 10));
    }
    format!("moved: {text}")
}

/// Hex sha256 of `bytes`, for plan states that must not carry the content itself.
pub(crate) fn digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

pub type Handler =
    for<'a> fn(&'a EngineCtx, &'a Value) -> BoxFuture<'a, Result<EngineOutput, EngineError>>;
pub type PreviewFn =
    for<'a> fn(&'a EngineCtx, &'a Value) -> BoxFuture<'a, Result<Preview, EngineError>>;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// One engine function. Names are `area.function` in lower snake case.
pub struct EngineFn {
    pub name: &'static str,
    pub summary: &'static str,
    pub class: EngineClass,
    pub input: fn() -> Value,
    pub output: fn() -> Option<Value>,
    pub handler: Handler,
    /// Required for every class other than read and redacted_read.
    pub preview: Option<PreviewFn>,
    pub timeout: Option<Duration>,
}

impl EngineFn {
    pub fn area(&self) -> &'static str {
        self.name
            .split_once('.')
            .map_or(self.name, |(area, _)| area)
    }
}

/// Close names for an unknown one, best first.
pub fn suggestions(name: &str) -> Vec<&'static str> {
    let mut scored: Vec<(f64, &'static str)> = functions()
        .iter()
        .map(|f| (strsim::jaro_winkler(name, f.name), f.name))
        .filter(|(score, _)| *score >= 0.75)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
    scored.into_iter().take(3).map(|(_, name)| name).collect()
}
