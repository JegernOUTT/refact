use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock as ARwLock;
use uuid::Uuid;

use crate::buddy::actor::{make_runtime_event, redact_sensitive, validate_workflow_id};
use crate::buddy::diagnostics::{DiagnosticContext, DiagnosticSeverity};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::buddy::types::{BuddyActivity, BuddyThreadMeta};
use crate::call_validation::ChatMessage;
use crate::global_context::GlobalContext;

#[cfg_attr(not(test), allow(dead_code))]
pub const AUTONOMOUS_BUDDY_CHAT_SUBAGENT: &str = "buddy_autonomous_chat";
#[cfg_attr(not(test), allow(dead_code))]
pub const AUTONOMOUS_PROMPT_CAP_CHARS: usize = 8_000;
#[cfg_attr(not(test), allow(dead_code))]
pub const AUTONOMOUS_EVIDENCE_CAP_CHARS: usize = 24_000;

#[cfg_attr(not(test), allow(dead_code))]
const AUTONOMOUS_REDACTION_SCAN_MULTIPLIER: usize = 4;
#[cfg_attr(not(test), allow(dead_code))]
const AUTONOMOUS_REDACTION_SCAN_EXTRA_CHARS: usize = 4_096;
#[cfg_attr(not(test), allow(dead_code))]
const TRUNCATED_MARKER: &str = "\n...[truncated]";
const MAX_DIAGNOSTIC_EVIDENCE: usize = 20;
const MAX_SECURITY_FINDINGS: usize = 20;
const MAX_PATH_EVIDENCE: usize = 40;
const MAX_MANIFEST_WALK_FILES: usize = 5_000;
const MAX_UNTRACKED_CONTENT_BYTES: u64 = 64 * 1024;
const ERROR_DETECTIVE_WORKFLOW_ID: &str = "buddy_error_detective";
const SECURITY_WHISPERER_WORKFLOW_ID: &str = "buddy_security_whisperer";
const SETUP_COACH_WORKFLOW_ID: &str = "buddy_setup_coach";
const DEPENDENCY_RADAR_WORKFLOW_ID: &str = "buddy_dependency_radar";
const DOCS_GARDENER_WORKFLOW_ID: &str = "buddy_docs_gardener";
const ARCHITECTURE_DRIFT_WORKFLOW_ID: &str = "buddy_architecture_drift_watcher";

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutonomousBuddyChatSpec {
    pub workflow_id: String,
    pub title: String,
    pub prompt: String,
    pub evidence: String,
    pub signal_hash: String,
    pub icon: String,
    pub badge: String,
    pub priority: String,
}

#[cfg_attr(not(test), allow(dead_code))]
impl AutonomousBuddyChatSpec {
    pub fn new(
        workflow_id: impl Into<String>,
        title: impl Into<String>,
        prompt: impl Into<String>,
        evidence: impl Into<String>,
    ) -> Self {
        let workflow_id = workflow_id.into();
        let title = title.into();
        let prompt = prompt.into();
        let evidence = evidence.into();
        let signal_hash = default_signal_hash(&workflow_id, &title, &prompt, &evidence);
        Self {
            workflow_id,
            title,
            prompt,
            evidence,
            signal_hash,
            icon: String::new(),
            badge: String::new(),
            priority: "normal".to_string(),
        }
    }

