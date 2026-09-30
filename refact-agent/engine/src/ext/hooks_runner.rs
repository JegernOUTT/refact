use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::app_state::AppState;
use crate::ext::config_dirs::{get_ext_dirs, CommandSource, ExtDirs};
use crate::ext::hooks::{load_hooks, HookConfig, HookEvent, HookKind};
use crate::files_correction::canonical_path;
use crate::privacy::{check_file_privacy, FilePrivacyLevel, PrivacySettings};
use crate::subchat::ToolsPolicy;

pub use crate::ext::hooks::HooksConfig;

const HOOKS_CACHE_TTL: Duration = Duration::from_secs(5);
const MAX_CONCURRENT_HOOKS: usize = 5;
const HOOK_POLL_INTERVAL: Duration = Duration::from_millis(50);
const PRIVACY_REDACTED: &str = "[privacy-blocked]";

static HOOK_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookLimits {
    pub default_timeout_secs: u64,
    pub max_output_bytes: usize,
    pub max_recursion_depth: u32,
}

impl Default for HookLimits {
    fn default() -> Self {
        Self::from(&HooksConfig::default())
    }
}

impl From<&HooksConfig> for HookLimits {
    fn from(config: &HooksConfig) -> Self {
        Self {
            default_timeout_secs: config.default_timeout_secs,
            max_output_bytes: config.max_output_bytes,
            max_recursion_depth: config.max_recursion_depth,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HookPayload {
    pub hook_event_name: String,
    pub session_id: String,
    pub project_dir: String,
    #[serde(rename = "hook_depth")]
    pub depth: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_prompt: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl HookPayload {
    pub fn new(event: &HookEvent, session_id: String, project_dir: String) -> Self {
        Self {
            hook_event_name: event.as_str().to_string(),
            session_id,
            project_dir,
            depth: 0,
            tool_name: None,
            tool_input: None,
            tool_output: None,
            user_prompt: None,
            extra: HashMap::new(),
        }
    }

    pub fn with_tool(mut self, name: &str, input: Option<serde_json::Value>) -> Self {
        self.tool_name = Some(name.to_string());
        self.tool_input = input;
        self
    }

    pub fn with_tool_output(mut self, output: String) -> Self {
        self.tool_output = Some(output);
        self
    }

    pub fn with_user_prompt(mut self, prompt: String) -> Self {
        self.user_prompt = Some(prompt);
        self
    }

    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }
}

/// What a raw hook process did. Control and observation hooks both go through
/// the same bounded execution, then interpret the result differently.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RawHookOutcome {
    Completed {
        code: i32,
        stdout: String,
        stderr: String,
    },
    Timeout,
    Cancelled,
    Failed(String),
}

/// A control hook's answer. Only a control hook can produce one of these.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlOutcome {
    Allow,
    Deny(String),
    Rewrite(serde_json::Value),
    Timeout,
    Cancelled,
    Failed(String),
}

/// An observation hook's answer. There is deliberately no conversion from this
/// type into a verdict, so an observation hook can neither block nor alter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationOutcome {
    Observed,
    Failed(String),
    Timeout,
    Cancelled,
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlVerdict {
    Allow,
    Deny(String),
    Rewrite(serde_json::Value),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRecursionError {
    pub event: String,
    pub depth: u32,
    pub max_depth: u32,
}

impl std::fmt::Display for HookRecursionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "hook recursion limit reached for {}: depth {} exceeds max_recursion_depth {}",
            self.event, self.depth, self.max_depth
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookRewriteError {
    NotAnObject,
    OutsideAuthority { tool: String },
}

impl std::fmt::Display for HookRewriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookRewriteError::NotAnObject => {
                write!(f, "hook rewrite must be a JSON object of tool arguments")
            }
            HookRewriteError::OutsideAuthority { tool } => write!(
                f,
                "hook rewrite refused: '{tool}' is outside the accepted tool authority"
            ),
        }
    }
}

pub fn check_hook_recursion(
    event: &HookEvent,
    depth: u32,
    max_depth: u32,
) -> Result<(), HookRecursionError> {
    if depth >= max_depth {
        Err(HookRecursionError {
            event: event.as_str().to_string(),
            depth,
            max_depth,
        })
    } else {
        Ok(())
    }
}

/// A hook bound to a control event. Constructing one is the only way to obtain
/// a `ControlOutcome`, and it is impossible for an observation event.
pub struct ControlHook {
    config: HookConfig,
}

/// A hook bound to an observation event. Its result carries no decision.
pub struct ObservationHook {
    config: HookConfig,
}

impl ControlHook {
    pub fn new(config: HookConfig) -> Option<Self> {
        matches!(config.event.kind(), HookKind::Control).then_some(Self { config })
    }

    async fn evaluate(
        &self,
        payload: &HookPayload,
        limits: &HookLimits,
        shutdown: Option<&Arc<AtomicBool>>,
    ) -> ControlOutcome {
        match run_single_hook(&self.config, payload, limits, shutdown).await {
            RawHookOutcome::Completed {
                code,
                stdout,
                stderr,
            } => control_outcome_from_process(code, &stdout, &stderr),
            RawHookOutcome::Timeout => ControlOutcome::Timeout,
            RawHookOutcome::Cancelled => ControlOutcome::Cancelled,
            RawHookOutcome::Failed(reason) => ControlOutcome::Failed(reason),
        }
    }
}

impl ObservationHook {
    pub fn new(config: HookConfig) -> Option<Self> {
        matches!(config.event.kind(), HookKind::Observation).then_some(Self { config })
    }

    async fn observe(
        &self,
        payload: &HookPayload,
        limits: &HookLimits,
        shutdown: Option<&Arc<AtomicBool>>,
    ) -> ObservationOutcome {
        match run_single_hook(&self.config, payload, limits, shutdown).await {
            RawHookOutcome::Completed { code, stderr, .. } => {
                if code == 0 {
                    ObservationOutcome::Observed
                } else {
                    tracing::warn!(
                        "hooks_runner: observation hook '{}' exited with code {}: {}",
                        self.config.command,
                        code,
                        stderr
                    );
                    ObservationOutcome::Failed(stderr)
                }
            }
            RawHookOutcome::Timeout => ObservationOutcome::Timeout,
            RawHookOutcome::Cancelled => ObservationOutcome::Cancelled,
            RawHookOutcome::Failed(reason) => ObservationOutcome::Failed(reason),
        }
    }
}

fn control_outcome_from_process(code: i32, stdout: &str, stderr: &str) -> ControlOutcome {
    if code == 0 || code == 2 {
        if let Some(outcome) = control_outcome_from_json(stdout) {
            return outcome;
        }
    }
    match code {
        0 => ControlOutcome::Allow,
        2 => {
            let reason = if stderr.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                stderr.trim().to_string()
            };
            ControlOutcome::Deny(reason)
        }
        other => {
            ControlOutcome::Failed(format!("hook exited with code {other}: {}", stderr.trim()))
        }
    }
}

fn control_outcome_from_json(stdout: &str) -> Option<ControlOutcome> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let decision = value.get("decision")?.as_str()?;
    match decision {
        "allow" => Some(ControlOutcome::Allow),
        "deny" | "block" => Some(ControlOutcome::Deny(
            value
                .get("reason")
                .and_then(|r| r.as_str())
                .unwrap_or("denied by hook")
                .to_string(),
        )),
        "rewrite" => match value.get("tool_input") {
            Some(input) => Some(ControlOutcome::Rewrite(input.clone())),
            None => Some(ControlOutcome::Failed(
                "hook requested a rewrite without tool_input".to_string(),
            )),
        },
        other => Some(ControlOutcome::Failed(format!(
            "hook returned unknown decision '{other}'"
        ))),
    }
}

