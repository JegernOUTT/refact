use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AMutex;
use refact_tool_api::{
    classify_command, default_catalogue, extract_command_segments, first_matching_rule,
    structural_flags, CommandSegments, MatchConfirmDeny, MatchConfirmDenyResult, RiskContext,
    RiskEntry, RiskLevel,
};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::files_correction::{get_project_dirs, get_unscoped_project_dirs};
use crate::global_context::GlobalContext;
use crate::tools::file_edit::auxiliary::active_execution_scope;
use crate::tools::shell_gate_llm::{validate_command, LlmVerdict};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalMode {
    Strict,
    Balanced,
    Permissive,
    Yolo,
}

impl Default for ApprovalMode {
    fn default() -> Self {
        Self::Balanced
    }
}

impl ApprovalMode {
    pub fn ask_threshold(&self) -> Option<RiskLevel> {
        match self {
            Self::Strict => Some(RiskLevel::Low),
            Self::Balanced => Some(RiskLevel::Medium),
            Self::Permissive => Some(RiskLevel::High),
            Self::Yolo => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellLlmAuthority {
    AskOnly,
    AskAndAllow,
}

impl Default for ShellLlmAuthority {
    fn default() -> Self {
        Self::AskOnly
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellLlmOnFailure {
    Pass,
    Ask,
}

impl Default for ShellLlmOnFailure {
    fn default() -> Self {
        Self::Pass
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ShellLlmValidation {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub authority: ShellLlmAuthority,
    #[serde(default = "default_llm_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub on_failure: ShellLlmOnFailure,
    #[serde(default = "default_true")]
    pub cache_per_chat: bool,
}

impl Default for ShellLlmValidation {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::new(),
            authority: ShellLlmAuthority::AskOnly,
            timeout_secs: default_llm_timeout(),
            on_failure: ShellLlmOnFailure::Pass,
            cache_per_chat: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ShellExecutionDefaults {
    #[serde(default = "default_foreground_timeout")]
    pub foreground_timeout_secs: u64,
    #[serde(default = "default_output_limit")]
    pub output_limit_lines: usize,
}

impl Default for ShellExecutionDefaults {
    fn default() -> Self {
        Self {
            foreground_timeout_secs: default_foreground_timeout(),
            output_limit_lines: default_output_limit(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RiskEntryOverride {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub level: Option<RiskLevel>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ShellGatePolicy {
    #[serde(default)]
    pub mode: ApprovalMode,
    #[serde(default = "default_deny_rules")]
    pub deny: Vec<String>,
    #[serde(default = "default_ask_rules")]
    pub ask: Vec<String>,
    #[serde(default = "default_allow_rules")]
    pub allow: Vec<String>,
    #[serde(default = "default_true")]
    pub trust_caller_confirmation: bool,
    #[serde(default)]
    pub llm_validation: ShellLlmValidation,
    #[serde(default)]
    pub execution: ShellExecutionDefaults,
    #[serde(default)]
    pub catalogue_overrides: Vec<RiskEntryOverride>,
    #[serde(skip)]
    pub catalogue: Vec<RiskEntry>,
}

impl Default for ShellGatePolicy {
    fn default() -> Self {
        Self {
            mode: ApprovalMode::default(),
            deny: default_deny_rules(),
            ask: default_ask_rules(),
            allow: default_allow_rules(),
            trust_caller_confirmation: true,
            llm_validation: ShellLlmValidation::default(),
            execution: ShellExecutionDefaults::default(),
            catalogue_overrides: Vec::new(),
            catalogue: default_catalogue(),
        }
    }
}

impl ShellGatePolicy {
    pub fn rebuild_catalogue(&mut self) {
        let mut catalogue = default_catalogue();
        for entry_override in &self.catalogue_overrides {
            let Some(index) = catalogue
                .iter()
                .position(|entry| entry.id == entry_override.id)
            else {
                tracing::warn!(id = %entry_override.id, "unknown shell risk catalogue override");
                continue;
            };
            if entry_override.enabled == Some(false) {
                catalogue.remove(index);
            } else if let Some(level) = entry_override.level {
                catalogue[index].level = level;
            }
        }
        self.catalogue = catalogue;
    }
}

fn default_true() -> bool {
    true
}

fn default_llm_timeout() -> u64 {
    8
}

fn default_foreground_timeout() -> u64 {
    10
}

fn default_output_limit() -> usize {
    40
}

pub struct GateContext {
    pub workspace_roots: Vec<PathBuf>,
    pub needs_confirmation: bool,
}

pub struct GateOutcome {
    pub decision: MatchConfirmDeny,
    pub layer: &'static str,
    pub reason: String,
    pub risk_level: Option<RiskLevel>,
}

pub fn evaluate(command: &str, policy: &ShellGatePolicy, ctx: &GateContext) -> GateOutcome {
    let segments = extract_command_segments(command);
    if let Some(rule) = first_matching_rule(command, &segments, &policy.deny) {
        return outcome(
            command,
            MatchConfirmDenyResult::DENY,
            &rule,
            "deny-rule",
            format!("Denied by rule `{rule}`."),
            None,
        );
    }
    if !segments.parse_ok {
        if let Some(rule) = deny_on_unparsable(command, &segments, &policy.deny) {
            return outcome(
                command,
                MatchConfirmDenyResult::DENY,
                &rule,
                "deny-rule",
                format!("Denied by rule `{rule}`."),
                None,
            );
        }
        if policy.mode != ApprovalMode::Yolo {
            return outcome(
                command,
                MatchConfirmDenyResult::CONFIRMATION,
                "unparsable-command",
                "unparsable",
                "Command could not be parsed safely.".into(),
                None,
            );
        }
    }
    if let Some(rule) = first_matching_rule(command, &segments, &policy.allow) {
        return outcome(
            command,
            MatchConfirmDenyResult::PASS,
            &rule,
            "allow-rule",
            format!("Allowed by rule `{rule}`."),
            None,
        );
    }
    if policy.trust_caller_confirmation && ctx.needs_confirmation {
        return outcome(
            command,
            MatchConfirmDenyResult::CONFIRMATION,
            "caller-requested",
            "caller-requested",
            "The model asked for confirmation before running this.".into(),
            None,
        );
    }
    if policy.mode == ApprovalMode::Yolo {
        return outcome(
            command,
            MatchConfirmDenyResult::PASS,
            "yolo-mode",
            "yolo",
            "Allowed by yolo mode.".into(),
            None,
        );
    }
    if let Some(flag) = structural_flags(&segments).into_iter().next() {
        return outcome(
            command,
            MatchConfirmDenyResult::CONFIRMATION,
            flag,
            "structural",
            format!("Command has the structural risk `{flag}`."),
            None,
        );
    }
    if let Some(rule) = first_matching_rule(command, &segments, &policy.ask) {
        return outcome(
            command,
            MatchConfirmDenyResult::CONFIRMATION,
            &rule,
            "ask-rule",
            format!("Confirmation required by rule `{rule}`."),
            None,
        );
    }
    let risk_context = RiskContext {
        workspace_roots: ctx.workspace_roots.clone(),
    };
    if let (Some(threshold), Some(finding)) = (
        policy.mode.ask_threshold(),
        classify_command(&segments, &policy.catalogue, &risk_context),
    ) {
        if finding.level >= threshold {
            return outcome(
                command,
                MatchConfirmDenyResult::CONFIRMATION,
                &format!("risk:{}", finding.entry_id),
                "risk",
                finding.reason,
                Some(finding.level),
            );
        }
    }
    outcome(
        command,
        MatchConfirmDenyResult::PASS,
        "",
        "default",
        "No approval rule matched.".into(),
        None,
    )
}

pub async fn evaluate_with_llm(
    gcx: Arc<GlobalContext>,
    command: &str,
    policy: &ShellGatePolicy,
    ctx: &GateContext,
    chat_id: &str,
) -> GateOutcome {
    let static_outcome = evaluate(command, policy, ctx);
    if !policy.llm_validation.enabled
        || static_outcome.decision.result == MatchConfirmDenyResult::DENY
    {
        return static_outcome;
    }
    let may_allow = static_outcome.decision.result == MatchConfirmDenyResult::CONFIRMATION
        && static_outcome.layer == "risk"
        && policy.llm_validation.authority == ShellLlmAuthority::AskAndAllow;
    if static_outcome.decision.result != MatchConfirmDenyResult::PASS && !may_allow {
        return static_outcome;
    }
    let verdict = validate_command(gcx, command, chat_id, &policy.llm_validation).await;
    apply_llm_verdict(static_outcome, verdict, &policy.llm_validation)
}

fn apply_llm_verdict(
    mut static_outcome: GateOutcome,
    verdict: Option<LlmVerdict>,
    cfg: &ShellLlmValidation,
) -> GateOutcome {
    if !cfg.enabled || static_outcome.decision.result == MatchConfirmDenyResult::DENY {
        return static_outcome;
    }
    match static_outcome.decision.result {
        MatchConfirmDenyResult::PASS => match verdict {
            Some(verdict) if verdict.ask => {
                static_outcome.decision.result = MatchConfirmDenyResult::CONFIRMATION;
                static_outcome.decision.rule = "llm".to_string();
                static_outcome.layer = "llm";
                static_outcome.reason = verdict.reason;
            }
            None if cfg.on_failure == ShellLlmOnFailure::Ask => {
                static_outcome.decision.result = MatchConfirmDenyResult::CONFIRMATION;
                static_outcome.decision.rule = "llm-unavailable".to_string();
                static_outcome.layer = "llm-unavailable";
                static_outcome.reason = "LLM command validation was unavailable.".to_string();
            }
            _ => {}
        },
        MatchConfirmDenyResult::CONFIRMATION
            if static_outcome.layer == "risk"
                && cfg.authority == ShellLlmAuthority::AskAndAllow =>
        {
            if let Some(verdict) = verdict.filter(|verdict| !verdict.ask) {
                static_outcome.decision.result = MatchConfirmDenyResult::PASS;
                static_outcome.decision.rule = "llm-allowed".to_string();
                static_outcome.layer = "llm-allowed";
                static_outcome.reason = verdict.reason;
            }
        }
        _ => {}
    }
    static_outcome
}

fn outcome(
    command: &str,
    result: MatchConfirmDenyResult,
    rule: &str,
    layer: &'static str,
    reason: String,
    risk_level: Option<RiskLevel>,
) -> GateOutcome {
    GateOutcome {
        decision: MatchConfirmDeny {
            result,
            command: command.to_string(),
            rule: rule.to_string(),
        },
        layer,
        reason,
        risk_level,
    }
}

fn deny_on_unparsable(
    command: &str,
    segments: &CommandSegments,
    deny: &[String],
) -> Option<String> {
    deny.iter()
        .find(|rule| {
            literal_deny_pattern(rule).is_some_and(|pattern| {
                first_matching_rule(command, segments, &[format!("raw:*{pattern}*")]).is_some()
            })
        })
        .cloned()
}

fn literal_deny_pattern(rule: &str) -> Option<&str> {
    if rule.starts_with("re:") || rule.starts_with("raw:") {
        return None;
    }
    Some(
        rule.strip_prefix("exec:")
            .or_else(|| rule.strip_prefix("argv:"))
            .unwrap_or(rule),
    )
}

pub fn default_deny_rules() -> Vec<String> {
    vec!["sudo".to_string(), "doas".to_string()]
}

pub fn default_ask_rules() -> Vec<String> {
    vec!["raw::(){ :|:& };:".to_string()]
}

pub fn default_allow_rules() -> Vec<String> {
    Vec::new()
}

pub fn parse_needs_confirmation(args: &HashMap<String, Value>) -> bool {
    args.get("needs_confirmation")
        .and_then(refact_tool_api::coerce_bool)
        .unwrap_or(false)
}

pub async fn gate_tool_call(
    ccx: Arc<AMutex<AtCommandsContext>>,
    command: String,
    args: &HashMap<String, Value>,
) -> Result<MatchConfirmDeny, String> {
    let (gcx, execution_scope, chat_id) = {
        let ccx_lock = ccx.lock().await;
        (
            ccx_lock.app.gcx.clone(),
            ccx_lock.execution_scope.clone(),
            ccx_lock.chat_id.clone(),
        )
    };
    let workspace_roots = if let Some(scope) = active_execution_scope(execution_scope.as_ref()) {
        vec![scope.effective_root().to_path_buf()]
    } else {
        get_unscoped_project_dirs(gcx.clone()).await
    };
    let policy = load_policy(gcx.clone()).await;
    let outcome = evaluate_with_llm(
        gcx.clone(),
        &command,
        &policy,
        &GateContext {
            workspace_roots,
            needs_confirmation: parse_needs_confirmation(args),
        },
        &chat_id,
    )
    .await;
    append_audit(gcx, AuditEntry::from_outcome(chat_id, command, &outcome)).await;
    Ok(outcome.decision)
}

pub fn policy_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".refact").join("shell_policy.yaml")
}

fn audit_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".refact").join("shell_audit.jsonl")
}

async fn project_dir(gcx: Arc<GlobalContext>) -> Option<PathBuf> {
    get_project_dirs(gcx).await.into_iter().next()
}

pub async fn load_policy(gcx: Arc<GlobalContext>) -> ShellGatePolicy {
    let Some(project_dir) = project_dir(gcx).await else {
        return ShellGatePolicy::default();
    };
    let path = policy_path(&project_dir);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ShellGatePolicy::default();
        }
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "failed to read shell policy");
            return ShellGatePolicy::default();
        }
    };
    match serde_yaml::from_str::<ShellGatePolicy>(&text) {
        Ok(mut policy) => {
            policy.rebuild_catalogue();
            policy
        }
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "failed to parse shell policy");
            ShellGatePolicy::default()
        }
    }
}

pub async fn save_policy(gcx: Arc<GlobalContext>, policy: &ShellGatePolicy) -> Result<(), String> {
    let project_dir = project_dir(gcx)
        .await
        .ok_or_else(|| "no active project".to_string())?;
    let path = policy_path(&project_dir);
    let parent = path
        .parent()
        .ok_or_else(|| "invalid shell policy path".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| error.to_string())?;
    let yaml = serde_yaml::to_string(policy).map_err(|error| error.to_string())?;
    tokio::fs::write(path, yaml)
        .await
        .map_err(|error| error.to_string())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuditEntry {
    pub ts_ms: u64,
    pub chat_id: String,
    pub command: String,
    pub decision: String,
    pub layer: String,
    pub rule: String,
    #[serde(default)]
    pub risk_level: Option<RiskLevel>,
}

impl AuditEntry {
    pub fn from_outcome(chat_id: String, command: String, outcome: &GateOutcome) -> Self {
        let decision = match outcome.decision.result {
            MatchConfirmDenyResult::PASS => "pass",
            MatchConfirmDenyResult::CONFIRMATION => "confirmation",
            MatchConfirmDenyResult::DENY => "deny",
        };
        Self {
            ts_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            chat_id,
            command,
            decision: decision.to_string(),
            layer: outcome.layer.to_string(),
            rule: outcome.decision.rule.clone(),
            risk_level: outcome.risk_level,
        }
    }
}

pub async fn append_audit(gcx: Arc<GlobalContext>, entry: AuditEntry) {
    if let Err(error) = append_audit_inner(gcx, entry).await {
        tracing::warn!(%error, "failed to append shell audit entry");
    }
}

async fn append_audit_inner(gcx: Arc<GlobalContext>, entry: AuditEntry) -> Result<(), String> {
    let project_dir = project_dir(gcx)
        .await
        .ok_or_else(|| "no active project".to_string())?;
    let path = audit_path(&project_dir);
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .map_err(|e| e.to_string())?;
    if tokio::fs::metadata(&path)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0)
        > 1024 * 1024
    {
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| e.to_string())?;
        let lines: Vec<&str> = text.lines().rev().take(500).collect();
        let retained = lines.into_iter().rev().collect::<Vec<_>>().join("\n") + "\n";
        tokio::fs::write(&path, retained)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| e.to_string())?;
    let mut line = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
    line.push(b'\n');
    file.write_all(&line).await.map_err(|e| e.to_string())
}

pub async fn read_audit(gcx: Arc<GlobalContext>, limit: usize) -> Vec<AuditEntry> {
    let Some(project_dir) = project_dir(gcx).await else {
        return Vec::new();
    };
    let path = audit_path(&project_dir);
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => text
            .lines()
            .rev()
            .filter_map(|line| serde_json::from_str(line).ok())
            .take(limit)
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "failed to read shell audit log");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge_outcome(result: MatchConfirmDenyResult, layer: &'static str) -> GateOutcome {
        outcome(
            "command",
            result,
            "rule",
            layer,
            "static reason".to_string(),
            None,
        )
    }

    fn enabled_llm() -> ShellLlmValidation {
        ShellLlmValidation {
            enabled: true,
            ..ShellLlmValidation::default()
        }
    }

    fn verdict(ask: bool) -> Option<LlmVerdict> {
        Some(LlmVerdict {
            ask,
            reason: "LLM reason".to_string(),
        })
    }

    #[test]
    fn disabled_llm_leaves_all_outcomes_untouched() {
        for (result, layer) in [
            (MatchConfirmDenyResult::PASS, "default"),
            (MatchConfirmDenyResult::CONFIRMATION, "risk"),
            (MatchConfirmDenyResult::DENY, "deny-rule"),
        ] {
            let merged = apply_llm_verdict(
                merge_outcome(result.clone(), layer),
                verdict(true),
                &ShellLlmValidation::default(),
            );
            assert_eq!(merged.decision.result, result);
            assert_eq!(merged.layer, layer);
            assert_eq!(merged.reason, "static reason");
        }
    }

    #[test]
    fn llm_can_escalate_pass() {
        let merged = apply_llm_verdict(
            merge_outcome(MatchConfirmDenyResult::PASS, "default"),
            verdict(true),
            &enabled_llm(),
        );
        assert_eq!(merged.decision.result, MatchConfirmDenyResult::CONFIRMATION);
        assert_eq!(merged.layer, "llm");
        assert_eq!(merged.reason, "LLM reason");
    }

    #[test]
    fn llm_failure_policy_applies_to_pass() {
        let pass_cfg = enabled_llm();
        let passed = apply_llm_verdict(
            merge_outcome(MatchConfirmDenyResult::PASS, "default"),
            None,
            &pass_cfg,
        );
        assert_eq!(passed.decision.result, MatchConfirmDenyResult::PASS);

        let ask_cfg = ShellLlmValidation {
            on_failure: ShellLlmOnFailure::Ask,
            ..enabled_llm()
        };
        let asked = apply_llm_verdict(
            merge_outcome(MatchConfirmDenyResult::PASS, "default"),
            None,
            &ask_cfg,
        );
        assert_eq!(asked.decision.result, MatchConfirmDenyResult::CONFIRMATION);
        assert_eq!(asked.layer, "llm-unavailable");
    }

    #[test]
    fn ask_and_allow_can_downgrade_only_risk_confirmation() {
        let cfg = ShellLlmValidation {
            authority: ShellLlmAuthority::AskAndAllow,
            ..enabled_llm()
        };
        let allowed = apply_llm_verdict(
            merge_outcome(MatchConfirmDenyResult::CONFIRMATION, "risk"),
            verdict(false),
            &cfg,
        );
        assert_eq!(allowed.decision.result, MatchConfirmDenyResult::PASS);
        assert_eq!(allowed.layer, "llm-allowed");

        for layer in ["deny-rule", "structural", "caller-requested"] {
            let unchanged = apply_llm_verdict(
                merge_outcome(MatchConfirmDenyResult::CONFIRMATION, layer),
                verdict(false),
                &cfg,
            );
            assert_eq!(
                unchanged.decision.result,
                MatchConfirmDenyResult::CONFIRMATION
            );
            assert_eq!(unchanged.layer, layer);
        }
    }

    #[test]
    fn ask_only_cannot_downgrade_risk_confirmation() {
        let unchanged = apply_llm_verdict(
            merge_outcome(MatchConfirmDenyResult::CONFIRMATION, "risk"),
            verdict(false),
            &enabled_llm(),
        );
        assert_eq!(
            unchanged.decision.result,
            MatchConfirmDenyResult::CONFIRMATION
        );
        assert_eq!(unchanged.layer, "risk");
    }

    #[test]
    fn llm_never_modifies_deny() {
        let denied = apply_llm_verdict(
            merge_outcome(MatchConfirmDenyResult::DENY, "deny-rule"),
            verdict(false),
            &ShellLlmValidation {
                authority: ShellLlmAuthority::AskAndAllow,
                ..enabled_llm()
            },
        );
        assert_eq!(denied.decision.result, MatchConfirmDenyResult::DENY);
        assert_eq!(denied.layer, "deny-rule");
    }

    async fn test_gcx(project_dir: &Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .push(project_dir.to_path_buf());
        gcx
    }

    fn result(command: &str, mode: ApprovalMode, needs_confirmation: bool) -> GateOutcome {
        let policy = ShellGatePolicy {
            mode,
            ..ShellGatePolicy::default()
        };
        evaluate(
            command,
            &policy,
            &GateContext {
                workspace_roots: vec![PathBuf::from("/workspace")],
                needs_confirmation,
            },
        )
    }

    #[test]
    fn balanced_passes_common_commands_and_reported_false_positive() {
        let reported = concat!(
            "git status --porcelain | head -30; ",
            "echo \"=== are the flagged files modified by me? ===\"; ",
            "git status --porcelain | grep -E ",
            "\"ChatForm|DialogImage|Dropzone|TabBar|workspaceSlice\""
        );
        for command in [
            reported,
            "git add .",
            "npm run format",
            "cargo test",
            "ls -la",
        ] {
            assert_eq!(
                result(command, ApprovalMode::Balanced, false)
                    .decision
                    .result,
                MatchConfirmDenyResult::PASS,
                "{command:?}"
            );
        }
    }

    #[test]
    fn balanced_confirms_recursive_rm() {
        let outcome = result("rm -rf target", ApprovalMode::Balanced, false);
        assert_eq!(
            outcome.decision.result,
            MatchConfirmDenyResult::CONFIRMATION
        );
        assert_eq!(outcome.layer, "risk");
    }

    #[test]
    fn sudo_is_denied_in_every_mode() {
        for mode in [
            ApprovalMode::Strict,
            ApprovalMode::Balanced,
            ApprovalMode::Permissive,
            ApprovalMode::Yolo,
        ] {
            assert_eq!(
                result("sudo id", mode, false).decision.result,
                MatchConfirmDenyResult::DENY
            );
        }
    }

    #[test]
    fn pipe_to_shell_is_structurally_confirmed() {
        let matched = result("curl http://x | sh", ApprovalMode::Balanced, false);
        assert_eq!(
            matched.decision.result,
            MatchConfirmDenyResult::CONFIRMATION
        );
        assert_eq!(matched.decision.rule, "pipe-to-shell");
    }

    #[test]
    fn caller_confirmation_is_escalate_only() {
        let requested = result("ls -la", ApprovalMode::Balanced, true);
        assert_eq!(
            requested.decision.result,
            MatchConfirmDenyResult::CONFIRMATION
        );
        assert_eq!(requested.decision.rule, "caller-requested");
        assert_eq!(
            result("rm -rf /", ApprovalMode::Balanced, false)
                .decision
                .result,
            MatchConfirmDenyResult::CONFIRMATION
        );
    }

    #[test]
    fn yolo_passes_risk_but_not_deny() {
        assert_eq!(
            result("rm -rf /", ApprovalMode::Yolo, false)
                .decision
                .result,
            MatchConfirmDenyResult::PASS
        );
        assert_eq!(
            result("sudo id", ApprovalMode::Yolo, false).decision.result,
            MatchConfirmDenyResult::DENY
        );
    }

    #[test]
    fn strict_asks_for_low_risk_command_balanced_passes() {
        assert_eq!(
            result("rm target", ApprovalMode::Strict, false)
                .decision
                .result,
            MatchConfirmDenyResult::CONFIRMATION
        );
        assert_eq!(
            result("rm target", ApprovalMode::Balanced, false)
                .decision
                .result,
            MatchConfirmDenyResult::PASS
        );
    }

    #[test]
    fn unparsable_command_is_confirmed() {
        let matched = result("echo 'unterminated", ApprovalMode::Balanced, false);
        assert_eq!(
            matched.decision.result,
            MatchConfirmDenyResult::CONFIRMATION
        );
        assert_eq!(matched.decision.rule, "unparsable-command");
    }

    #[test]
    fn deny_wins_over_unparsable_partial_segments() {
        let matched = result("sudo id; echo 'unterminated", ApprovalMode::Balanced, false);
        assert_eq!(matched.decision.result, MatchConfirmDenyResult::DENY);
        assert_eq!(matched.decision.rule, "sudo");
    }

    #[test]
    fn overrides_change_level_and_disable_entry() {
        let defaults = default_catalogue();
        let changed_id = defaults[0].id.clone();
        let disabled_id = defaults[1].id.clone();
        let mut policy = ShellGatePolicy::default();
        policy.catalogue_overrides = vec![
            RiskEntryOverride {
                id: changed_id.clone(),
                level: Some(RiskLevel::Low),
                enabled: None,
            },
            RiskEntryOverride {
                id: disabled_id.clone(),
                level: None,
                enabled: Some(false),
            },
        ];
        policy.rebuild_catalogue();
        assert_eq!(
            policy
                .catalogue
                .iter()
                .find(|entry| entry.id == changed_id)
                .unwrap()
                .level,
            RiskLevel::Low
        );
        assert!(policy.catalogue.iter().all(|entry| entry.id != disabled_id));
    }

    #[tokio::test]
    async fn malformed_policy_falls_back_to_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = test_gcx(temp.path()).await;
        let path = policy_path(temp.path());
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, "mode: [not valid").await.unwrap();
        let policy = load_policy(gcx).await;
        assert_eq!(policy.mode, ApprovalMode::Balanced);
        assert_eq!(policy.catalogue, default_catalogue());
    }

    #[test]
    fn legacy_approval_memory_key_is_ignored() {
        let policy: ShellGatePolicy =
            serde_yaml::from_str("mode: balanced\nremember_approvals_per_chat: false\n").unwrap();
        assert_eq!(policy.mode, ApprovalMode::Balanced);
    }

    #[tokio::test]
    async fn saved_policy_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = test_gcx(temp.path()).await;
        let mut policy = ShellGatePolicy::default();
        policy.mode = ApprovalMode::Strict;
        policy.catalogue_overrides = vec![RiskEntryOverride {
            id: default_catalogue()[0].id.clone(),
            level: Some(RiskLevel::Low),
            enabled: None,
        }];
        policy.rebuild_catalogue();
        save_policy(gcx.clone(), &policy).await.unwrap();
        assert_eq!(load_policy(gcx).await, policy);
    }

    #[tokio::test]
    async fn audit_file_stays_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = test_gcx(temp.path()).await;
        for index in 0..650 {
            append_audit(
                gcx.clone(),
                AuditEntry {
                    ts_ms: index,
                    chat_id: "chat".to_string(),
                    command: "x".repeat(2048),
                    decision: "pass".to_string(),
                    layer: "default".to_string(),
                    rule: String::new(),
                    risk_level: None,
                },
            )
            .await;
        }
        let entries = read_audit(gcx, 1000).await;
        assert!(entries.len() <= 501);
        assert_eq!(entries.first().unwrap().ts_ms, 649);
    }
}