    pub fn with_display(
        mut self,
        icon: impl Into<String>,
        badge: impl Into<String>,
        priority: impl Into<String>,
    ) -> Self {
        self.icon = icon.into();
        self.badge = badge.into();
        self.priority = priority.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct AutonomousLastResult {
    pub signal_hash: String,
    pub chat_id: String,
    pub completed_at: String,
}

#[cfg_attr(not(test), allow(dead_code))]
impl AutonomousLastResult {
    pub fn new(signal_hash: impl Into<String>, chat_id: impl Into<String>) -> Self {
        Self {
            signal_hash: signal_hash.into(),
            chat_id: chat_id.into(),
            completed_at: Utc::now().to_rfc3339(),
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn signal_hash<I, S>(parts: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut hasher = Sha256::new();
    for part in parts {
        let text = part.as_ref();
        hasher.update(text.len().to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(text.as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

#[cfg_attr(not(test), allow(dead_code))]
fn default_signal_hash(workflow_id: &str, title: &str, prompt: &str, evidence: &str) -> String {
    let prompt = redact_and_cap_prompt(prompt);
    let evidence = redact_and_cap_evidence(evidence);
    signal_hash([workflow_id, title, prompt.as_str(), evidence.as_str()])
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_last_autonomous_result(raw: Option<&str>) -> Option<AutonomousLastResult> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = serde_json::from_str::<AutonomousLastResult>(raw).ok()?;
    if parsed.signal_hash.is_empty() || parsed.chat_id.is_empty() || parsed.completed_at.is_empty()
    {
        return None;
    }
    Some(parsed)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn serialize_last_autonomous_result(result: &AutonomousLastResult) -> String {
    serde_json::json!({
        "signal_hash": result.signal_hash,
        "chat_id": result.chat_id,
        "completed_at": result.completed_at,
    })
    .to_string()
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn same_signal(ctx: &BuddyJobContext, hash: &str) -> bool {
    parse_last_autonomous_result(ctx.job_state.last_result.as_deref())
        .map(|last| last.signal_hash == hash)
        .unwrap_or(false)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn redact_and_cap_prompt(text: &str) -> String {
    redact_and_cap_text(text, AUTONOMOUS_PROMPT_CAP_CHARS)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn redact_and_cap_evidence(text: &str) -> String {
    redact_and_cap_text(text, AUTONOMOUS_EVIDENCE_CAP_CHARS)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn redact_and_cap_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let scan_cap = redaction_scan_cap(max_chars);
    let (scan_text, truncated) = bounded_redaction_window(text, scan_cap);
    let mut redacted = redact_sensitive(scan_text);
    if truncated {
        redacted.push_str(TRUNCATED_MARKER);
    }
    cap_text(&redacted, max_chars)
}

#[cfg_attr(not(test), allow(dead_code))]
fn redaction_scan_cap(max_chars: usize) -> usize {
    max_chars
        .saturating_mul(AUTONOMOUS_REDACTION_SCAN_MULTIPLIER)
        .max(max_chars.saturating_add(AUTONOMOUS_REDACTION_SCAN_EXTRA_CHARS))
}

#[cfg_attr(not(test), allow(dead_code))]
fn bounded_redaction_window(text: &str, scan_cap: usize) -> (&str, bool) {
    if text.len() <= scan_cap {
        return (text, false);
    }

    let prefix = crate::llm::safe_truncate(text, scan_cap);
    if prefix
        .chars()
        .last()
        .map(is_redaction_boundary)
        .unwrap_or(true)
        || text[prefix.len()..]
            .chars()
            .next()
            .map(is_redaction_boundary)
            .unwrap_or(false)
    {
        return (prefix, true);
    }

    let end = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| is_redaction_boundary(*ch))
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);

    (&prefix[..end], true)
}

#[cfg_attr(not(test), allow(dead_code))]
fn is_redaction_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ',' | ';' | ')' | ']' | '}' | '"' | '\'' | '`' | '<' | '>'
        )
}

#[cfg_attr(not(test), allow(dead_code))]
fn cap_text(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= TRUNCATED_MARKER.len() {
        return crate::llm::safe_truncate(TRUNCATED_MARKER, max_chars).to_string();
    }
    let keep = max_chars - TRUNCATED_MARKER.len();
    let prefix = crate::llm::safe_truncate(text, keep).trim_end().to_string();
    format!("{}{}", prefix, TRUNCATED_MARKER)
}

#[cfg_attr(not(test), allow(dead_code))]
pub async fn run_autonomous_buddy_chat(
    gcx: Arc<ARwLock<GlobalContext>>,
    spec: AutonomousBuddyChatSpec,
) -> Result<String, String> {
    if !validate_workflow_id(&spec.workflow_id) {
        return Err(format!(
            "invalid autonomous buddy workflow id: {}",
            spec.workflow_id
        ));
    }

    let (messages, max_steps) = build_autonomous_messages(gcx.clone(), &spec).await?;

    let mut config = crate::subchat::resolve_subchat_config(
        gcx.clone(),
        AUTONOMOUS_BUDDY_CHAT_SUBAGENT,
        true,
        Some(format!("buddy-{}-{}", spec.workflow_id, Uuid::new_v4())),
        Some(spec.title.clone()),
        None,
        None,
        None,
        Some(vec![]),
        max_steps,
        false,
        None,
        "buddy".to_string(),
    )
    .await?;

    config.mode = "buddy".to_string();
    config.buddy_meta = Some(BuddyThreadMeta {
        is_buddy_chat: true,
        buddy_chat_kind: "system".to_string(),
        workflow_id: Some(spec.workflow_id.clone()),
    });

    let result = crate::subchat::run_subchat(gcx, messages, config).await?;
    result
        .chat_id
        .ok_or_else(|| "autonomous buddy chat did not return a chat_id".to_string())
}

#[cfg_attr(not(test), allow(dead_code))]
async fn build_autonomous_messages(
    gcx: Arc<ARwLock<GlobalContext>>,
    spec: &AutonomousBuddyChatSpec,
) -> Result<(Vec<ChatMessage>, usize), String> {
    let subagent_config = crate::yaml_configs::customization_registry::get_subagent_config(
        gcx,
        AUTONOMOUS_BUDDY_CHAT_SUBAGENT,
        None,
    )
    .await
    .ok_or_else(|| {
        format!(
            "subagent config '{}' not found",
            AUTONOMOUS_BUDDY_CHAT_SUBAGENT
        )
    })?;

    let system_prompt = subagent_config.messages.system_prompt.ok_or_else(|| {
        format!(
            "messages.system_prompt not defined for subagent '{}'",
            AUTONOMOUS_BUDDY_CHAT_SUBAGENT
        )
    })?;
    let user_template = subagent_config.messages.user_template.ok_or_else(|| {
        format!(
            "messages.user_template not defined for subagent '{}'",
            AUTONOMOUS_BUDDY_CHAT_SUBAGENT
        )
    })?;

    let max_steps = subagent_config
        .subchat
        .max_steps
        .unwrap_or(1)
        .max(1)
        .min(10);
    let user_prompt = render_autonomous_template(&user_template, spec);
    let messages = vec![
        ChatMessage::new("system".to_string(), system_prompt),
        ChatMessage::new("user".to_string(), user_prompt),
    ];
    Ok((messages, max_steps))
}

#[cfg_attr(not(test), allow(dead_code))]
fn render_autonomous_template(template: &str, spec: &AutonomousBuddyChatSpec) -> String {
    let prompt = redact_and_cap_prompt(&spec.prompt);
    let evidence = redact_and_cap_evidence(&spec.evidence);
    let replacements = [
        ("{{workflow_id}}", spec.workflow_id.as_str()),
        ("{{title}}", spec.title.as_str()),
        ("{{signal_hash}}", spec.signal_hash.as_str()),
        ("{{icon}}", spec.icon.as_str()),
        ("{{badge}}", spec.badge.as_str()),
        ("{{priority}}", spec.priority.as_str()),
        ("{{prompt}}", prompt.as_str()),
        ("{{evidence}}", evidence.as_str()),
    ];
    let mut rendered = template.to_string();
    for (needle, value) in replacements {
        rendered = rendered.replace(needle, value);
    }
    rendered
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiagnosticEvidence {
    repeated_error_type: Option<String>,
    repeated_count: usize,
    high_or_critical_count: usize,
    summaries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityFinding {
    pub path: String,
    pub kind: String,
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DependencyManifestEvidence {
    pub changed_manifests: Vec<PathStatus>,
    pub manifest_counts: BTreeMap<String, usize>,
    pub total_manifest_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocsEvidence {
    pub changed_code_paths: Vec<PathStatus>,
    pub changed_doc_paths: Vec<PathStatus>,
    pub has_readme: bool,
    pub has_agents: bool,
    pub docs_file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArchitectureEvidence {
    pub changed_file_count: usize,
    pub additions: usize,
    pub deletions: usize,
    pub path_groups: Vec<PathGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathStatus {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathGroup {
    pub group: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default)]
struct LocalGitEvidence {
    changed_paths: Vec<PathStatus>,
    additions: usize,
    deletions: usize,
    security_findings: Vec<SecurityFinding>,
}

#[derive(Debug, Clone, Copy)]
struct AutonomousJobDefinition {
    workflow_id: &'static str,
    title: &'static str,
    icon: &'static str,
    badge: &'static str,
    priority: &'static str,
    cooldown_seconds: u64,
    scheduler_priority: u32,
    prompt: &'static str,
}

pub struct ErrorDetectiveJob;
pub struct SecurityWhispererJob;
pub struct SetupCoachJob;
pub struct DependencyRadarJob;
pub struct DocsGardenerJob;
pub struct ArchitectureDriftWatcherJob;

fn error_detective_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        workflow_id: ERROR_DETECTIVE_WORKFLOW_ID,
        title: "Error Detective",
        icon: "🕵️",
        badge: "Error Detective",
        priority: "high",
        cooldown_seconds: 15 * 60,
        scheduler_priority: 5,
        prompt: "Analyze the diagnostic pattern using only the summaries below. Explain the likely failure cluster, risk, and the smallest safe next checks. Do not invent log details that are not in evidence.",
    }
}

fn security_whisperer_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        workflow_id: SECURITY_WHISPERER_WORKFLOW_ID,
        title: "Security Whisperer",
        icon: "🛡️",
        badge: "Security",
        priority: "critical",
        cooldown_seconds: 30 * 60,
        scheduler_priority: 5,
        prompt: "Review the redacted local security findings. Highlight immediate leakage risk, containment steps, and safer follow-up. Never ask for or repeat raw secret values.",
    }
}

fn setup_coach_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        workflow_id: SETUP_COACH_WORKFLOW_ID,
        title: "Setup Coach",
        icon: "🧰",
        badge: "Setup",
        priority: "normal",
        cooldown_seconds: 2 * 60 * 60,
        scheduler_priority: 5,
        prompt: "Review the local setup checklist and recommend the next onboarding step. Keep it practical and based only on the checklist.",
    }
}

fn dependency_radar_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        workflow_id: DEPENDENCY_RADAR_WORKFLOW_ID,
        title: "Dependency Radar",
        icon: "📦",
        badge: "Dependencies",
        priority: "normal",
        cooldown_seconds: 2 * 60 * 60,
        scheduler_priority: 5,
        prompt: "Review local dependency manifest activity. Identify coordination risks and safe review steps without using package registry data.",
    }
}

fn docs_gardener_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        workflow_id: DOCS_GARDENER_WORKFLOW_ID,
        title: "Docs Gardener",
        icon: "📚",
        badge: "Docs",
        priority: "normal",
        cooldown_seconds: 3 * 60 * 60,
        scheduler_priority: 5,
        prompt: "Review the documentation signal. Suggest whether README, AGENTS, or docs should be updated based only on changed path categories and docs presence.",
    }
}

fn architecture_drift_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        workflow_id: ARCHITECTURE_DRIFT_WORKFLOW_ID,
        title: "Architecture Drift Watcher",
        icon: "🏗️",
        badge: "Architecture",
        priority: "normal",
        cooldown_seconds: 4 * 60 * 60,
        scheduler_priority: 5,
        prompt: "Review the local architecture drift signal. Summarize subsystem concentration or cross-cutting risk, and propose lightweight guardrails. Use only path groups and diff stats.",
    }
}

async fn execute_autonomous_job(
    gcx: Arc<ARwLock<GlobalContext>>,
    ctx: &BuddyJobContext,
    definition: AutonomousJobDefinition,
    evidence: String,
) -> BuddyJobResult {
    let spec = AutonomousBuddyChatSpec::new(
        definition.workflow_id,
        definition.title,
        definition.prompt,
        evidence,
    )
    .with_display(definition.icon, definition.badge, definition.priority);

    if same_signal(ctx, &spec.signal_hash) {
        return BuddyJobResult::default();
    }

    let signal_hash = spec.signal_hash.clone();
    let chat_id = match run_autonomous_buddy_chat(gcx, spec).await {
        Ok(chat_id) => chat_id,
        Err(err) => {
            tracing::debug!(
                "buddy: autonomous job {} skipped after subchat failure: {}",
                definition.workflow_id,
                err
            );
            return BuddyJobResult::default();
        }
    };

    let activity = BuddyActivity {
        icon: definition.icon.to_string(),
        title: format!("{} opened a Buddy check-in", definition.title),
        description: format!("Buddy created a {} system conversation.", definition.badge),
        timestamp: Utc::now().to_rfc3339(),
        activity_type: definition.workflow_id.to_string(),
    };
    let mut runtime_event = make_runtime_event(
        "buddy_autonomous_chat",
        definition.title,
        definition.workflow_id,
        &format!("{}:{}", definition.workflow_id, signal_hash),
        "completed",
        Some(definition.priority),
    );
    runtime_event.description = Some(format!(
        "Buddy created a {} system conversation from local signals.",
        definition.badge
    ));
    runtime_event.chat_id = Some(chat_id.clone());
    BuddyJobResult {
        activity: Some(activity),
        runtime_event: Some(runtime_event),
        last_result: Some(serialize_last_autonomous_result(
            &AutonomousLastResult::new(signal_hash, chat_id),
        )),
        ..Default::default()
    }
}

fn diagnostic_evidence(ctx: &BuddyJobContext) -> Option<DiagnosticEvidence> {
    let recent: Vec<&DiagnosticContext> = ctx.recent_diagnostics.iter().rev().take(50).collect();
    if recent.is_empty() {
        return None;
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut high_or_critical_count = 0;
    for diag in &recent {
        *counts.entry(diag.error_type.clone()).or_default() += 1;
        if matches!(
            diag.severity,
            DiagnosticSeverity::High | DiagnosticSeverity::Critical
        ) {
            high_or_critical_count += 1;
        }
    }

    let repeated = counts
        .into_iter()
        .filter(|(_, count)| *count >= 3)
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
    if repeated.is_none() && high_or_critical_count == 0 {
        return None;
    }

    let summaries = recent
        .into_iter()
        .take(MAX_DIAGNOSTIC_EVIDENCE)
        .map(format_diagnostic_summary)
        .collect();
    Some(DiagnosticEvidence {
        repeated_error_type: repeated.as_ref().map(|(error_type, _)| error_type.clone()),
        repeated_count: repeated.map(|(_, count)| count).unwrap_or(0),
        high_or_critical_count,
        summaries,
    })
}

fn format_diagnostic_summary(diag: &DiagnosticContext) -> String {
    let source = diag
        .source_file
        .as_deref()
        .or(diag.tool_name.as_deref())
        .unwrap_or("unknown");
    let preview = preview_text(&diag.error_message, 240);
    format!(
        "type={} severity={:?} source={} timestamp={} preview={}",
        clean_evidence_value(&diag.error_type),
        diag.severity,
        clean_evidence_value(source),
        clean_evidence_value(&diag.collected_at),
        preview
    )
}

fn render_diagnostic_evidence(evidence: &DiagnosticEvidence) -> String {
    let mut lines = vec![
        "Diagnostic signal:".to_string(),
        format!(
            "- repeated_error_type: {}",
            evidence.repeated_error_type.as_deref().unwrap_or("none")
        ),
        format!("- repeated_count: {}", evidence.repeated_count),
        format!(
            "- high_or_critical_count: {}",
            evidence.high_or_critical_count
        ),
        "- summaries:".to_string(),
    ];
    for summary in &evidence.summaries {
        lines.push(format!("  - {summary}"));
    }
    lines.join("\n")
}

fn setup_evidence(project_root: &Path) -> Option<String> {
    let has_agents = project_root.join("AGENTS.md").exists();
    let has_refact = project_root.join(".refact").exists();
    let has_refact_knowledge = project_root.join(".refact").join("knowledge").exists();
    let has_refact_tasks = project_root.join(".refact").join("tasks").exists();
    let has_readme = has_root_file_case_insensitive(project_root, "readme");
    let has_git = git2::Repository::discover(project_root).is_ok();
    if has_agents && has_refact && has_refact_knowledge && has_readme {
        return None;
    }

    Some(
        [
            "Local setup checklist:".to_string(),
            format!("- AGENTS.md present: {has_agents}"),
            format!("- README present: {has_readme}"),
            format!("- .refact directory present: {has_refact}"),
            format!("- .refact/knowledge present: {has_refact_knowledge}"),
            format!("- .refact/tasks present: {has_refact_tasks}"),
            format!("- git workspace detected: {has_git}"),
        ]
        .join("\n"),
    )
}

fn has_root_file_case_insensitive(project_root: &Path, stem: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(project_root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .map(|name| name.to_ascii_lowercase().starts_with(stem))
            .unwrap_or(false)
    })
}

fn collect_local_git_evidence(
    project_root: &Path,
    scan_security: bool,
) -> Option<LocalGitEvidence> {
    let repo = git2::Repository::discover(project_root).ok()?;
    let repo_root = repo.workdir()?.to_path_buf();
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .show(git2::StatusShow::IndexAndWorkdir);
    let statuses = repo.statuses(Some(&mut opts)).ok()?;
    let mut changed_paths = Vec::new();
    let mut seen = HashSet::new();
    let mut security_findings = Vec::new();

    for entry in statuses.iter() {
        let status = entry.status();
        if status.is_empty() {
            continue;
        }
        let Some(path) = entry.path() else {
            continue;
        };
        if seen.insert(path.to_string()) {
            changed_paths.push(PathStatus {
                path: path.to_string(),
                status: git_status_label(status).to_string(),
            });
        }
        if scan_security && security_findings.len() < MAX_SECURITY_FINDINGS {
            if let Ok(Some(content)) = git_blob_or_workdir_content(&repo, &repo_root, path, status)
            {
                security_findings.extend(scan_security_findings(
                    path,
                    &content,
                    MAX_SECURITY_FINDINGS - security_findings.len(),
                ));
            }
        }
    }

    let (additions, deletions) = diff_stats(&repo).unwrap_or((0, 0));
    changed_paths.sort_by(|a, b| a.path.cmp(&b.path));
    changed_paths.truncate(MAX_PATH_EVIDENCE);
    Some(LocalGitEvidence {
        changed_paths,
        additions,
        deletions,
        security_findings,
    })
}

fn git_status_label(status: git2::Status) -> &'static str {
    if status.is_conflicted() {
        "conflicted"
    } else if status.intersects(git2::Status::INDEX_NEW | git2::Status::WT_NEW) {
        "added"
    } else if status.intersects(git2::Status::INDEX_DELETED | git2::Status::WT_DELETED) {
        "deleted"
    } else if status.intersects(git2::Status::INDEX_RENAMED | git2::Status::WT_RENAMED) {
        "renamed"
    } else if status.intersects(git2::Status::INDEX_TYPECHANGE | git2::Status::WT_TYPECHANGE) {
        "typechanged"
    } else {
        "modified"
    }
}

fn git_blob_or_workdir_content(
    repo: &git2::Repository,
    repo_root: &Path,
    path: &str,
    status: git2::Status,
) -> Result<Option<String>, String> {
    if status.is_wt_deleted() || status.is_index_deleted() {
        return Ok(None);
    }
    let workdir_path = repo_root.join(path);
    if workdir_path.is_file() {
        let metadata = std::fs::metadata(&workdir_path).map_err(|e| e.to_string())?;
        if metadata.len() > MAX_UNTRACKED_CONTENT_BYTES {
            return Ok(None);
        }
        return std::fs::read_to_string(&workdir_path)
            .map(Some)
            .map_err(|e| e.to_string());
    }

    let index = repo.index().map_err(|e| e.to_string())?;
    let Some(entry) = index.get_path(Path::new(path), 0) else {
        return Ok(None);
    };
    let blob = repo.find_blob(entry.id).map_err(|e| e.to_string())?;
    if blob.size() > MAX_UNTRACKED_CONTENT_BYTES as usize {
        return Ok(None);
    }
    Ok(std::str::from_utf8(blob.content())
        .ok()
        .map(ToString::to_string))
}

fn diff_stats(repo: &git2::Repository) -> Option<(usize, usize)> {
    let root = repo.workdir()?;
    let staged_numstat =
        crate::worktrees::git::run_git_lossy(root, &["diff", "--cached", "--numstat"]);
    let unstaged_numstat = crate::worktrees::git::run_git_lossy(root, &["diff", "--numstat"]);
    let untracked_numstat =
        crate::worktrees::git::run_git_lossy(root, &["ls-files", "--others", "--exclude-standard"]);
    let mut additions = 0usize;
    let mut deletions = 0usize;
    accumulate_numstat(&staged_numstat, &mut additions, &mut deletions);
    accumulate_numstat(&unstaged_numstat, &mut additions, &mut deletions);
    for rel in untracked_numstat
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let path = root.join(rel);
        if let Ok(content) = std::fs::read_to_string(path) {
            additions = additions.saturating_add(content.lines().count());
        }
    }
    Some((additions, deletions))
}

fn accumulate_numstat(output: &str, additions: &mut usize, deletions: &mut usize) {
    for line in output.lines() {
        let mut parts = line.split('\t');
        if let Some(value) = parts.next().and_then(|part| part.parse::<usize>().ok()) {
            *additions = additions.saturating_add(value);
        }
        if let Some(value) = parts.next().and_then(|part| part.parse::<usize>().ok()) {
            *deletions = deletions.saturating_add(value);
        }
    }
}

fn scan_security_findings(path: &str, content: &str, limit: usize) -> Vec<SecurityFinding> {
    if limit == 0 {
        return Vec::new();
    }

    secret_patterns()
        .iter()
        .flat_map(|(kind, regex)| {
            regex.find_iter(content).map(move |m| SecurityFinding {
                path: path.to_string(),
                kind: (*kind).to_string(),
                preview: security_preview(content, m.start(), m.end()),
            })
        })
        .take(limit)
        .collect()
}

fn secret_patterns() -> &'static [(&'static str, Regex)] {
    static PATTERNS: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (
                "bearer_token",
                Regex::new(r#"(?i)Bearer\s+[^\s"',]+"#).unwrap(),
            ),
            (
                "openai_key",
                Regex::new(r#"\bsk-[A-Za-z0-9]{8,}\b"#).unwrap(),
            ),
            (
                "github_token",
                Regex::new(r#"(?i)\bghp_[A-Za-z0-9]{10,}\b"#).unwrap(),
            ),
            (
                "gitlab_token",
                Regex::new(r#"(?i)\bglpat-[A-Za-z0-9_-]{10,}\b"#).unwrap(),
            ),
            (
                "assigned_secret",
                Regex::new(
                    r#"(?i)\b(api[_-]?key|apikey|token|secret|password)\s*[:=]\s*[^\s"',;]+"#,
                )
                .unwrap(),
            ),
            (
                "authorization_header",
                Regex::new(r#"(?i)Authorization:\s*[^\s"',]+"#).unwrap(),
            ),
        ]
    })
}

fn security_preview(content: &str, start: usize, end: usize) -> String {
    let line_start = content[..start].rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    let line_end = content[end..]
        .find('\n')
        .map(|idx| end + idx)
        .unwrap_or(content.len());
    preview_text(&content[line_start..line_end], 200)
}

fn diagnostic_security_findings(diagnostics: &[DiagnosticContext]) -> Vec<SecurityFinding> {
    diagnostics
        .iter()
        .rev()
        .take(MAX_DIAGNOSTIC_EVIDENCE)
        .flat_map(|diag| {
            let source = diag
                .source_file
                .as_deref()
                .or(diag.tool_name.as_deref())
                .unwrap_or("diagnostic");
            scan_security_findings(source, &diag.error_message, MAX_SECURITY_FINDINGS)
        })
        .take(MAX_SECURITY_FINDINGS)
        .collect()
}

fn render_security_evidence(findings: &[SecurityFinding]) -> String {
    let mut lines = vec!["Redacted local security findings:".to_string()];
    for finding in findings.iter().take(MAX_SECURITY_FINDINGS) {
        lines.push(format!(
            "- path={} kind={} preview={}",
            clean_evidence_value(&finding.path),
            clean_evidence_value(&finding.kind),
            finding.preview
        ));
    }
    lines.join("\n")
}

fn dependency_manifest_evidence(
    git: Option<&LocalGitEvidence>,
    project_root: &Path,
) -> DependencyManifestEvidence {
    let changed_manifests = git
        .map(|evidence| {
            evidence
                .changed_paths
                .iter()
                .filter(|path| classify_dependency_manifest(&path.path).is_some())
                .take(MAX_PATH_EVIDENCE)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let manifest_counts = count_dependency_manifests(project_root);
    let total_manifest_count = manifest_counts.values().copied().sum();
    DependencyManifestEvidence {
        changed_manifests,
        manifest_counts,
        total_manifest_count,
    }
}

fn dependency_manifest_trigger(evidence: &DependencyManifestEvidence) -> bool {
    !evidence.changed_manifests.is_empty() || evidence.total_manifest_count >= 12
}

fn render_dependency_evidence(evidence: &DependencyManifestEvidence) -> String {
    let mut lines = vec![
        "Local dependency manifest evidence:".to_string(),
        format!("- total_manifest_count: {}", evidence.total_manifest_count),
        "- manifest_counts:".to_string(),
    ];
    for (kind, count) in &evidence.manifest_counts {
        lines.push(format!("  - {kind}: {count}"));
    }
    lines.push("- changed_manifests:".to_string());
    for path in &evidence.changed_manifests {
        lines.push(format!(
            "  - {} ({})",
            clean_evidence_value(&path.path),
            clean_evidence_value(&path.status)
        ));
    }
    lines.join("\n")
}

fn classify_dependency_manifest(path: &str) -> Option<&'static str> {
    let normalized = path.replace('\\', "/");
    let file = normalized.rsplit('/').next()?.to_ascii_lowercase();
    match file.as_str() {
        "package.json" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" | "bun.lockb" => {
            Some("javascript")
        }
        "cargo.toml" | "cargo.lock" => Some("rust"),
        "pyproject.toml" | "poetry.lock" | "pdm.lock" | "requirements.txt" | "pipfile"
        | "pipfile.lock" => Some("python"),
        "go.mod" | "go.sum" => Some("go"),
        "pom.xml" | "build.gradle" | "build.gradle.kts" | "gradle.lockfile" => Some("jvm"),
        "gemfile" | "gemfile.lock" => Some("ruby"),
        "composer.json" | "composer.lock" => Some("php"),
        "mix.exs" | "mix.lock" => Some("elixir"),
        "packages.config" | "paket.dependencies" | "paket.lock" => Some("dotnet"),
        _ => None,
    }
}

fn count_dependency_manifests(project_root: &Path) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    let mut stack = vec![project_root.to_path_buf()];
    let mut scanned = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if scanned >= MAX_MANIFEST_WALK_FILES {
                return counts;
            }
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if should_skip_dir(&name) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            scanned += 1;
            let Ok(rel) = path.strip_prefix(project_root) else {
                continue;
            };
            if let Some(kind) = classify_dependency_manifest(&rel.to_string_lossy()) {
                *counts.entry(kind.to_string()).or_insert(0) += 1;
            }
        }
    }
    counts
}