/// A verdict together with the hook that produced it. Deny precedence is
/// deterministic, so the hook that actually decided is always identifiable.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlDecision {
    pub verdict: ControlVerdict,
    /// Index into the outcomes that were resolved. `None` for an allow that
    /// nothing objected to.
    pub decided_by: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecisionResult {
    pub accepted: bool,
    pub denial_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecisionRequest {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub accepted: bool,
    pub source: String,
    pub request_id: Option<String>,
    pub tool_call_id: String,
    pub reason_type: String,
}

/// Deny always wins, whatever the order the outcomes arrive in and whatever
/// their timing. A timeout or a shutdown cancellation is a deny, never a
/// fallthrough to allow. A non-blocking hook failure stays advisory.
pub fn resolve_control_decision(outcomes: &[ControlOutcome]) -> ControlDecision {
    let decision = |verdict, index| ControlDecision {
        verdict,
        decided_by: Some(index),
    };
    let explicit_deny = outcomes.iter().enumerate().find_map(|(index, o)| match o {
        ControlOutcome::Deny(reason) => Some((index, reason.clone())),
        _ => None,
    });
    if let Some((index, reason)) = explicit_deny {
        return decision(ControlVerdict::Deny(reason), index);
    }
    if let Some(index) = outcomes
        .iter()
        .position(|o| matches!(o, ControlOutcome::Timeout))
    {
        return decision(
            ControlVerdict::Deny("control hook timed out, failing closed".to_string()),
            index,
        );
    }
    if let Some(index) = outcomes
        .iter()
        .position(|o| matches!(o, ControlOutcome::Cancelled))
    {
        return decision(
            ControlVerdict::Deny("control hook cancelled by shutdown".to_string()),
            index,
        );
    }
    let rewrite = outcomes.iter().enumerate().find_map(|(index, o)| match o {
        ControlOutcome::Rewrite(value) => Some((index, value.clone())),
        _ => None,
    });
    match rewrite {
        Some((index, value)) => decision(ControlVerdict::Rewrite(value), index),
        None => ControlDecision {
            verdict: ControlVerdict::Allow,
            decided_by: None,
        },
    }
}

pub fn resolve_control_verdict(outcomes: &[ControlOutcome]) -> ControlVerdict {
    resolve_control_decision(outcomes).verdict
}

fn referenced_tool_names(args: &serde_json::Value) -> Vec<String> {
    let mut found = Vec::new();
    collect_tool_names(args, &mut found);
    found
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn collect_tool_names(value: &serde_json::Value, found: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if matches!(
                    key.as_str(),
                    "tool_name" | "tool" | "tools" | "allowed_tools"
                ) {
                    match child {
                        serde_json::Value::String(name) => push_tool_names(name, found),
                        serde_json::Value::Array(items) => {
                            for item in items {
                                if let Some(name) = item.as_str() {
                                    push_tool_names(name, found);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                collect_tool_names(child, found);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_tool_names(item, found);
            }
        }
        _ => {}
    }
}

fn push_tool_names(raw: &str, found: &mut Vec<String>) {
    for name in raw.split(',') {
        let name = name.trim();
        if !name.is_empty() {
            found.push(name.to_string());
        }
    }
}

/// Why a registered hook is or is not executed. "Untrusted project" is the one
/// this surface must never blur: such a hook is listed, and never run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookTrust {
    Global,
    TrustedProject,
    UntrustedProject,
    InstalledPlugin,
}

pub fn source_trust(source: &CommandSource, trusted_projects: &[String]) -> HookTrust {
    match source {
        CommandSource::GlobalClaude | CommandSource::GlobalRefact => HookTrust::Global,
        CommandSource::ProjectClaude(root) | CommandSource::ProjectRefact(root) => {
            if project_root_trusted(root, trusted_projects) {
                HookTrust::TrustedProject
            } else {
                HookTrust::UntrustedProject
            }
        }
        CommandSource::InstalledPlugin(_) => HookTrust::InstalledPlugin,
    }
}

pub fn source_label(source: &CommandSource) -> String {
    match source {
        CommandSource::GlobalClaude => "global .claude".to_string(),
        CommandSource::GlobalRefact => "global refact".to_string(),
        CommandSource::ProjectClaude(root) => format!("project .claude {}", root.display()),
        CommandSource::ProjectRefact(root) => format!("project refact {}", root.display()),
        CommandSource::InstalledPlugin(name) => format!("installed plugin {name}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookRegistration {
    pub event: String,
    pub kind: &'static str,
    pub matcher: Option<String>,
    pub command: String,
    pub source: String,
    pub trust: HookTrust,
    /// False means this hook has never run and never will under the current
    /// trust configuration.
    pub runs: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookOutcomeTag {
    Allow,
    Deny,
    Rewrite,
    Timeout,
    Cancelled,
    Failed,
    Observed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookRun {
    pub command: String,
    pub source: String,
    pub trust: HookTrust,
    pub outcome: HookOutcomeTag,
    pub detail: Option<String>,
    /// This hook's answer is the one deny precedence selected.
    pub decided: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookVerdictTag {
    Allow,
    Deny,
    Rewrite,
    Observed,
    RecursionBlocked,
    RewriteAccepted,
    RewriteRefused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookActivityRecord {
    pub event: String,
    pub kind: &'static str,
    pub verdict: HookVerdictTag,
    /// The command of the hook that decided, when one did.
    pub decided_by: Option<String>,
    pub reason: Option<String>,
    pub tool: Option<String>,
    pub runs: Vec<HookRun>,
    pub at_ms: u64,
}

const HOOK_ACTIVITY_CAPACITY: usize = 100;
pub const HOOK_ACTIVITY_DEFAULT_LIMIT: usize = 50;

/// A bounded record of what hooks actually did. Read-only: nothing here can be
/// replayed or edited.
pub struct HookActivityLog {
    entries: std::sync::Mutex<std::collections::VecDeque<HookActivityRecord>>,
    capacity: usize,
}

impl HookActivityLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: std::sync::Mutex::new(std::collections::VecDeque::new()),
            capacity: capacity.max(1),
        }
    }

    pub fn record(&self, record: HookActivityRecord) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.push_back(record);
        while entries.len() > self.capacity {
            entries.pop_front();
        }
    }

    /// Newest first.
    pub fn recent(&self, limit: usize) -> Vec<HookActivityRecord> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.iter().rev().take(limit).cloned().collect()
    }
}

static HOOK_ACTIVITY: OnceLock<HookActivityLog> = OnceLock::new();

pub fn hook_activity() -> &'static HookActivityLog {
    HOOK_ACTIVITY.get_or_init(|| HookActivityLog::new(HOOK_ACTIVITY_CAPACITY))
}

#[cfg(test)]
pub(crate) fn clear_hook_cache_for_tests() {
    if let Some(cache) = HOOKS_CACHE.get() {
        if let Ok(mut guard) = cache.try_write() {
            *guard = None;
        }
    }
    if let Some(activity) = HOOK_ACTIVITY.get() {
        if let Ok(mut entries) = activity.entries.lock() {
            entries.clear();
        }
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn control_run(
    config: &HookConfig,
    outcome: &ControlOutcome,
    trusted: &[String],
    decided: bool,
) -> HookRun {
    let (tag, detail) = match outcome {
        ControlOutcome::Allow => (HookOutcomeTag::Allow, None),
        ControlOutcome::Deny(reason) => (HookOutcomeTag::Deny, Some(reason.clone())),
        ControlOutcome::Rewrite(_) => (HookOutcomeTag::Rewrite, None),
        ControlOutcome::Timeout => (HookOutcomeTag::Timeout, None),
        ControlOutcome::Cancelled => (HookOutcomeTag::Cancelled, None),
        ControlOutcome::Failed(reason) => (HookOutcomeTag::Failed, Some(reason.clone())),
    };
    HookRun {
        command: config.command.clone(),
        source: source_label(&config.source),
        trust: source_trust(&config.source, trusted),
        outcome: tag,
        detail,
        decided,
    }
}

fn observation_run(
    config: &HookConfig,
    outcome: &ObservationOutcome,
    trusted: &[String],
) -> HookRun {
    let (tag, detail) = match outcome {
        ObservationOutcome::Observed => (HookOutcomeTag::Observed, None),
        ObservationOutcome::Failed(reason) => (HookOutcomeTag::Failed, Some(reason.clone())),
        ObservationOutcome::Timeout => (HookOutcomeTag::Timeout, None),
        ObservationOutcome::Cancelled => (HookOutcomeTag::Cancelled, None),
        ObservationOutcome::Skipped(reason) => (HookOutcomeTag::Skipped, Some(reason.clone())),
    };
    HookRun {
        command: config.command.clone(),
        source: source_label(&config.source),
        trust: source_trust(&config.source, trusted),
        outcome: tag,
        detail,
        decided: false,
    }
}

pub fn control_activity(
    event: &HookEvent,
    tool: Option<&str>,
    configs: &[HookConfig],
    outcomes: &[ControlOutcome],
    trusted: &[String],
    decision: &ControlDecision,
) -> HookActivityRecord {
    let (verdict, reason) = match &decision.verdict {
        ControlVerdict::Allow => (HookVerdictTag::Allow, None),
        ControlVerdict::Deny(reason) => (HookVerdictTag::Deny, Some(reason.clone())),
        ControlVerdict::Rewrite(_) => (HookVerdictTag::Rewrite, None),
    };
    HookActivityRecord {
        event: event.as_str().to_string(),
        kind: "control",
        verdict,
        decided_by: decision
            .decided_by
            .and_then(|index| configs.get(index))
            .map(|config| config.command.clone()),
        reason,
        tool: tool.map(str::to_string),
        runs: configs
            .iter()
            .zip(outcomes.iter())
            .enumerate()
            .map(|(index, (config, outcome))| {
                control_run(config, outcome, trusted, decision.decided_by == Some(index))
            })
            .collect(),
        at_ms: unix_now_ms(),
    }
}

pub fn observation_activity(
    event: &HookEvent,
    tool: Option<&str>,
    configs: &[HookConfig],
    outcomes: &[ObservationOutcome],
    trusted: &[String],
) -> HookActivityRecord {
    HookActivityRecord {
        event: event.as_str().to_string(),
        kind: "observation",
        verdict: HookVerdictTag::Observed,
        decided_by: None,
        reason: None,
        tool: tool.map(str::to_string),
        runs: configs
            .iter()
            .zip(outcomes.iter())
            .map(|(config, outcome)| observation_run(config, outcome, trusted))
            .collect(),
        at_ms: unix_now_ms(),
    }
}

pub fn recursion_activity(
    event: &HookEvent,
    tool: Option<&str>,
    error: &HookRecursionError,
) -> HookActivityRecord {
    HookActivityRecord {
        event: event.as_str().to_string(),
        kind: "control",
        verdict: HookVerdictTag::RecursionBlocked,
        decided_by: None,
        reason: Some(error.to_string()),
        tool: tool.map(str::to_string),
        runs: Vec::new(),
        at_ms: unix_now_ms(),
    }
}

pub fn revalidation_activity(tool: &str, refused: Option<String>) -> HookActivityRecord {
    HookActivityRecord {
        event: HookEvent::PreToolUse.as_str().to_string(),
        kind: "rewrite_revalidation",
        verdict: match refused {
            Some(_) => HookVerdictTag::RewriteRefused,
            None => HookVerdictTag::RewriteAccepted,
        },
        decided_by: None,
        reason: refused,
        tool: Some(tool.to_string()),
        runs: Vec::new(),
        at_ms: unix_now_ms(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookSurface {
    pub enabled: bool,
    pub registrations: Vec<HookRegistration>,
    pub recent: Vec<HookActivityRecord>,
    pub registered: usize,
    pub runnable: usize,
    pub blocked: usize,
    pub denials: usize,
    pub max_recursion_depth: u32,
    pub generated_at_ms: u64,
}

pub fn hook_registration(config: &HookConfig, hooks: &HooksConfig) -> HookRegistration {
    let trust = source_trust(&config.source, &hooks.trusted_projects);
    let blocked_reason = if !hooks.enabled {
        Some("hooks are disabled in privacy.yaml, so nothing is executed".to_string())
    } else {
        match trust {
            HookTrust::Global | HookTrust::TrustedProject => None,
            HookTrust::UntrustedProject => Some(
                "this project is not listed in hooks.trusted_projects, so this hook is never executed"
                    .to_string(),
            ),
            HookTrust::InstalledPlugin => {
                Some("hooks shipped by an installed plugin are never executed".to_string())
            }
        }
    };
    HookRegistration {
        event: config.event.as_str().to_string(),
        kind: match config.event.kind() {
            HookKind::Control => "control",
            HookKind::Observation => "observation",
        },
        matcher: config.matcher.clone(),
        command: config.command.clone(),
        source: source_label(&config.source),
        trust,
        runs: blocked_reason.is_none(),
        blocked_reason,
    }
}

pub fn build_hook_surface(
    configs: &[HookConfig],
    hooks: &HooksConfig,
    recent: Vec<HookActivityRecord>,
) -> HookSurface {
    let mut registrations: Vec<HookRegistration> = configs
        .iter()
        .map(|config| hook_registration(config, hooks))
        .collect();
    registrations.sort_by(|left, right| {
        left.event
            .cmp(&right.event)
            .then_with(|| left.command.cmp(&right.command))
            .then_with(|| left.source.cmp(&right.source))
    });
    let runnable = registrations.iter().filter(|entry| entry.runs).count();
    HookSurface {
        enabled: hooks.enabled,
        registered: registrations.len(),
        blocked: registrations.len() - runnable,
        runnable,
        denials: recent
            .iter()
            .filter(|record| {
                matches!(
                    record.verdict,
                    HookVerdictTag::Deny
                        | HookVerdictTag::RecursionBlocked
                        | HookVerdictTag::RewriteRefused
                )
            })
            .count(),
        registrations,
        recent,
        max_recursion_depth: hooks.max_recursion_depth,
        generated_at_ms: unix_now_ms(),
    }
}

/// Every registered hook, whether or not it is allowed to run, plus what the
/// hooks that did run actually returned.
pub async fn hook_surface(app: AppState, limit: usize) -> HookSurface {
    let hooks = app.gcx.hooks_config.clone();
    let ext_dirs = get_ext_dirs(app).await;
    let compiled = get_compiled_hooks_from_ext_dirs(&ext_dirs).await;
    let configs: Vec<HookConfig> = compiled.iter().map(|hook| hook.config.clone()).collect();
    build_hook_surface(&configs, &hooks, hook_activity().recent(limit))
}

/// A rewritten tool call is re-checked against the same authority the original
/// call is checked against, so a hook can never widen authority or reach a tool
/// outside the ceiling. Records the outcome so a refused rewrite is visible.
pub fn revalidate_rewritten_tool_call(
    tool_name: &str,
    rewritten: &serde_json::Value,
    authority: Option<&ToolsPolicy>,
) -> Result<serde_json::Map<String, serde_json::Value>, HookRewriteError> {
    let result = revalidate_rewritten_arguments(tool_name, rewritten, authority);
    hook_activity().record(revalidation_activity(
        tool_name,
        result.as_ref().err().map(ToString::to_string),
    ));
    result
}

/// A rewritten tool call is re-checked against the same authority the original
/// call is checked against, so a hook can never widen authority or reach a tool
/// outside the ceiling.
pub fn revalidate_rewritten_arguments(
    tool_name: &str,
    rewritten: &serde_json::Value,
    authority: Option<&ToolsPolicy>,
) -> Result<serde_json::Map<String, serde_json::Value>, HookRewriteError> {
    let object = rewritten
        .as_object()
        .ok_or(HookRewriteError::NotAnObject)?
        .clone();
    let Some(authority) = authority else {
        return Ok(object);
    };
    if !crate::agents::tool_authority::allows_dispatch(authority, tool_name) {
        return Err(HookRewriteError::OutsideAuthority {
            tool: tool_name.to_string(),
        });
    }
    for referenced in referenced_tool_names(rewritten) {
        if !crate::agents::tool_authority::allows_dispatch(authority, &referenced) {
            return Err(HookRewriteError::OutsideAuthority { tool: referenced });
        }
    }
    Ok(object)
}

#[derive(Clone)]
struct CompiledHook {
    config: HookConfig,
    compiled_matcher: Option<regex::Regex>,
}

struct HooksCacheEntry {
    hooks: Arc<Vec<CompiledHook>>,
    loaded_at: Instant,
    project_key: String,
}

static HOOKS_CACHE: OnceLock<tokio::sync::RwLock<Option<HooksCacheEntry>>> = OnceLock::new();

fn compile_hooks(hooks: Vec<HookConfig>) -> Vec<CompiledHook> {
    let mut result = Vec::new();
    for config in hooks {
        let compiled_matcher = match &config.matcher {
            None => None,
            Some(p) if p.is_empty() => None,
            Some(p) => match regex::Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    tracing::warn!("hooks_runner: invalid matcher regex '{}': {}", p, e);
                    continue;
                }
            },
        };
        result.push(CompiledHook {
            config,
            compiled_matcher,
        });
    }
    result
}

fn compiled_matcher_matches(compiled: Option<&regex::Regex>, tool_name: Option<&str>) -> bool {
    match compiled {
        None => true,
        Some(re) => match tool_name {
            Some(name) => re.is_match(name),
            None => false,
        },
    }
}

async fn get_compiled_hooks_from_ext_dirs(ext_dirs: &ExtDirs) -> Arc<Vec<CompiledHook>> {
    let project_key = ext_dirs
        .project_dirs
        .first()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lock = HOOKS_CACHE.get_or_init(|| tokio::sync::RwLock::new(None));
    {
        let read = lock.read().await;
        if let Some(entry) = &*read {
            if entry.loaded_at.elapsed() < HOOKS_CACHE_TTL && entry.project_key == project_key {
                return Arc::clone(&entry.hooks);
            }
        }
    }
    let raw_hooks = load_hooks(ext_dirs).await;
    let compiled = Arc::new(compile_hooks(raw_hooks));
    let mut write = lock.write().await;
    *write = Some(HooksCacheEntry {
        hooks: Arc::clone(&compiled),
        loaded_at: Instant::now(),
        project_key,
    });
    compiled
}

pub async fn get_project_dir_string(app: AppState) -> String {
    let dirs = crate::files_correction::get_project_dirs(app.gcx.clone()).await;
    dirs.into_iter()
        .next()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn is_global_source(source: &CommandSource) -> bool {
    matches!(
        source,
        CommandSource::GlobalClaude | CommandSource::GlobalRefact
    )
}

pub fn is_project_source(source: &CommandSource) -> bool {
    matches!(
        source,
        CommandSource::ProjectClaude(_) | CommandSource::ProjectRefact(_)
    )
}

/// Whether a hook from `source` is allowed to run. Global hooks always run.
/// Project hooks run only when their project root is listed in
/// `hooks.trusted_projects`, so trusting one repository never trusts another.
/// Installed-plugin hooks are not executed.
pub fn hook_source_allowed(source: &CommandSource, trusted_projects: &[String]) -> bool {
    match source {
        CommandSource::GlobalClaude | CommandSource::GlobalRefact => true,
        CommandSource::ProjectClaude(root) | CommandSource::ProjectRefact(root) => {
            project_root_trusted(root, trusted_projects)
        }
        CommandSource::InstalledPlugin(_) => false,
    }
}

fn project_root_trusted(root: &Path, trusted_projects: &[String]) -> bool {
    let canon_root = canonical_path(root.to_string_lossy().into_owned());
    trusted_projects
        .iter()
        .any(|trusted| canonical_path(trusted.clone()) == canon_root)
}

pub fn redact_privacy_blocked_paths(value: &mut serde_json::Value, privacy: &Arc<PrivacySettings>) {
    match value {
        serde_json::Value::String(text) => {
            if check_file_privacy(
                privacy.clone(),
                Path::new(text.as_str()),
                &FilePrivacyLevel::OnlySendToServersIControl,
            )
            .is_err()
            {
                *text = PRIVACY_REDACTED.to_string();
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_privacy_blocked_paths(item, privacy);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, child) in map.iter_mut() {
                redact_privacy_blocked_paths(child, privacy);
            }
        }
        _ => {}
    }
}

async fn apply_privacy_filter(app: &AppState, payload: &mut HookPayload) {
    let privacy = crate::privacy::load_privacy_if_needed(app.gcx.clone()).await;
    if let Some(input) = payload.tool_input.as_mut() {
        redact_privacy_blocked_paths(input, &privacy);
    }
}

async fn matching_hooks(
    app: AppState,
    event: &HookEvent,
    tool_name: Option<&str>,
    trusted_projects: &[String],
) -> Vec<HookConfig> {
    let ext_dirs = get_ext_dirs(app).await;
    let compiled = get_compiled_hooks_from_ext_dirs(&ext_dirs).await;
    compiled
        .iter()
        .filter(|h| hook_source_allowed(&h.config.source, trusted_projects))
        .filter(|h| h.config.event == *event)
        .filter(|h| compiled_matcher_matches(h.compiled_matcher.as_ref(), tool_name))
        .map(|h| h.config.clone())
        .collect()
}

/// Run every control hook registered for `event` and collapse the answers into
/// a single deterministic verdict.
pub async fn run_control_hooks(
    app: AppState,
    event: HookEvent,
    payload: HookPayload,
) -> ControlVerdict {
    run_control_hooks_with_decider(app, event, payload).await.0
}

async fn run_control_hooks_with_decider(
    app: AppState,
    event: HookEvent,
    mut payload: HookPayload,
) -> (ControlVerdict, Option<String>) {
    let config = app.gcx.hooks_config.clone();
    if !config.enabled {
        return (ControlVerdict::Allow, None);
    }
    let limits = HookLimits::from(&config);
    if let Err(error) = check_hook_recursion(&event, payload.depth, limits.max_recursion_depth) {
        tracing::warn!("hooks_runner: {}", error);
        hook_activity().record(recursion_activity(
            &event,
            payload.tool_name.as_deref(),
            &error,
        ));
        return (ControlVerdict::Deny(error.to_string()), None);
    }
    let shutdown = app.gcx.shutdown_flag.clone();
    if shutdown.load(Ordering::Relaxed) {
        return (ControlVerdict::Allow, None);
    }
    let tool_name = payload.tool_name.clone();
    let hooks = matching_hooks(
        app.clone(),
        &event,
        tool_name.as_deref(),
        &config.trusted_projects,
    )
    .await;
    let control_hooks: Vec<ControlHook> = hooks.into_iter().filter_map(ControlHook::new).collect();
    if control_hooks.is_empty() {
        return (ControlVerdict::Allow, None);
    }
    apply_privacy_filter(&app, &mut payload).await;
    let futs: Vec<_> = control_hooks
        .iter()
        .map(|hook| run_with_permit(hook.evaluate(&payload, &limits, Some(&shutdown))))
        .collect();
    let outcomes = futures::future::join_all(futs).await;
    let decision = resolve_control_decision(&outcomes);
    let configs: Vec<HookConfig> = control_hooks
        .iter()
        .map(|hook| hook.config.clone())
        .collect();
    hook_activity().record(control_activity(
        &event,
        tool_name.as_deref(),
        &configs,
        &outcomes,
        &config.trusted_projects,
        &decision,
    ));
    let decided_by = decision
        .decided_by
        .and_then(|index| configs.get(index))
        .map(|config| config.command.clone());
    (decision.verdict, decided_by)
}

pub async fn apply_permission_decision(
    app: AppState,
    session_id: String,
    project_dir: String,
    request: PermissionDecisionRequest,
) -> PermissionDecisionResult {
    if !app.gcx.hooks_config.enabled {
        return PermissionDecisionResult {
            accepted: request.accepted,
            denial_reason: None,
        };
    }
    let event = HookEvent::PermissionDecision;
    let mut payload = HookPayload::new(&event, session_id, project_dir)
        .with_tool(&request.tool_name, Some(request.tool_input));
    payload.extra.insert(
        "decision".into(),
        serde_json::json!(if request.accepted {
            "approve"
        } else {
            "reject"
        }),
    );
    payload
        .extra
        .insert("source".into(), serde_json::json!(request.source));
    payload
        .extra
        .insert("reason_type".into(), serde_json::json!(request.reason_type));
    payload.extra.insert(
        "tool_call_id".into(),
        serde_json::json!(request.tool_call_id),
    );
    if let Some(request_id) = request.request_id {
        payload
            .extra
            .insert("request_id".into(), serde_json::json!(request_id));
    }
    if !request.accepted {
        run_control_hooks_with_decider(app, event, payload).await;
        return PermissionDecisionResult {
            accepted: false,
            denial_reason: None,
        };
    }
    let raw_decision = request.accepted;
    let denial_limit = HookLimits::from(&app.gcx.hooks_config).max_output_bytes;
    let (verdict, hook) = run_control_hooks_with_decider(app, event, payload).await;
    match verdict {
        ControlVerdict::Deny(reason) => {
            let denial_reason = match hook {
                Some(hook) => format!("denied by PermissionDecision hook '{hook}': {reason}"),
                None => format!("denied by PermissionDecision hook: {reason}"),
            };
            PermissionDecisionResult {
                accepted: false,
                denial_reason: Some(truncate_utf8_bytes(&denial_reason, denial_limit)),
            }
        }
        ControlVerdict::Rewrite(_) => {
            tracing::warn!("hooks_runner: ignored rewrite for PermissionDecision event");
            PermissionDecisionResult {
                accepted: raw_decision,
                denial_reason: None,
            }
        }
        ControlVerdict::Allow => PermissionDecisionResult {
            accepted: true,
            denial_reason: None,
        },
    }
}

/// Run every observation hook registered for `event`. Observation hooks are
/// awaited so they cannot outlive shutdown, but nothing they return can block
/// or alter the turn.
pub async fn run_observation_hooks(
    app: AppState,
    event: HookEvent,
    mut payload: HookPayload,
) -> Vec<ObservationOutcome> {
    let config = app.gcx.hooks_config.clone();
    if !config.enabled {
        return vec![];
    }
    let limits = HookLimits::from(&config);
    if let Err(error) = check_hook_recursion(&event, payload.depth, limits.max_recursion_depth) {
        tracing::warn!("hooks_runner: {}", error);
        hook_activity().record(recursion_activity(
            &event,
            payload.tool_name.as_deref(),
            &error,
        ));
        return vec![ObservationOutcome::Skipped(error.to_string())];
    }
    let shutdown = app.gcx.shutdown_flag.clone();
    if shutdown.load(Ordering::Relaxed) {
        return vec![];
    }
    let tool_name = payload.tool_name.clone();
    let hooks = matching_hooks(
        app.clone(),
        &event,
        tool_name.as_deref(),
        &config.trusted_projects,
    )
    .await;
    let observation_hooks: Vec<ObservationHook> =
        hooks.into_iter().filter_map(ObservationHook::new).collect();
    if observation_hooks.is_empty() {
        return vec![];
    }
    apply_privacy_filter(&app, &mut payload).await;
    let futs: Vec<_> = observation_hooks
        .iter()
        .map(|hook| run_with_permit(hook.observe(&payload, &limits, Some(&shutdown))))
        .collect();
    let outcomes = futures::future::join_all(futs).await;
    let configs: Vec<HookConfig> = observation_hooks
        .iter()
        .map(|hook| hook.config.clone())
        .collect();
    hook_activity().record(observation_activity(
        &event,
        tool_name.as_deref(),
        &configs,
        &outcomes,
        &config.trusted_projects,
    ));
    outcomes
}

async fn run_with_permit<F: std::future::Future>(fut: F) -> F::Output {
    let semaphore =
        HOOK_SEMAPHORE.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_HOOKS));
    let _permit = semaphore
        .acquire()
        .await
        .expect("hook semaphore should not be closed");
    fut.await
}

async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(mut reader: R, max_bytes: usize) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                let to_store = (max_bytes.saturating_sub(buf.len())).min(n);
                if to_store > 0 {
                    buf.extend_from_slice(&chunk[..to_store]);
                }
            }
            Err(_) => break,
        }
    }
    buf
}

async fn run_single_hook(
    config: &HookConfig,
    payload: &HookPayload,
    limits: &HookLimits,
    shutdown: Option<&Arc<AtomicBool>>,
) -> RawHookOutcome {
    let payload_json = match serde_json::to_string(payload) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("hooks_runner: failed to serialize payload: {}", e);
            return RawHookOutcome::Failed(format!("failed to serialize payload: {}", e));
        }
    };

    let timeout_secs = config.timeout.unwrap_or(limits.default_timeout_secs);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

    let mut cmd = make_hook_command(config, payload);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("hooks_runner: failed to spawn '{}': {}", config.command, e);
            return RawHookOutcome::Failed(format!("failed to spawn: {}", e));
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload_json.as_bytes()).await;
        drop(stdin);
    }

    let max_output = limits.max_output_bytes;
    let stdout_task = child
        .stdout
        .take()
        .map(|out| tokio::spawn(read_bounded(out, max_output)));
    let stderr_task = child
        .stderr
        .take()
        .map(|err| tokio::spawn(read_bounded(err, max_output)));

    let terminal = loop {
        if shutdown.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            tracing::info!(
                "hooks_runner: shutdown requested, killing hook: {}",
                config.command
            );
            let _ = child.kill().await;
            let _ = child.wait().await;
            break RawHookOutcome::Cancelled;
        }
        match tokio::time::timeout(HOOK_POLL_INTERVAL, child.wait()).await {
            Ok(Ok(status)) => {
                break RawHookOutcome::Completed {
                    code: status.code().unwrap_or(-1),
                    stdout: String::new(),
                    stderr: String::new(),
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "hooks_runner: failed to wait for '{}': {}",
                    config.command,
                    e
                );
                break RawHookOutcome::Failed(format!("failed to wait: {}", e));
            }
            Err(_) => {
                if tokio::time::Instant::now() >= deadline {
                    tracing::warn!(
                        "hooks_runner: hook timed out after {}s, killing process: {}",
                        timeout_secs,
                        config.command
                    );
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    break RawHookOutcome::Timeout;
                }
            }
        }
    };

    match terminal {
        RawHookOutcome::Completed { code, .. } => {
            let stdout_bytes = match stdout_task {
                Some(t) => t.await.unwrap_or_default(),
                None => Vec::new(),
            };
            let stderr_bytes = match stderr_task {
                Some(t) => t.await.unwrap_or_default(),
                None => Vec::new(),
            };
            RawHookOutcome::Completed {
                code,
                stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
                stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
            }
        }
        other => {
            if let Some(t) = stdout_task {
                t.abort();
            }
            if let Some(t) = stderr_task {
                t.abort();
            }
            other
        }
    }
}