fn docs_evidence(git: Option<&LocalGitEvidence>, project_root: &Path) -> DocsEvidence {
    let changed_paths = git
        .map(|evidence| evidence.changed_paths.as_slice())
        .unwrap_or_default();
    let changed_code_paths = changed_paths
        .iter()
        .filter(|path| is_code_path(&path.path))
        .take(MAX_PATH_EVIDENCE)
        .cloned()
        .collect();
    let changed_doc_paths = changed_paths
        .iter()
        .filter(|path| is_docs_path(&path.path))
        .take(MAX_PATH_EVIDENCE)
        .cloned()
        .collect();
    DocsEvidence {
        changed_code_paths,
        changed_doc_paths,
        has_readme: has_root_file_case_insensitive(project_root, "readme"),
        has_agents: project_root.join("AGENTS.md").exists(),
        docs_file_count: count_docs_files(project_root),
    }
}

fn docs_trigger(evidence: &DocsEvidence) -> bool {
    (!evidence.changed_code_paths.is_empty() && evidence.changed_doc_paths.is_empty())
        || !evidence.has_readme
        || !evidence.has_agents
        || evidence.docs_file_count == 0
}

fn render_docs_evidence(evidence: &DocsEvidence) -> String {
    let mut lines = vec![
        "Docs signal:".to_string(),
        format!("- has_readme: {}", evidence.has_readme),
        format!("- has_agents: {}", evidence.has_agents),
        format!("- docs_file_count: {}", evidence.docs_file_count),
        format!(
            "- changed_code_path_count: {}",
            evidence.changed_code_paths.len()
        ),
        format!(
            "- changed_doc_path_count: {}",
            evidence.changed_doc_paths.len()
        ),
        "- changed_code_paths:".to_string(),
    ];
    for path in &evidence.changed_code_paths {
        lines.push(format!(
            "  - {} ({})",
            clean_evidence_value(&path.path),
            clean_evidence_value(&path.status)
        ));
    }
    lines.push("- changed_doc_paths:".to_string());
    for path in &evidence.changed_doc_paths {
        lines.push(format!(
            "  - {} ({})",
            clean_evidence_value(&path.path),
            clean_evidence_value(&path.status)
        ));
    }
    lines.join("\n")
}

fn is_docs_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let file = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    normalized.starts_with("docs/")
        || normalized.contains("/docs/")
        || file.starts_with("readme")
        || file.starts_with("changelog")
        || file == "agents.md"
        || file.ends_with(".md")
        || file.ends_with(".mdx")
        || file.ends_with(".rst")
        || file.ends_with(".adoc")
}

fn is_code_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    if is_docs_path(&normalized) || normalized.starts_with(".refact/") {
        return false;
    }
    let Some(file) = normalized.rsplit('/').next() else {
        return false;
    };
    if classify_dependency_manifest(file).is_some() {
        return false;
    }
    matches!(
        file.rsplit('.').next().unwrap_or_default(),
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "kts"
            | "cpp"
            | "cc"
            | "c"
            | "h"
            | "hpp"
            | "cs"
            | "php"
            | "rb"
            | "swift"
            | "scala"
            | "sql"
            | "yaml"
            | "yml"
            | "toml"
    )
}

fn count_docs_files(project_root: &Path) -> usize {
    let mut count = 0;
    let mut stack = vec![project_root.to_path_buf()];
    let mut scanned = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if scanned >= MAX_MANIFEST_WALK_FILES {
                return count;
            }
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if should_skip_dir(&name) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            scanned += 1;
            let Ok(rel) = path.strip_prefix(project_root) else {
                continue;
            };
            if is_docs_path(&rel.to_string_lossy()) {
                count += 1;
            }
        }
    }
    count
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | "vendor" | "dist" | "build" | ".next" | ".cache"
    )
}