fn make_hook_command(config: &HookConfig, payload: &HookPayload) -> tokio::process::Command {
    #[cfg(unix)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(&config.command);
        c
    };

    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/c").arg(&config.command);
        c
    };

    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("REFACT_PROJECT_DIR", &payload.project_dir)
        .env("REFACT_SESSION_ID", &payload.session_id)
        .env("REFACT_HOOK_EVENT", &payload.hook_event_name)
        .env("REFACT_HOOK_DEPTH", payload.depth.to_string());

    cmd
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_payload(event: &HookEvent, tool_name: Option<&str>) -> HookPayload {
        let mut payload = HookPayload::new(event, "test-session".to_string(), "/tmp".to_string());
        payload.tool_name = tool_name.map(|s| s.to_string());
        payload
    }

    async fn test_app(
        config_dir: &std::path::Path,
        workspace: &std::path::Path,
        enabled: bool,
    ) -> AppState {
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.join("cache"),
            config_dir.to_path_buf(),
        )
        .await;
        Arc::get_mut(&mut gcx).unwrap().hooks_config.enabled = enabled;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.into()];
        AppState::from_gcx(gcx).await
    }

    fn make_hook_config(event: HookEvent, matcher: Option<&str>, command: &str) -> HookConfig {
        HookConfig {
            event,
            matcher: matcher.map(|s| s.to_string()),
            command: command.to_string(),
            timeout: Some(5),
            source: CommandSource::GlobalRefact,
        }
    }

    fn permission_config(config_dir: &std::path::Path, command: &str) {
        std::fs::create_dir_all(config_dir).unwrap();
        std::fs::write(
            config_dir.join("hooks.yaml"),
            format!(
                "hooks:\n  PermissionDecision:\n    - hooks:\n        - type: command\n          command: {command:?}\n"
            ),
        )
        .unwrap();
        clear_hook_cache_for_tests();
    }

    fn stderr_exit_command(message: &str, code: i32) -> String {
        if cfg!(windows) {
            format!("echo {message} 1>&2 && exit /b {code}")
        } else {
            format!("sh -c 'echo {message} >&2; exit {code}'")
        }
    }

    async fn evaluate(config: &HookConfig, payload: &HookPayload) -> ControlOutcome {
        ControlHook::new(config.clone())
            .expect("event should be a control event")
            .evaluate(payload, &HookLimits::default(), None)
            .await
    }

    async fn observe(config: &HookConfig, payload: &HookPayload) -> ObservationOutcome {
        ObservationHook::new(config.clone())
            .expect("event should be an observation event")
            .observe(payload, &HookLimits::default(), None)
            .await
    }

    #[test]
    fn test_payload_serialization_minimal() {
        let payload = make_payload(&HookEvent::PreToolUse, None);
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("PreToolUse"));
        assert!(json.contains("test-session"));
        assert!(json.contains("\"hook_depth\":0"));
        assert!(!json.contains("tool_name"));
        assert!(!json.contains("tool_output"));
        assert!(!json.contains("user_prompt"));
    }

    #[test]
    fn test_payload_serialization_with_tool() {
        let payload = make_payload(&HookEvent::PreToolUse, None)
            .with_tool("shell", Some(serde_json::json!({"cmd": "ls"})));
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("shell"));
        assert!(json.contains("tool_input"));
        assert!(json.contains("cmd"));
    }

    #[test]
    fn test_payload_extra_flattened() {
        let mut payload = make_payload(&HookEvent::Stop, None);
        payload
            .extra
            .insert("finish_reason".to_string(), serde_json::json!("stop"));
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("finish_reason"));
        assert!(json.contains("\"stop\""));
    }

    #[test]
    fn test_every_event_builds_a_payload_naming_itself() {
        let events = [
            HookEvent::PreToolUse,
            HookEvent::PostToolUse,
            HookEvent::UserPromptSubmit,
            HookEvent::PermissionDecision,
            HookEvent::SessionStart,
            HookEvent::SessionEnd,
            HookEvent::Stop,
            HookEvent::SubagentStart,
            HookEvent::SubagentStop,
            HookEvent::Notification,
            HookEvent::PreCompact,
            HookEvent::PostCompact,
            HookEvent::GoalValidation,
        ];
        for event in events {
            let payload = HookPayload::new(&event, "session".to_string(), "/project".to_string());
            assert_eq!(payload.hook_event_name, event.as_str());
            assert_eq!(payload.session_id, "session");
            assert_eq!(payload.project_dir, "/project");
            assert_eq!(payload.depth, 0);
            let json = serde_json::to_value(&payload).unwrap();
            assert_eq!(json["hook_event_name"], event.as_str());
        }
    }

    #[test]
    fn test_tool_events_carry_tool_payload_fields() {
        let pre = make_payload(&HookEvent::PreToolUse, None)
            .with_tool("shell", Some(serde_json::json!({"command": "ls"})));
        assert_eq!(pre.tool_name.as_deref(), Some("shell"));
        assert_eq!(pre.tool_input.as_ref().unwrap()["command"], "ls");
        let post = pre.clone().with_tool_output("done".to_string());
        assert_eq!(post.tool_output.as_deref(), Some("done"));
        let prompt =
            make_payload(&HookEvent::UserPromptSubmit, None).with_user_prompt("hi".to_string());
        assert_eq!(prompt.user_prompt.as_deref(), Some("hi"));
    }

    #[test]
    fn test_control_hook_cannot_be_built_for_observation_event() {
        for event in [
            HookEvent::SessionStart,
            HookEvent::SessionEnd,
            HookEvent::Stop,
            HookEvent::SubagentStart,
            HookEvent::SubagentStop,
            HookEvent::Notification,
            HookEvent::PostCompact,
            HookEvent::GoalValidation,
        ] {
            let config = make_hook_config(event, None, &stderr_exit_command("nope", 2));
            assert!(ControlHook::new(config.clone()).is_none());
            assert!(ObservationHook::new(config).is_some());
        }
    }

    #[test]
    fn test_observation_hook_cannot_be_built_for_control_event() {
        for event in [
            HookEvent::PreToolUse,
            HookEvent::PostToolUse,
            HookEvent::UserPromptSubmit,
            HookEvent::PermissionDecision,
            HookEvent::PreCompact,
        ] {
            let config = make_hook_config(event, None, "echo ok");
            assert!(ObservationHook::new(config.clone()).is_none());
            assert!(ControlHook::new(config).is_some());
        }
    }

    #[tokio::test]
    async fn test_observation_hook_exit_two_does_not_block_or_alter() {
        let config = make_hook_config(
            HookEvent::SessionStart,
            None,
            &stderr_exit_command("i_want_to_block", 2),
        );
        let payload = make_payload(&HookEvent::SessionStart, None);
        let outcome = observe(&config, &payload).await;
        assert!(matches!(outcome, ObservationOutcome::Failed(_)));
        assert!(ControlHook::new(config).is_none());
    }

    #[tokio::test]
    async fn test_observation_hook_rewrite_attempt_is_ignored() {
        let config = make_hook_config(
            HookEvent::Stop,
            None,
            "echo '{\"decision\":\"rewrite\",\"tool_input\":{\"command\":\"rm -rf /\"}}'",
        );
        let payload = make_payload(&HookEvent::Stop, None);
        assert_eq!(
            observe(&config, &payload).await,
            ObservationOutcome::Observed
        );
    }

    #[tokio::test]
    async fn test_observation_hook_failure_is_not_fatal() {
        let config = make_hook_config(HookEvent::SessionEnd, None, "definitely_not_a_command_xyz");
        let payload = make_payload(&HookEvent::SessionEnd, None);
        let outcome = observe(&config, &payload).await;
        assert!(matches!(
            outcome,
            ObservationOutcome::Failed(_) | ObservationOutcome::Observed
        ));
    }

    #[test]
    fn test_deny_wins_over_allow_in_any_order() {
        let allow_first = vec![
            ControlOutcome::Allow,
            ControlOutcome::Allow,
            ControlOutcome::Deny("nope".to_string()),
        ];
        let deny_first = vec![
            ControlOutcome::Deny("nope".to_string()),
            ControlOutcome::Allow,
            ControlOutcome::Allow,
        ];
        let interleaved = vec![
            ControlOutcome::Allow,
            ControlOutcome::Deny("nope".to_string()),
            ControlOutcome::Allow,
        ];
        for outcomes in [allow_first, deny_first, interleaved] {
            assert_eq!(
                resolve_control_verdict(&outcomes),
                ControlVerdict::Deny("nope".to_string())
            );
        }
    }

    #[test]
    fn test_deny_wins_over_rewrite_in_any_order() {
        let rewrite = ControlOutcome::Rewrite(serde_json::json!({"a": 1}));
        let deny = ControlOutcome::Deny("nope".to_string());
        assert_eq!(
            resolve_control_verdict(&[rewrite.clone(), deny.clone()]),
            ControlVerdict::Deny("nope".to_string())
        );
        assert_eq!(
            resolve_control_verdict(&[deny, rewrite]),
            ControlVerdict::Deny("nope".to_string())
        );
    }

    #[test]
    fn test_timeout_is_a_deny_and_never_falls_through_to_allow() {
        let verdict = resolve_control_verdict(&[ControlOutcome::Timeout]);
        assert!(matches!(verdict, ControlVerdict::Deny(_)));
        assert_ne!(verdict, ControlVerdict::Allow);
        let with_allows = resolve_control_verdict(&[
            ControlOutcome::Allow,
            ControlOutcome::Timeout,
            ControlOutcome::Allow,
        ]);
        assert!(matches!(with_allows, ControlVerdict::Deny(_)));
        let with_rewrite = resolve_control_verdict(&[
            ControlOutcome::Rewrite(serde_json::json!({"a": 1})),
            ControlOutcome::Timeout,
        ]);
        assert!(matches!(with_rewrite, ControlVerdict::Deny(_)));
    }

    #[test]
    fn test_explicit_deny_reason_beats_timeout_reason_deterministically() {
        let a = resolve_control_verdict(&[
            ControlOutcome::Timeout,
            ControlOutcome::Deny("explicit".to_string()),
        ]);
        let b = resolve_control_verdict(&[
            ControlOutcome::Deny("explicit".to_string()),
            ControlOutcome::Timeout,
        ]);
        assert_eq!(a, ControlVerdict::Deny("explicit".to_string()));
        assert_eq!(a, b);
    }

    #[test]
    fn test_cancelled_control_hook_is_a_deny() {
        let verdict = resolve_control_verdict(&[ControlOutcome::Cancelled, ControlOutcome::Allow]);
        assert!(matches!(verdict, ControlVerdict::Deny(_)));
    }

    #[test]
    fn test_advisory_failure_still_allows() {
        let verdict = resolve_control_verdict(&[
            ControlOutcome::Failed("hook crashed".to_string()),
            ControlOutcome::Allow,
        ]);
        assert_eq!(verdict, ControlVerdict::Allow);
    }

    #[test]
    fn test_empty_outcomes_allow() {
        assert_eq!(resolve_control_verdict(&[]), ControlVerdict::Allow);
    }

    #[test]
    fn test_rewrite_survives_when_nothing_denies() {
        let verdict = resolve_control_verdict(&[
            ControlOutcome::Allow,
            ControlOutcome::Rewrite(serde_json::json!({"command": "ls -la"})),
        ]);
        assert_eq!(
            verdict,
            ControlVerdict::Rewrite(serde_json::json!({"command": "ls -la"}))
        );
    }

    #[test]
    fn test_control_outcome_from_process_exit_codes() {
        assert_eq!(
            control_outcome_from_process(0, "", ""),
            ControlOutcome::Allow
        );
        assert_eq!(
            control_outcome_from_process(2, "", "blocked here"),
            ControlOutcome::Deny("blocked here".to_string())
        );
        assert!(matches!(
            control_outcome_from_process(1, "", "oops"),
            ControlOutcome::Failed(_)
        ));
    }

    #[test]
    fn test_control_outcome_from_json_decisions() {
        assert_eq!(
            control_outcome_from_process(0, r#"{"decision":"deny","reason":"no"}"#, ""),
            ControlOutcome::Deny("no".to_string())
        );
        assert_eq!(
            control_outcome_from_process(0, r#"{"decision":"allow"}"#, ""),
            ControlOutcome::Allow
        );
        assert_eq!(
            control_outcome_from_process(0, r#"{"decision":"rewrite","tool_input":{"a":1}}"#, ""),
            ControlOutcome::Rewrite(serde_json::json!({"a": 1}))
        );
        assert!(matches!(
            control_outcome_from_process(0, r#"{"decision":"rewrite"}"#, ""),
            ControlOutcome::Failed(_)
        ));
        assert_eq!(
            control_outcome_from_process(0, "just some text", ""),
            ControlOutcome::Allow
        );
    }

    #[test]
    fn test_recursion_check_produces_structured_error_at_the_limit() {
        assert!(check_hook_recursion(&HookEvent::PreToolUse, 0, 2).is_ok());
        assert!(check_hook_recursion(&HookEvent::PreToolUse, 1, 2).is_ok());
        let error = check_hook_recursion(&HookEvent::PreToolUse, 2, 2).unwrap_err();
        assert_eq!(
            error,
            HookRecursionError {
                event: "PreToolUse".to_string(),
                depth: 2,
                max_depth: 2,
            }
        );
        assert!(error.to_string().contains("max_recursion_depth"));
        assert!(check_hook_recursion(&HookEvent::PreToolUse, 9, 2).is_err());
    }

    #[test]
    fn test_recursion_error_is_a_deny_not_a_hang() {
        let error = check_hook_recursion(&HookEvent::PreToolUse, 5, 2).unwrap_err();
        let verdict = ControlVerdict::Deny(error.to_string());
        assert!(matches!(verdict, ControlVerdict::Deny(reason) if reason.contains("recursion")));
    }

    #[test]
    fn test_rewrite_must_be_an_object() {
        let error =
            revalidate_rewritten_arguments("shell", &serde_json::json!("ls"), None).unwrap_err();
        assert_eq!(error, HookRewriteError::NotAnObject);
    }

    #[test]
    fn test_rewrite_without_authority_is_passed_through() {
        let args = revalidate_rewritten_arguments(
            "shell",
            &serde_json::json!({"command": "ls -la"}),
            None,
        )
        .unwrap();
        assert_eq!(args["command"], "ls -la");
    }

    #[test]
    fn test_rewrite_within_authority_is_accepted() {
        let authority = ToolsPolicy::Only(vec!["shell".to_string()]);
        let args = revalidate_rewritten_arguments(
            "shell",
            &serde_json::json!({"command": "ls -la"}),
            Some(&authority),
        )
        .unwrap();
        assert_eq!(args["command"], "ls -la");
    }

    #[test]
    fn test_rewrite_cannot_reach_a_tool_outside_authority() {
        let authority = ToolsPolicy::Only(vec!["cat".to_string()]);
        let error = revalidate_rewritten_arguments(
            "shell",
            &serde_json::json!({"command": "ls"}),
            Some(&authority),
        )
        .unwrap_err();
        assert_eq!(
            error,
            HookRewriteError::OutsideAuthority {
                tool: "shell".to_string()
            }
        );
    }

    #[test]
    fn test_rewrite_cannot_widen_child_tool_authority() {
        let authority = ToolsPolicy::Only(vec!["subagent".to_string(), "cat".to_string()]);
        let widened = serde_json::json!({
            "task": "do it",
            "tools": ["cat", "shell"]
        });
        let error =
            revalidate_rewritten_arguments("subagent", &widened, Some(&authority)).unwrap_err();
        assert_eq!(
            error,
            HookRewriteError::OutsideAuthority {
                tool: "shell".to_string()
            }
        );
        let narrowed = serde_json::json!({"task": "do it", "tools": ["cat"]});
        assert!(revalidate_rewritten_arguments("subagent", &narrowed, Some(&authority)).is_ok());
    }

    #[test]
    fn test_rewrite_cannot_widen_nested_tool_reference() {
        let authority = ToolsPolicy::Only(vec!["cat".to_string()]);
        let nested = serde_json::json!({
            "goal": {"budget": {"max_turns": 3}},
            "spec": {"allowed_tools": "cat, shell"}
        });
        let error = revalidate_rewritten_arguments("cat", &nested, Some(&authority)).unwrap_err();
        assert_eq!(
            error,
            HookRewriteError::OutsideAuthority {
                tool: "shell".to_string()
            }
        );
    }

    #[test]
    fn test_rewrite_denied_by_a_none_authority() {
        let authority = ToolsPolicy::None;
        assert!(revalidate_rewritten_arguments(
            "cat",
            &serde_json::json!({"paths": "a.rs"}),
            Some(&authority)
        )
        .is_err());
    }

    #[test]
    fn test_referenced_tool_names_collects_all_shapes() {
        let value = serde_json::json!({
            "tool_name": "alpha",
            "nested": {"tools": ["beta", "gamma"]},
            "list": [{"allowed_tools": "delta, epsilon"}]
        });
        let mut names = referenced_tool_names(&value);
        names.sort();
        assert_eq!(names, vec!["alpha", "beta", "delta", "epsilon", "gamma"]);
    }

    #[test]
    fn test_compiled_matcher_matches_none_matches_all() {
        assert!(compiled_matcher_matches(None, Some("shell")));
        assert!(compiled_matcher_matches(None, Some("cat")));
        assert!(compiled_matcher_matches(None, None));
    }

    #[test]
    fn test_compiled_matcher_matches_pattern_with_tool_name() {
        let re = regex::Regex::new("Bash|shell").unwrap();
        assert!(compiled_matcher_matches(Some(&re), Some("shell")));
        assert!(compiled_matcher_matches(Some(&re), Some("Bash")));
        assert!(!compiled_matcher_matches(Some(&re), Some("cat")));
    }

    #[test]
    fn test_compiled_matcher_matches_pattern_without_tool_name_returns_false() {
        let re = regex::Regex::new("shell").unwrap();
        assert!(!compiled_matcher_matches(Some(&re), None));
    }

    #[test]
    fn test_compile_hooks_skips_invalid_regex() {
        let configs = vec![
            make_hook_config(HookEvent::PreToolUse, Some("[invalid"), "cmd1"),
            make_hook_config(HookEvent::PreToolUse, Some("shell"), "cmd2"),
        ];
        let compiled = compile_hooks(configs);
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled[0].config.command, "cmd2");
    }

    #[test]
    fn test_compile_hooks_none_matcher_becomes_match_all() {
        let configs = vec![make_hook_config(HookEvent::SessionStart, None, "cmd")];
        let compiled = compile_hooks(configs);
        assert_eq!(compiled.len(), 1);
        assert!(compiled[0].compiled_matcher.is_none());
        assert!(compiled_matcher_matches(
            compiled[0].compiled_matcher.as_ref(),
            Some("anything")
        ));
    }

    #[test]
    fn test_compile_hooks_empty_matcher_becomes_match_all() {
        let configs = vec![make_hook_config(HookEvent::SessionStart, Some(""), "cmd")];
        let compiled = compile_hooks(configs);
        assert_eq!(compiled.len(), 1);
        assert!(compiled[0].compiled_matcher.is_none());
    }

    #[tokio::test]
    async fn test_control_hook_success_allows() {
        let config = make_hook_config(HookEvent::PreToolUse, None, "echo success_output");
        let payload = make_payload(&HookEvent::PreToolUse, Some("shell"));
        assert_eq!(evaluate(&config, &payload).await, ControlOutcome::Allow);
    }

    #[tokio::test]
    async fn test_control_hook_exit_2_denies() {
        let config = make_hook_config(
            HookEvent::PreToolUse,
            None,
            &stderr_exit_command("block_reason", 2),
        );
        let payload = make_payload(&HookEvent::PreToolUse, Some("shell"));
        match evaluate(&config, &payload).await {
            ControlOutcome::Deny(reason) => assert!(reason.contains("block_reason")),
            other => panic!("Expected Deny, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_control_hook_nonzero_exit_is_advisory() {
        let config = make_hook_config(
            HookEvent::PostToolUse,
            None,
            &stderr_exit_command("warn_output", 1),
        );
        let payload = make_payload(&HookEvent::PostToolUse, Some("cat"));
        match evaluate(&config, &payload).await {
            ControlOutcome::Failed(msg) => assert!(msg.contains("warn_output")),
            other => panic!("Expected Failed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_control_hook_timeout_resolves_to_deny() {
        let mut config = make_hook_config(HookEvent::PreToolUse, None, "sleep 60");
        config.timeout = Some(1);
        let payload = make_payload(&HookEvent::PreToolUse, None);
        let outcome = evaluate(&config, &payload).await;
        assert_eq!(outcome, ControlOutcome::Timeout);
        let verdict = resolve_control_verdict(&[outcome]);
        assert!(matches!(verdict, ControlVerdict::Deny(_)));
        assert_ne!(verdict, ControlVerdict::Allow);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_hook_timeout_kills_process() {
        let tmpdir = tempfile::tempdir().unwrap();
        let pid_file = tmpdir.path().join("pid.txt");
        let pid_path_str = pid_file.to_str().unwrap().to_string();
        let cmd = format!("echo $$ > '{}'; sleep 60", pid_path_str);
        let mut config = make_hook_config(HookEvent::SessionStart, None, &cmd);
        config.timeout = Some(2);
        let payload = make_payload(&HookEvent::SessionStart, None);
        let result = observe(&config, &payload).await;
        assert_eq!(result, ObservationOutcome::Timeout);

        let pid_str = std::fs::read_to_string(&pid_file)
            .expect("PID file should have been written by the child process");
        let pid: u32 = pid_str.trim().parse().expect("PID should be numeric");

        let kill_output = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .expect("kill command should execute");
        assert!(
            !kill_output.status.success(),
            "Process {} should have been killed and reaped after timeout, but kill -0 succeeded",
            pid
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_shutdown_cancels_and_joins_in_flight_hook() {
        let tmpdir = tempfile::tempdir().unwrap();
        let pid_file = tmpdir.path().join("pid.txt");
        let pid_path_str = pid_file.to_str().unwrap().to_string();
        let cmd = format!("echo $$ > '{}'; sleep 60", pid_path_str);
        let mut config = make_hook_config(HookEvent::PreToolUse, None, &cmd);
        config.timeout = Some(600);
        let payload = make_payload(&HookEvent::PreToolUse, None);
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            flag.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let hook = ControlHook::new(config).unwrap();
        let outcome = hook
            .evaluate(&payload, &HookLimits::default(), Some(&shutdown))
            .await;
        assert_eq!(outcome, ControlOutcome::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "shutdown should cancel the hook instead of waiting for its timeout"
        );
        assert!(matches!(
            resolve_control_verdict(&[outcome]),
            ControlVerdict::Deny(_)
        ));

        let pid: u32 = std::fs::read_to_string(&pid_file)
            .expect("PID file should exist")
            .trim()
            .parse()
            .expect("PID should be numeric");
        let kill_output = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .expect("kill command should execute");
        assert!(
            !kill_output.status.success(),
            "Process {} should have been killed and reaped on shutdown",
            pid
        );
    }

    fn project_hook(root: &str, command: &str) -> HookConfig {
        HookConfig {
            event: HookEvent::PreToolUse,
            matcher: None,
            command: command.to_string(),
            timeout: Some(5),
            source: CommandSource::ProjectRefact(PathBuf::from(root)),
        }
    }

    #[test]
    fn test_hooks_surface_names_the_hook_that_denied_and_respects_deny_precedence() {
        let configs = vec![
            make_hook_config(HookEvent::PreToolUse, None, "allow.sh"),
            make_hook_config(HookEvent::PreToolUse, None, "block.sh"),
            make_hook_config(HookEvent::PreToolUse, None, "rewrite.sh"),
        ];
        let outcomes = vec![
            ControlOutcome::Allow,
            ControlOutcome::Deny("no rm".to_string()),
            ControlOutcome::Rewrite(serde_json::json!({"command": "ls"})),
        ];
        let decision = resolve_control_decision(&outcomes);
        assert_eq!(decision.decided_by, Some(1));
        assert_eq!(
            decision.verdict,
            ControlVerdict::Deny("no rm".to_string()),
            "deny outranks a rewrite whatever the order"
        );
        let record = control_activity(
            &HookEvent::PreToolUse,
            Some("shell"),
            &configs,
            &outcomes,
            &[],
            &decision,
        );
        assert_eq!(record.verdict, HookVerdictTag::Deny);
        assert_eq!(record.decided_by.as_deref(), Some("block.sh"));
        assert_eq!(record.reason.as_deref(), Some("no rm"));
        assert_eq!(record.tool.as_deref(), Some("shell"));
        assert_eq!(
            record
                .runs
                .iter()
                .map(|run| (run.command.as_str(), run.decided))
                .collect::<Vec<_>>(),
            vec![
                ("allow.sh", false),
                ("block.sh", true),
                ("rewrite.sh", false)
            ]
        );
    }

    #[test]
    fn test_hooks_surface_attributes_a_timeout_deny_to_the_hook_that_timed_out() {
        let configs = vec![
            make_hook_config(HookEvent::PreToolUse, None, "fast.sh"),
            make_hook_config(HookEvent::PreToolUse, None, "slow.sh"),
        ];
        let outcomes = vec![ControlOutcome::Allow, ControlOutcome::Timeout];
        let decision = resolve_control_decision(&outcomes);
        let record = control_activity(
            &HookEvent::PreToolUse,
            None,
            &configs,
            &outcomes,
            &[],
            &decision,
        );
        assert_eq!(record.verdict, HookVerdictTag::Deny);
        assert_eq!(record.decided_by.as_deref(), Some("slow.sh"));
        assert_eq!(record.runs[1].outcome, HookOutcomeTag::Timeout);
    }

    #[test]
    fn test_hooks_surface_allow_names_no_decider() {
        let configs = vec![make_hook_config(HookEvent::PreToolUse, None, "ok.sh")];
        let outcomes = vec![ControlOutcome::Allow];
        let decision = resolve_control_decision(&outcomes);
        assert_eq!(decision.decided_by, None);
        let record = control_activity(
            &HookEvent::PreToolUse,
            None,
            &configs,
            &outcomes,
            &[],
            &decision,
        );
        assert_eq!(record.verdict, HookVerdictTag::Allow);
        assert_eq!(record.decided_by, None);
        assert!(!record.runs[0].decided);
    }

    #[test]
    fn test_hooks_surface_shows_an_untrusted_project_hook_as_never_executed() {
        let configs = vec![
            make_hook_config(HookEvent::PreToolUse, None, "global.sh"),
            project_hook("/untrusted", "project.sh"),
            HookConfig {
                source: CommandSource::InstalledPlugin("vendor".to_string()),
                ..make_hook_config(HookEvent::SessionStart, None, "plugin.sh")
            },
        ];
        let surface = build_hook_surface(&configs, &HooksConfig::default(), Vec::new());
        assert_eq!(surface.registered, 3);
        assert_eq!(surface.runnable, 1);
        assert_eq!(surface.blocked, 2);

        let untrusted = surface
            .registrations
            .iter()
            .find(|entry| entry.command == "project.sh")
            .unwrap();
        assert_eq!(untrusted.trust, HookTrust::UntrustedProject);
        assert!(!untrusted.runs);
        assert!(untrusted
            .blocked_reason
            .as_deref()
            .unwrap()
            .contains("never executed"));

        let plugin = surface
            .registrations
            .iter()
            .find(|entry| entry.command == "plugin.sh")
            .unwrap();
        assert_eq!(plugin.trust, HookTrust::InstalledPlugin);
        assert!(!plugin.runs);

        let global = surface
            .registrations
            .iter()
            .find(|entry| entry.command == "global.sh")
            .unwrap();
        assert_eq!(global.trust, HookTrust::Global);
        assert!(global.runs);
        assert_eq!(global.blocked_reason, None);
        assert!(!surface
            .recent
            .iter()
            .any(|record| record.runs.iter().any(|run| run.command == "project.sh")));
    }

    #[test]
    fn test_hooks_surface_marks_a_trusted_project_hook_as_running() {
        let configs = vec![project_hook("/trusted", "project.sh")];
        let hooks = HooksConfig {
            trusted_projects: vec!["/trusted".to_string()],
            ..HooksConfig::default()
        };
        let surface = build_hook_surface(&configs, &hooks, Vec::new());
        assert_eq!(surface.registrations[0].trust, HookTrust::TrustedProject);
        assert!(surface.registrations[0].runs);
        assert_eq!(surface.runnable, 1);
    }

    #[test]
    fn test_hooks_surface_is_empty_when_nothing_is_registered() {
        let surface = build_hook_surface(&[], &HooksConfig::default(), Vec::new());
        assert_eq!(surface.registered, 0);
        assert_eq!(surface.runnable, 0);
        assert_eq!(surface.blocked, 0);
        assert_eq!(surface.denials, 0);
        assert!(surface.registrations.is_empty());
        assert!(surface.recent.is_empty());
        assert!(surface.enabled);
    }

    #[test]
    fn test_hooks_surface_counts_recursion_and_refused_rewrites_as_denials() {
        let error = check_hook_recursion(&HookEvent::PreToolUse, 2, 2).unwrap_err();
        let recursion = recursion_activity(&HookEvent::PreToolUse, Some("shell"), &error);
        assert_eq!(recursion.verdict, HookVerdictTag::RecursionBlocked);
        assert!(recursion.reason.unwrap().contains("max_recursion_depth"));
        let refused = revalidation_activity("shell", Some("outside authority".to_string()));
        assert_eq!(refused.verdict, HookVerdictTag::RewriteRefused);
        let accepted = revalidation_activity("shell", None);
        assert_eq!(accepted.verdict, HookVerdictTag::RewriteAccepted);
        let recursion = recursion_activity(&HookEvent::PreToolUse, Some("shell"), &error);
        let surface = build_hook_surface(
            &[],
            &HooksConfig::default(),
            vec![recursion, refused, accepted],
        );
        assert_eq!(surface.denials, 2);
    }

    #[test]
    fn test_hook_activity_log_is_bounded_and_newest_first() {
        let log = HookActivityLog::new(2);
        assert!(log.recent(10).is_empty());
        for command in ["one", "two", "three"] {
            log.record(revalidation_activity(command, None));
        }
        let recent = log.recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].tool.as_deref(), Some("three"));
        assert_eq!(recent[1].tool.as_deref(), Some("two"));
        assert_eq!(log.recent(1).len(), 1);
    }

    #[test]
    fn test_observation_activity_never_claims_a_verdict() {
        let configs = vec![make_hook_config(HookEvent::SessionStart, None, "watch.sh")];
        let record = observation_activity(
            &HookEvent::SessionStart,
            None,
            &configs,
            &[ObservationOutcome::Failed("boom".to_string())],
            &[],
        );
        assert_eq!(record.verdict, HookVerdictTag::Observed);
        assert_eq!(record.decided_by, None);
        assert!(!record.runs[0].decided);
        assert_eq!(record.runs[0].outcome, HookOutcomeTag::Failed);
    }

    #[test]
    fn test_revalidate_rewritten_tool_call_records_both_outcomes() {
        let log = HookActivityLog::new(8);
        let authority = ToolsPolicy::Only(vec!["cat".to_string()]);
        let refused = revalidate_rewritten_arguments(
            "shell",
            &serde_json::json!({"command": "ls"}),
            Some(&authority),
        );
        log.record(revalidation_activity(
            "shell",
            refused.as_ref().err().map(ToString::to_string),
        ));
        let accepted = revalidate_rewritten_arguments(
            "cat",
            &serde_json::json!({"paths": "a.rs"}),
            Some(&authority),
        );
        log.record(revalidation_activity(
            "cat",
            accepted.as_ref().err().map(ToString::to_string),
        ));
        let recent = log.recent(10);
        assert_eq!(recent[0].verdict, HookVerdictTag::RewriteAccepted);
        assert_eq!(recent[1].verdict, HookVerdictTag::RewriteRefused);
        assert!(recent[1].reason.as_ref().unwrap().contains("authority"));
    }

    #[test]
    fn test_is_global_source() {
        assert!(is_global_source(&CommandSource::GlobalClaude));
        assert!(is_global_source(&CommandSource::GlobalRefact));
        assert!(!is_global_source(&CommandSource::ProjectClaude(
            PathBuf::from("/p")
        )));
        assert!(!is_global_source(&CommandSource::ProjectRefact(
            PathBuf::from("/p")
        )));
    }

    #[test]
    fn test_is_project_source() {
        assert!(is_project_source(&CommandSource::ProjectRefact(
            PathBuf::from("/p")
        )));
        assert!(is_project_source(&CommandSource::ProjectClaude(
            PathBuf::from("/p")
        )));
        assert!(!is_project_source(&CommandSource::GlobalRefact));
        assert!(!is_project_source(&CommandSource::GlobalClaude));
    }

    #[test]
    fn test_hooks_config_default_trusts_no_projects() {
        assert!(HooksConfig::default().trusted_projects.is_empty());
        assert!(HooksConfig::default().enabled);
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial(permission_decision_hooks)]
    async fn permission_decision_deny_names_hook_and_records_single_event() {
        let workspace = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let app = test_app(config.path(), workspace.path(), true).await;
        let marker = workspace.path().join("permission-hook-count");
        permission_config(
            config.path(),
            &format!("echo blocked >&2; touch '{}'; exit 2", marker.display()),
        );
        let request = PermissionDecisionRequest {
            tool_name: "shell".into(),
            tool_input: serde_json::json!({"command":"touch forbidden"}),
            accepted: true,
            source: "user_confirmation".into(),
            request_id: None,
            tool_call_id: "call".into(),
            reason_type: "confirmation".into(),
        };
        let result = apply_permission_decision(
            app.clone(),
            "session".into(),
            workspace.path().to_string_lossy().into_owned(),
            request,
        )
        .await;
        assert!(!result.accepted);
        assert!(result
            .denial_reason
            .as_deref()
            .unwrap()
            .contains("PermissionDecision hook"));
        assert!(marker.exists());
        assert_eq!(
            hook_activity()
                .recent(10)
                .iter()
                .filter(|entry| entry.event == "PermissionDecision")
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial(permission_decision_hooks)]
    async fn permission_decision_reject_is_not_widened_by_allow_hook_and_disabled_hooks_are_silent()
    {
        let workspace = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let app = test_app(config.path(), workspace.path(), true).await;
        permission_config(config.path(), "exit 0");
        let request = PermissionDecisionRequest {
            tool_name: "shell".into(),
            tool_input: serde_json::json!({"command":"echo no"}),
            accepted: false,
            source: "user_confirmation".into(),
            request_id: None,
            tool_call_id: "call".into(),
            reason_type: "confirmation".into(),
        };
        let result = apply_permission_decision(
            app.clone(),
            "session".into(),
            workspace.path().display().to_string(),
            request.clone(),
        )
        .await;
        assert!(!result.accepted);
        assert_eq!(
            hook_activity()
                .recent(5)
                .iter()
                .filter(|entry| entry.event == "PermissionDecision")
                .count(),
            1
        );
        let disabled_app = test_app(config.path(), workspace.path(), false).await;
        let result = apply_permission_decision(
            disabled_app,
            "session".into(),
            workspace.path().display().to_string(),
            request,
        )
        .await;
        assert!(!result.accepted);
        assert_eq!(
            hook_activity()
                .recent(5)
                .iter()
                .filter(|entry| entry.event == "PermissionDecision")
                .count(),
            1
        );
    }

    #[test]
    fn test_hook_limits_follow_settings() {
        let config = HooksConfig {
            enabled: true,
            trusted_projects: vec![],
            default_timeout_secs: 7,
            max_recursion_depth: 3,
            max_output_bytes: 4096,
        };
        let limits = HookLimits::from(&config);
        assert_eq!(limits.default_timeout_secs, 7);
        assert_eq!(limits.max_recursion_depth, 3);
        assert_eq!(limits.max_output_bytes, 4096);
    }

    #[test]
    fn test_hook_source_allowed_global_always_runs() {
        let none: Vec<String> = vec![];
        assert!(hook_source_allowed(&CommandSource::GlobalRefact, &none));
        assert!(hook_source_allowed(&CommandSource::GlobalClaude, &none));
    }

    #[test]
    fn test_hook_source_allowed_project_requires_trust_entry() {
        let refact = CommandSource::ProjectRefact(PathBuf::from("/myproject"));
        let claude = CommandSource::ProjectClaude(PathBuf::from("/myproject"));
        assert!(!hook_source_allowed(&refact, &[]));
        assert!(!hook_source_allowed(&claude, &[]));
        assert!(!hook_source_allowed(&refact, &["/other".to_string()]));
        let trusted = vec!["/myproject".to_string()];
        assert!(hook_source_allowed(&refact, &trusted));
        assert!(hook_source_allowed(&claude, &trusted));
    }

    #[test]
    fn test_hook_source_allowed_installed_plugin_not_executed() {
        let plugin = CommandSource::InstalledPlugin("p".to_string());
        assert!(!hook_source_allowed(&plugin, &[]));
        assert!(!hook_source_allowed(&plugin, &["/anything".to_string()]));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_untrusted_project_hook_is_not_executed_until_trusted() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("repo");
        tokio::fs::create_dir_all(&project_root).await.unwrap();
        let sentinel = tmp.path().join("hook_ran.txt");
        let command = format!("touch '{}'", sentinel.to_str().unwrap());
        let config = HookConfig {
            event: HookEvent::PreToolUse,
            matcher: None,
            command,
            timeout: Some(5),
            source: CommandSource::ProjectRefact(project_root.clone()),
        };
        let payload = make_payload(&HookEvent::PreToolUse, Some("shell"));

        let untrusted: Vec<String> = vec![];
        assert!(!hook_source_allowed(&config.source, &untrusted));
        let outcomes: Vec<ControlOutcome> = if hook_source_allowed(&config.source, &untrusted) {
            vec![evaluate(&config, &payload).await]
        } else {
            vec![]
        };
        assert_eq!(resolve_control_verdict(&outcomes), ControlVerdict::Allow);
        assert!(
            !sentinel.exists(),
            "an untrusted project hook must not have been executed"
        );

        let trusted = vec![project_root.to_string_lossy().into_owned()];
        assert!(hook_source_allowed(&config.source, &trusted));
        if hook_source_allowed(&config.source, &trusted) {
            evaluate(&config, &payload).await;
        }
        assert!(
            sentinel.exists(),
            "a trusted project hook should have been executed"
        );
    }

    #[tokio::test]
    async fn test_hooks_cache_project_separation() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();

        tokio::fs::write(
            tmp1.path().join("hooks.yaml"),
            "hooks:\n  SessionStart:\n    - hooks:\n        - type: command\n          command: cmd_project_a",
        ).await.unwrap();

        tokio::fs::write(
            tmp2.path().join("hooks.yaml"),
            "hooks:\n  SessionEnd:\n    - hooks:\n        - type: command\n          command: cmd_project_b",
        ).await.unwrap();

        let ext_dirs1 = ExtDirs {
            global_dirs: vec![tmp1.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![PathBuf::from("/project/a")],
        };
        let ext_dirs2 = ExtDirs {
            global_dirs: vec![tmp2.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![PathBuf::from("/project/b")],
        };

        let hooks1 = get_compiled_hooks_from_ext_dirs(&ext_dirs1).await;
        let hooks2 = get_compiled_hooks_from_ext_dirs(&ext_dirs2).await;

        assert!(hooks1.iter().any(|h| h.config.command == "cmd_project_a"));
        assert!(!hooks1.iter().any(|h| h.config.command == "cmd_project_b"));
        assert!(hooks2.iter().any(|h| h.config.command == "cmd_project_b"));
        assert!(!hooks2.iter().any(|h| h.config.command == "cmd_project_a"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_hook_output_is_bounded_by_settings() {
        let config = make_hook_config(HookEvent::PreToolUse, None, "yes | head -c 102400");
        let payload = make_payload(&HookEvent::PreToolUse, Some("shell"));
        let limits = HookLimits {
            default_timeout_secs: 5,
            max_output_bytes: 2048,
            max_recursion_depth: 2,
        };
        match run_single_hook(&config, &payload, &limits, None).await {
            RawHookOutcome::Completed { stdout, .. } => {
                assert!(
                    stdout.len() <= 2048,
                    "output should respect the configured cap: {} > 2048",
                    stdout.len()
                );
            }
            other => panic!("Expected Completed, got {:?}", other),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_hook_stderr_is_bounded_by_settings() {
        let config = make_hook_config(
            HookEvent::PreToolUse,
            None,
            "yes | head -c 102400 >&2; exit 2",
        );
        let payload = make_payload(&HookEvent::PreToolUse, Some("shell"));
        let limits = HookLimits {
            default_timeout_secs: 5,
            max_output_bytes: 2048,
            max_recursion_depth: 2,
        };
        match run_single_hook(&config, &payload, &limits, None).await {
            RawHookOutcome::Completed { stderr, .. } => {
                assert!(stderr.len() <= 2048, "stderr should respect the cap");
            }
            other => panic!("Expected Completed, got {:?}", other),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_hook_receives_depth_in_environment_and_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("depth.txt");
        let command = format!(
            "printf '%s' \"$REFACT_HOOK_DEPTH\" > '{}'",
            out.to_str().unwrap()
        );
        let config = make_hook_config(HookEvent::PreToolUse, None, &command);
        let payload = make_payload(&HookEvent::PreToolUse, Some("shell")).with_depth(1);
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["hook_depth"], 1);
        evaluate(&config, &payload).await;
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "1");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_control_hooks_run_in_parallel() {
        let hooks = vec![
            make_hook_config(HookEvent::PreToolUse, None, "sleep 1"),
            make_hook_config(HookEvent::PreToolUse, None, "sleep 1"),
            make_hook_config(HookEvent::PreToolUse, None, "sleep 1"),
        ];
        let payload = make_payload(&HookEvent::PreToolUse, None);
        let start = std::time::Instant::now();
        let futs: Vec<_> = hooks.iter().map(|c| evaluate(c, &payload)).collect();
        let outcomes = futures::future::join_all(futs).await;
        assert_eq!(outcomes.len(), 3);
        assert!(
            start.elapsed().as_secs() < 3,
            "hooks should run in parallel, took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn test_parallel_deny_propagates() {
        let hooks = vec![
            make_hook_config(HookEvent::PreToolUse, None, "echo success"),
            make_hook_config(
                HookEvent::PreToolUse,
                None,
                &stderr_exit_command("blocked", 2),
            ),
            make_hook_config(HookEvent::PreToolUse, None, "echo success2"),
        ];
        let payload = make_payload(&HookEvent::PreToolUse, None);
        let futs: Vec<_> = hooks.iter().map(|c| evaluate(c, &payload)).collect();
        let outcomes = futures::future::join_all(futs).await;
        match resolve_control_verdict(&outcomes) {
            ControlVerdict::Deny(reason) => assert!(reason.contains("blocked")),
            other => panic!("Expected Deny, got {:?}", other),
        }
    }

    #[test]
    fn test_privacy_filter_redacts_blocked_paths_in_tool_input() {
        let privacy = Arc::new(PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: vec![],
                blocked: vec!["*.pem".to_string()],
            },
            loaded_ts: 0,
        });
        let mut value = serde_json::json!({
            "paths": ["src/main.rs", "keys/server.pem"],
            "nested": {"file": "secret.pem"}
        });
        redact_privacy_blocked_paths(&mut value, &privacy);
        assert_eq!(value["paths"][0], "src/main.rs");
        assert_eq!(value["paths"][1], PRIVACY_REDACTED);
        assert_eq!(value["nested"]["file"], PRIVACY_REDACTED);
    }

    #[test]
    fn test_privacy_filter_leaves_allowed_paths_alone() {
        let privacy = Arc::new(PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: vec![],
                blocked: vec![],
            },
            loaded_ts: 0,
        });
        let mut value = serde_json::json!({"file": "src/main.rs"});
        redact_privacy_blocked_paths(&mut value, &privacy);
        assert_eq!(value["file"], "src/main.rs");
    }

    #[test]
    fn test_extra_payload_map_is_available_for_new_events() {
        let mut payload = make_payload(&HookEvent::PermissionDecision, Some("shell"));
        payload
            .extra
            .insert("decision".to_string(), serde_json::json!("denied"));
        payload
            .extra
            .insert("rule".to_string(), serde_json::json!("deny-rm"));
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["hook_event_name"], "PermissionDecision");
        assert_eq!(json["decision"], "denied");
        assert_eq!(json["rule"], "deny-rm");
        let _ = HashMap::<String, serde_json::Value>::new();
    }
}