fn architecture_evidence(git: &LocalGitEvidence) -> ArchitectureEvidence {
    let relevant_paths: Vec<PathStatus> = git
        .changed_paths
        .iter()
        .filter(|path| is_code_path(&path.path) || is_docs_path(&path.path))
        .cloned()
        .collect();
    let path_groups = architecture_groups(&relevant_paths);
    ArchitectureEvidence {
        changed_file_count: relevant_paths.len(),
        additions: git.additions,
        deletions: git.deletions,
        path_groups,
    }
}

fn architecture_trigger(evidence: &ArchitectureEvidence) -> bool {
    let total_lines = evidence.additions + evidence.deletions;
    let largest_group = evidence
        .path_groups
        .iter()
        .map(|group| group.count)
        .max()
        .unwrap_or(0);
    largest_group >= 8
        || evidence.path_groups.len() >= 5 && evidence.changed_file_count >= 12
        || total_lines >= 700
        || evidence.changed_file_count >= 25
}

fn architecture_groups(paths: &[PathStatus]) -> Vec<PathGroup> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for path in paths {
        let group = architecture_group_for_path(&path.path);
        *counts.entry(group).or_insert(0) += 1;
    }
    let mut groups: Vec<PathGroup> = counts
        .into_iter()
        .map(|(group, count)| PathGroup { group, count })
        .collect();
    groups.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.group.cmp(&b.group)));
    groups.truncate(10);
    groups
}

fn architecture_group_for_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut parts = normalized.split('/').filter(|part| !part.is_empty());
    match (parts.next(), parts.next()) {
        (Some("src"), Some(next)) => format!("src/{next}"),
        (Some("refact-agent"), Some(next)) => format!("refact-agent/{next}"),
        (Some("extra"), Some(next)) => format!("extra/{next}"),
        (Some("crates"), Some(next)) => format!("crates/{next}"),
        (Some(first), _) => first.to_string(),
        _ => "root".to_string(),
    }
}

fn render_architecture_evidence(evidence: &ArchitectureEvidence) -> String {
    let mut lines = vec![
        "Architecture drift signal:".to_string(),
        format!("- changed_file_count: {}", evidence.changed_file_count),
        format!("- additions: {}", evidence.additions),
        format!("- deletions: {}", evidence.deletions),
        "- path_groups:".to_string(),
    ];
    for group in &evidence.path_groups {
        lines.push(format!(
            "  - {}: {} files",
            clean_evidence_value(&group.group),
            group.count
        ));
    }
    lines.join("\n")
}

fn clean_evidence_value(value: &str) -> String {
    preview_text(value, 240)
}

fn preview_text(value: &str, max_chars: usize) -> String {
    let single_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    redact_and_cap_text(&single_line, max_chars)
}

#[async_trait::async_trait]
impl BuddyJob for ErrorDetectiveJob {
    fn id(&self) -> &str {
        ERROR_DETECTIVE_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        error_detective_definition().cooldown_seconds
    }

    fn priority(&self) -> u32 {
        error_detective_definition().scheduler_priority
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        diagnostic_evidence(ctx).is_some()
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let Some(evidence) = diagnostic_evidence(&ctx) else {
            return BuddyJobResult::default();
        };
        execute_autonomous_job(
            gcx,
            &ctx,
            error_detective_definition(),
            render_diagnostic_evidence(&evidence),
        )
        .await
    }
}

#[async_trait::async_trait]
impl BuddyJob for SecurityWhispererJob {
    fn id(&self) -> &str {
        SECURITY_WHISPERER_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        security_whisperer_definition().cooldown_seconds
    }

    fn priority(&self) -> u32 {
        security_whisperer_definition().scheduler_priority
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        let diagnostic_findings = diagnostic_security_findings(&ctx.recent_diagnostics);
        if !diagnostic_findings.is_empty() {
            return true;
        }
        let root = ctx.project_root.clone();
        tokio::task::spawn_blocking(move || {
            collect_local_git_evidence(&root, true)
                .map(|evidence| !evidence.security_findings.is_empty())
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let root = ctx.project_root.clone();
        let mut findings = tokio::task::spawn_blocking(move || {
            collect_local_git_evidence(&root, true)
                .map(|evidence| evidence.security_findings)
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        findings.extend(diagnostic_security_findings(&ctx.recent_diagnostics));
        findings.truncate(MAX_SECURITY_FINDINGS);
        if findings.is_empty() {
            return BuddyJobResult::default();
        }
        execute_autonomous_job(
            gcx,
            &ctx,
            security_whisperer_definition(),
            render_security_evidence(&findings),
        )
        .await
    }
}

#[async_trait::async_trait]
impl BuddyJob for SetupCoachJob {
    fn id(&self) -> &str {
        SETUP_COACH_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        setup_coach_definition().cooldown_seconds
    }

    fn priority(&self) -> u32 {
        setup_coach_definition().scheduler_priority
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        setup_evidence(&ctx.project_root).is_some()
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let Some(evidence) = setup_evidence(&ctx.project_root) else {
            return BuddyJobResult::default();
        };
        execute_autonomous_job(gcx, &ctx, setup_coach_definition(), evidence).await
    }
}

#[async_trait::async_trait]
impl BuddyJob for DependencyRadarJob {
    fn id(&self) -> &str {
        DEPENDENCY_RADAR_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        dependency_radar_definition().cooldown_seconds
    }

    fn priority(&self) -> u32 {
        dependency_radar_definition().scheduler_priority
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        let root = ctx.project_root.clone();
        tokio::task::spawn_blocking(move || {
            let git = collect_local_git_evidence(&root, false);
            dependency_manifest_trigger(&dependency_manifest_evidence(git.as_ref(), &root))
        })
        .await
        .unwrap_or(false)
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let root = ctx.project_root.clone();
        let evidence = tokio::task::spawn_blocking(move || {
            let git = collect_local_git_evidence(&root, false);
            dependency_manifest_evidence(git.as_ref(), &root)
        })
        .await
        .unwrap_or_default();
        if !dependency_manifest_trigger(&evidence) {
            return BuddyJobResult::default();
        }
        execute_autonomous_job(
            gcx,
            &ctx,
            dependency_radar_definition(),
            render_dependency_evidence(&evidence),
        )
        .await
    }
}

#[async_trait::async_trait]
impl BuddyJob for DocsGardenerJob {
    fn id(&self) -> &str {
        DOCS_GARDENER_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        docs_gardener_definition().cooldown_seconds
    }

    fn priority(&self) -> u32 {
        docs_gardener_definition().scheduler_priority
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        let root = ctx.project_root.clone();
        tokio::task::spawn_blocking(move || {
            let git = collect_local_git_evidence(&root, false);
            docs_trigger(&docs_evidence(git.as_ref(), &root))
        })
        .await
        .unwrap_or(false)
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let root = ctx.project_root.clone();
        let evidence = tokio::task::spawn_blocking(move || {
            let git = collect_local_git_evidence(&root, false);
            docs_evidence(git.as_ref(), &root)
        })
        .await
        .unwrap_or_default();
        if !docs_trigger(&evidence) {
            return BuddyJobResult::default();
        }
        execute_autonomous_job(
            gcx,
            &ctx,
            docs_gardener_definition(),
            render_docs_evidence(&evidence),
        )
        .await
    }
}

#[async_trait::async_trait]
impl BuddyJob for ArchitectureDriftWatcherJob {
    fn id(&self) -> &str {
        ARCHITECTURE_DRIFT_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        architecture_drift_definition().cooldown_seconds
    }

    fn priority(&self) -> u32 {
        architecture_drift_definition().scheduler_priority
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        let root = ctx.project_root.clone();
        tokio::task::spawn_blocking(move || {
            collect_local_git_evidence(&root, false)
                .map(|git| architecture_trigger(&architecture_evidence(&git)))
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let root = ctx.project_root.clone();
        let evidence = tokio::task::spawn_blocking(move || {
            collect_local_git_evidence(&root, false).map(|git| architecture_evidence(&git))
        })
        .await
        .unwrap_or(None);
        let Some(evidence) = evidence else {
            return BuddyJobResult::default();
        };
        if !architecture_trigger(&evidence) {
            return BuddyJobResult::default();
        }
        execute_autonomous_job(
            gcx,
            &ctx,
            architecture_drift_definition(),
            render_architecture_evidence(&evidence),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::scheduler::BuddyJobContext;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};
    use crate::call_validation::ChatContent;
    use crate::yaml_configs::customization_types::SubagentConfig;

    fn context_with_last_result(last_result: Option<String>) -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            onboarding: BuddyOnboarding::default(),
            recent_diagnostics: vec![],
            project_root: std::path::PathBuf::from("/tmp/project"),
            job_state: BuddyJobState {
                last_result,
                ..Default::default()
            },
            total_workflow_runs: 0,
            suggestion_state: vec![],
            pet: BuddyPetState::default(),
            active_quest: None,
            settings: BuddySettings::default(),
            pulse: BuddyPulse::default(),
            facts: vec![],
        }
    }

    #[test]
    fn signal_hash_is_stable_and_changes_with_signal() {
        let first = signal_hash(["buddy_error_detective", "a", "b"]);
        let second = signal_hash(["buddy_error_detective", "a", "b"]);
        let changed = signal_hash(["buddy_error_detective", "a", "c"]);
        let boundary_a = signal_hash(["ab", "c"]);
        let boundary_b = signal_hash(["a", "bc"]);

        assert_eq!(first, second);
        assert_ne!(first, changed);
        assert_ne!(boundary_a, boundary_b);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn default_signal_hash_uses_redacted_capped_prompt_and_evidence() {
        let prompt = "Review failed login traces";
        let first_evidence = "request failed password=alpha-secret token=first-token";
        let second_evidence = "request failed password=beta-secret token=second-token";
        let first = AutonomousBuddyChatSpec::new(
            "buddy_security_whisperer",
            "Security Whisperer",
            prompt,
            first_evidence,
        );
        let second = AutonomousBuddyChatSpec::new(
            "buddy_security_whisperer",
            "Security Whisperer",
            prompt,
            second_evidence,
        );
        let expected = signal_hash([
            "buddy_security_whisperer",
            "Security Whisperer",
            redact_and_cap_prompt(prompt).as_str(),
            redact_and_cap_evidence(first_evidence).as_str(),
        ]);
        let raw_first = signal_hash([
            "buddy_security_whisperer",
            "Security Whisperer",
            prompt,
            first_evidence,
        ]);
        let raw_second = signal_hash([
            "buddy_security_whisperer",
            "Security Whisperer",
            prompt,
            second_evidence,
        ]);

        assert_ne!(raw_first, raw_second);
        assert_eq!(first.signal_hash, second.signal_hash);
        assert_eq!(first.signal_hash, expected);
        assert_ne!(first.signal_hash, raw_first);

        let displayed = first.with_display("🛡️", "Security", "high");
        assert_eq!(displayed.icon, "🛡️");
        assert_eq!(displayed.badge, "Security");
        assert_eq!(displayed.priority, "high");
    }

    #[test]
    fn last_result_json_round_trips_and_malformed_values_fallback() {
        let result = AutonomousLastResult {
            signal_hash: "hash-a".to_string(),
            chat_id: "chat-a".to_string(),
            completed_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let serialized = serialize_last_autonomous_result(&result);

        assert_eq!(
            parse_last_autonomous_result(Some(&serialized)),
            Some(result)
        );
        assert_eq!(parse_last_autonomous_result(Some("legacy-value")), None);
        assert_eq!(parse_last_autonomous_result(Some("{")), None);
        assert_eq!(parse_last_autonomous_result(Some("{}")), None);
        assert_eq!(parse_last_autonomous_result(None), None);

        let dynamic = AutonomousLastResult::new("hash-b", "chat-b");
        assert_eq!(dynamic.signal_hash, "hash-b");
        assert_eq!(dynamic.chat_id, "chat-b");
        assert!(!dynamic.completed_at.is_empty());
    }

    #[test]
    fn same_signal_uses_parsed_last_result() {
        let result = AutonomousLastResult {
            signal_hash: "same".to_string(),
            chat_id: "chat".to_string(),
            completed_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let ctx = context_with_last_result(Some(serialize_last_autonomous_result(&result)));
        let malformed_ctx = context_with_last_result(Some("same".to_string()));

        assert!(same_signal(&ctx, "same"));
        assert!(!same_signal(&ctx, "different"));
        assert!(!same_signal(&malformed_ctx, "same"));
    }

    #[test]
    fn redaction_and_capping_remove_obvious_raw_secrets() {
        let raw = "Bearer abcdef12345 password=plainsecret sk-abcdef123456 ghp_abcdef1234567890";
        let redacted = redact_and_cap_text(raw, 512);
        let capped = redact_and_cap_text(&format!("{} {}", raw, "x".repeat(256)), 64);

        assert!(!redacted.contains("abcdef12345"));
        assert!(!redacted.contains("plainsecret"));
        assert!(!redacted.contains("sk-abcdef123456"));
        assert!(!redacted.contains("ghp_abcdef1234567890"));
        assert!(redacted.contains("[REDACTED"));
        assert!(capped.len() <= 64);
        assert!(!capped.contains("plainsecret"));
    }

    #[test]
    fn redaction_scans_beyond_final_cap_without_partial_secret_leaks() {
        let raw = format!(
            "{} password=plainsecret token=othertoken {}",
            "x".repeat(40),
            "y".repeat(10_000)
        );
        let capped = redact_and_cap_text(&raw, 96);

        assert!(capped.len() <= 96);
        assert!(!capped.contains("plainsecret"));
        assert!(!capped.contains("othertoken"));
        assert!(capped.contains("[REDACTED"));
    }

    #[test]
    fn bounded_redaction_window_does_not_split_secret_tokens() {
        let raw = format!("{} sk-{}", "x".repeat(32), "a".repeat(128));
        let (window, truncated) = bounded_redaction_window(&raw, 48);

        assert!(truncated);
        assert!(!window.contains("sk-"));
    }

    #[test]
    fn rendered_autonomous_prompt_contains_no_raw_obvious_secrets() {
        let spec = AutonomousBuddyChatSpec::new(
            "buddy_security_whisperer",
            "Security Whisperer",
            "Check Bearer rawtokenvalue and secret=promptsecret",
            "Found password=evidencesecret sk-abcdef123456 ghp_abcdef1234567890",
        );
        let rendered = render_autonomous_template(
            "Task:\n{{prompt}}\nEvidence:\n{{evidence}}\nSignal:\n{{signal_hash}}",
            &spec,
        );

        for raw in [
            "rawtokenvalue",
            "promptsecret",
            "evidencesecret",
            "sk-abcdef123456",
            "ghp_abcdef1234567890",
        ] {
            assert!(!rendered.contains(raw), "raw secret leaked: {rendered}");
        }
        assert!(rendered.contains("[REDACTED"));
    }

    #[test]
    fn autonomous_yaml_defaults_to_stateless_no_tools_report_sections() {
        let config: SubagentConfig = serde_yaml::from_str(include_str!(
            "../../yaml_configs/defaults/subagents/buddy_autonomous_chat.yaml"
        ))
        .unwrap();
        let system_prompt = config.messages.system_prompt.as_deref().unwrap_or_default();

        assert!(!config.subchat.stateful);
        assert!(config.tools.is_empty());
        assert!(system_prompt.contains("Summary"));
        assert!(system_prompt.contains("Evidence"));
        assert!(system_prompt.contains("Risk or opportunity"));
        assert!(system_prompt.contains("Suggested next steps"));
    }

    #[test]
    fn dependency_manifest_classifier_recognizes_common_manifests() {
        assert_eq!(
            classify_dependency_manifest("package.json"),
            Some("javascript")
        );
        assert_eq!(
            classify_dependency_manifest("web/pnpm-lock.yaml"),
            Some("javascript")
        );
        assert_eq!(
            classify_dependency_manifest("refact-agent/engine/Cargo.toml"),
            Some("rust")
        );
        assert_eq!(
            classify_dependency_manifest("requirements.txt"),
            Some("python")
        );
        assert_eq!(classify_dependency_manifest("go.mod"), Some("go"));
        assert_eq!(classify_dependency_manifest("README.md"), None);
    }

    #[test]
    fn dependency_manifest_trigger_uses_changed_manifests_or_many_local_manifests() {
        let changed = DependencyManifestEvidence {
            changed_manifests: vec![PathStatus {
                path: "Cargo.toml".to_string(),
                status: "modified".to_string(),
            }],
            ..Default::default()
        };
        let many = DependencyManifestEvidence {
            manifest_counts: BTreeMap::from([("javascript".to_string(), 12)]),
            total_manifest_count: 12,
            ..Default::default()
        };
        let quiet = DependencyManifestEvidence {
            total_manifest_count: 2,
            ..Default::default()
        };

        assert!(dependency_manifest_trigger(&changed));
        assert!(dependency_manifest_trigger(&many));
        assert!(!dependency_manifest_trigger(&quiet));
    }

    #[test]
    fn docs_and_code_path_classifiers_split_docs_from_code() {
        assert!(is_docs_path("README.md"));
        assert!(is_docs_path("docs/setup.mdx"));
        assert!(is_docs_path("AGENTS.md"));
        assert!(!is_docs_path("src/main.rs"));
        assert!(is_code_path("src/main.rs"));
        assert!(is_code_path("refact-agent/gui/src/App.tsx"));
        assert!(!is_code_path("docs/architecture.md"));
        assert!(!is_code_path("package-lock.json"));
    }

    #[test]
    fn docs_trigger_detects_code_without_docs_and_missing_docs() {
        let code_without_docs = DocsEvidence {
            changed_code_paths: vec![PathStatus {
                path: "src/lib.rs".to_string(),
                status: "modified".to_string(),
            }],
            has_readme: true,
            has_agents: true,
            docs_file_count: 1,
            ..Default::default()
        };
        let with_docs = DocsEvidence {
            changed_code_paths: code_without_docs.changed_code_paths.clone(),
            changed_doc_paths: vec![PathStatus {
                path: "README.md".to_string(),
                status: "modified".to_string(),
            }],
            has_readme: true,
            has_agents: true,
            docs_file_count: 1,
        };
        let missing_agents = DocsEvidence {
            has_readme: true,
            has_agents: false,
            docs_file_count: 1,
            ..Default::default()
        };

        assert!(docs_trigger(&code_without_docs));
        assert!(!docs_trigger(&with_docs));
        assert!(docs_trigger(&missing_agents));
    }

    #[test]
    fn architecture_grouping_and_thresholds_detect_drift() {
        let mut same_subsystem = Vec::new();
        for idx in 0..8 {
            same_subsystem.push(PathStatus {
                path: format!("src/chat/file_{idx}.rs"),
                status: "modified".to_string(),
            });
        }
        let evidence = ArchitectureEvidence {
            changed_file_count: same_subsystem.len(),
            additions: 10,
            deletions: 5,
            path_groups: architecture_groups(&same_subsystem),
        };
        assert_eq!(evidence.path_groups[0].group, "src/chat");
        assert_eq!(evidence.path_groups[0].count, 8);
        assert!(architecture_trigger(&evidence));

        let broad = ArchitectureEvidence {
            changed_file_count: 12,
            additions: 10,
            deletions: 5,
            path_groups: vec![
                PathGroup {
                    group: "a".to_string(),
                    count: 3,
                },
                PathGroup {
                    group: "b".to_string(),
                    count: 3,
                },
                PathGroup {
                    group: "c".to_string(),
                    count: 2,
                },
                PathGroup {
                    group: "d".to_string(),
                    count: 2,
                },
                PathGroup {
                    group: "e".to_string(),
                    count: 2,
                },
            ],
        };
        assert!(architecture_trigger(&broad));

        let large_stats = ArchitectureEvidence {
            changed_file_count: 2,
            additions: 500,
            deletions: 250,
            path_groups: vec![PathGroup {
                group: "src".to_string(),
                count: 2,
            }],
        };
        assert!(architecture_trigger(&large_stats));

        let small = ArchitectureEvidence {
            changed_file_count: 2,
            additions: 20,
            deletions: 10,
            path_groups: vec![PathGroup {
                group: "src".to_string(),
                count: 2,
            }],
        };
        assert!(!architecture_trigger(&small));
    }

    #[test]
    fn security_scanner_redacts_secret_values() {
        let raw = "const token = \"ghp_abcdefghijklmnopqrstuvwxyz\"; password=plainsecret";
        let findings = scan_security_findings("src/config.ts", raw, 10);
        let rendered = render_security_evidence(&findings);

        assert!(!findings.is_empty());
        assert!(!rendered.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
        assert!(!rendered.contains("plainsecret"));
        assert!(rendered.contains("[REDACTED"));
        assert!(rendered.contains("src/config.ts"));
    }

    #[test]
    fn unchanged_signal_produces_no_chat_in_helper_logic() {
        let spec = AutonomousBuddyChatSpec::new(
            DEPENDENCY_RADAR_WORKFLOW_ID,
            "Dependency Radar",
            dependency_radar_definition().prompt,
            "Local dependency manifest evidence:\n- total_manifest_count: 12",
        );
        let result = AutonomousLastResult {
            signal_hash: spec.signal_hash.clone(),
            chat_id: "chat-a".to_string(),
            completed_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let ctx = context_with_last_result(Some(serialize_last_autonomous_result(&result)));

        assert!(same_signal(&ctx, &spec.signal_hash));
    }

    #[tokio::test]
    async fn build_autonomous_messages_render_safe_user_prompt() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let spec = AutonomousBuddyChatSpec::new(
            "buddy_security_whisperer",
            "Security Whisperer",
            "Check token=promptsecret",
            "Found password=evidencesecret",
        );
        let (messages, max_steps) = build_autonomous_messages(gcx, &spec).await.unwrap();

        assert_eq!(max_steps, 1);
        assert_eq!(messages.len(), 2);
        let ChatContent::SimpleText(user_prompt) = &messages[1].content else {
            panic!("expected simple text user prompt");
        };
        assert!(!user_prompt.contains("promptsecret"));
        assert!(!user_prompt.contains("evidencesecret"));
        assert!(user_prompt.contains("[REDACTED"));
    }

    #[tokio::test]
    async fn run_autonomous_buddy_chat_rejects_invalid_workflow_id() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let spec = AutonomousBuddyChatSpec::new("../bad", "Bad", "Prompt", "Evidence");
        let err = run_autonomous_buddy_chat(gcx, spec).await.unwrap_err();

        assert!(err.contains("invalid autonomous buddy workflow id"));
    }
}
