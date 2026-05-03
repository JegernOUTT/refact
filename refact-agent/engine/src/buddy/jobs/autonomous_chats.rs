use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock as ARwLock;
use uuid::Uuid;

use crate::buddy::actor::{make_runtime_event, redact_sensitive, validate_workflow_id};
use crate::buddy::autonomous_workflows::{
    autonomous_workflow_meta, is_autonomous_workflow_id, AutonomousWorkflowMeta,
    ARCHITECTURE_DRIFT_WORKFLOW_ID, BEHAVIOR_LEARNER_WORKFLOW_ID, DEPENDENCY_RADAR_WORKFLOW_ID,
    DOCS_GARDENER_WORKFLOW_ID, ERROR_DETECTIVE_WORKFLOW_ID, KNOWLEDGE_CONFLICT_WORKFLOW_ID,
    MEMORY_GARDENER_WORKFLOW_ID, MODEL_COST_OPTIMIZER_WORKFLOW_ID, SECURITY_WHISPERER_WORKFLOW_ID,
    SETUP_COACH_WORKFLOW_ID, USER_HABIT_COACH_WORKFLOW_ID,
};
use crate::buddy::diagnostics::{DiagnosticContext, DiagnosticSeverity};
use crate::buddy::memory_lifecycle::{MemoryCandidate, MemoryCandidateStatus, MemorySource};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::buddy::types::{BuddyActivity, BuddyFact, BuddyFactKind, BuddyRuntimeEvent, BuddyThreadMeta};
use crate::call_validation::ChatMessage;
use crate::global_context::GlobalContext;
use crate::stats::event::LlmCallEvent;

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
const MAX_BEHAVIOR_TRAJECTORY_BYTES: u64 = 1_024 * 1_024;
const MAX_BEHAVIOR_TRAJECTORY_META_BYTES: u64 = 32 * 1024;
const MAX_BEHAVIOR_TRAJECTORY_SCAN_FILES: usize = 5_000;
const MAX_BEHAVIOR_TRAJECTORY_SCAN_ENTRIES: usize = 5_000;
const MAX_BEHAVIOR_TASK_DIR_SCAN_ENTRIES: usize = 1_000;
const MODEL_COST_RECENT_EVENT_LIMIT: usize = 500;

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

fn preserve_last_result(ctx: &BuddyJobContext) -> BuddyJobResult {
    BuddyJobResult {
        last_result: ctx.job_state.last_result.clone(),
        ..Default::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutonomousEvidence {
    prompt: String,
    evidence: String,
}

pub struct BuddyMemoryGardenerJob;
pub struct BuddyKnowledgeConflictResolverJob;
pub struct BuddyBehaviorLearnerJob;
pub struct BuddyUserHabitCoachJob;
pub struct BuddyModelCostOptimizerJob;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryRouting {
    MemoryGardener,
    KnowledgeConflictResolver,
}

const MEMORY_COOLDOWN_SECS: u64 = 6 * 60 * 60;
const BEHAVIOR_COOLDOWN_SECS: u64 = 4 * 60 * 60;
const MAX_FACT_EVIDENCE_ITEMS: usize = 12;
const MAX_TRAJECTORY_SNIPPETS: usize = 12;
const MAX_BEHAVIOR_PREFERENCE_WRITES: usize = 2;

fn fact_kind_name(kind: BuddyFactKind) -> &'static str {
    match kind {
        BuddyFactKind::TaskStuck => "task_stuck",
        BuddyFactKind::TaskAbandoned => "task_abandoned",
        BuddyFactKind::TaskClusterDuplicate => "task_cluster_duplicate",
        BuddyFactKind::TrajectoryClutter => "trajectory_clutter",
        BuddyFactKind::ChatRetryStreak => "chat_retry_streak",
        BuddyFactKind::MemoryOrphan => "memory_orphan",
        BuddyFactKind::MemoryStaleConflict => "memory_stale_conflict",
        BuddyFactKind::MemoryRecurringLesson => "memory_recurring_lesson",
        BuddyFactKind::ModePromptOverlap => "mode_prompt_overlap",
        BuddyFactKind::SkillTriggerWeak => "skill_trigger_weak",
        BuddyFactKind::AgentsMdGapDetected => "agents_md_gap_detected",
        BuddyFactKind::DefaultModelMissing => "default_model_missing",
        BuddyFactKind::BrokenModelReference => "broken_model_reference",
        BuddyFactKind::McpAuthExpired => "mcp_auth_expired",
        BuddyFactKind::IntegrationFailing => "integration_failing",
        BuddyFactKind::DiagnosticCluster => "diagnostic_cluster",
        BuddyFactKind::FrontendErrorBurst => "frontend_error_burst",
        BuddyFactKind::GitDiffWidening => "git_diff_widening",
        BuddyFactKind::UncommittedPressure => "uncommitted_pressure",
        BuddyFactKind::WorktreeHygiene => "worktree_hygiene",
    }
}

fn push_payload_string(lines: &mut Vec<String>, label: &str, value: &serde_json::Value) {
    if let Some(text) = value.as_str() {
        let text = redact_and_cap_text(text, 240);
        if !text.is_empty() {
            lines.push(format!("{}: {}", label, text));
        }
    }
}

fn payload_string_array(value: &serde_json::Value, key: &str, max: usize) -> Vec<String> {
    let mut items: Vec<String> = value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|item| redact_and_cap_text(item, 120))
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default();
    items.sort();
    items.dedup();
    items.truncate(max);
    items
}

fn memory_fact_to_evidence_line(fact: &BuddyFact) -> String {
    let mut parts = vec![format!(
        "{} key={} source={} confidence={:.2}",
        fact_kind_name(fact.kind),
        redact_and_cap_text(&fact.key, 180),
        fact.source,
        fact.confidence
    )];
    if let Some(count) = fact.payload.get("count").and_then(|v| v.as_u64()) {
        parts.push(format!("count={}", count));
    }
    let memory_ids = payload_string_array(&fact.payload, "memory_ids", 8);
    if !memory_ids.is_empty() {
        parts.push(format!("memory_ids={}", memory_ids.join(", ")));
    }
    let doc_ids = payload_string_array(&fact.payload, "doc_ids", 8);
    if !doc_ids.is_empty() {
        parts.push(format!("doc_ids={}", doc_ids.join(", ")));
    }
    let tags = payload_string_array(&fact.payload, "tags", 8);
    if !tags.is_empty() {
        parts.push(format!("tags={}", tags.join(", ")));
    }
    if let Some(title) = fact.payload.get("title").and_then(|v| v.as_str()) {
        parts.push(format!("title={}", redact_and_cap_text(title, 160)));
    }
    if let Some(summary) = fact
        .payload
        .get("conflict_summary")
        .or_else(|| fact.payload.get("summary"))
        .and_then(|v| v.as_str())
    {
        parts.push(format!("summary={}", redact_and_cap_text(summary, 240)));
    }
    if let Some(tag_hash) = fact.payload.get("tag_hash").and_then(|v| v.as_str()) {
        parts.push(format!("tag_hash={}", redact_and_cap_text(tag_hash, 120)));
    }
    parts.join("; ")
}

fn stable_facts<'a>(facts: impl Iterator<Item = &'a BuddyFact>) -> Vec<&'a BuddyFact> {
    let mut facts = facts.collect::<Vec<_>>();
    facts.sort_by(|a, b| {
        fact_kind_name(a.kind)
            .cmp(fact_kind_name(b.kind))
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.source.cmp(b.source))
            .then_with(|| a.seen_at.cmp(&b.seen_at))
    });
    facts
}

fn memory_route_for_fact_kind(kind: BuddyFactKind) -> Option<MemoryRouting> {
    match kind {
        BuddyFactKind::MemoryOrphan | BuddyFactKind::MemoryRecurringLesson => {
            Some(MemoryRouting::MemoryGardener)
        }
        BuddyFactKind::MemoryStaleConflict => Some(MemoryRouting::KnowledgeConflictResolver),
        _ => None,
    }
}

fn memory_gardener_evidence(ctx: &BuddyJobContext) -> Option<AutonomousEvidence> {
    let mut lines = Vec::new();
    if ctx.pulse.memory.total > 0 || ctx.pulse.memory.orphan > 0 {
        lines.push(format!(
            "memory_pulse total={} orphan={}",
            ctx.pulse.memory.total, ctx.pulse.memory.orphan
        ));
    }
    for fact in stable_facts(ctx.facts.iter().filter(|fact| {
        matches!(
            memory_route_for_fact_kind(fact.kind),
            Some(MemoryRouting::MemoryGardener)
        )
    }))
    .into_iter()
    .take(MAX_FACT_EVIDENCE_ITEMS)
    {
        lines.push(memory_fact_to_evidence_line(fact));
    }
    if ctx.pulse.memory.orphan == 0
        && !lines
            .iter()
            .any(|line| line.contains("memory_orphan") || line.contains("memory_recurring_lesson"))
    {
        return None;
    }
    Some(AutonomousEvidence {
        prompt: "Review memory garden signals and recommend safe cleanup, consolidation, or follow-up. Use only the metadata in evidence; do not assume full memory contents.".to_string(),
        evidence: lines.join("\n"),
    })
}

fn stale_conflict_fact_actionable(fact: &BuddyFact) -> bool {
    if fact.kind != BuddyFactKind::MemoryStaleConflict {
        return false;
    }
    !payload_string_array(&fact.payload, "doc_ids", 1).is_empty()
        || !payload_string_array(&fact.payload, "memory_ids", 1).is_empty()
        || fact
            .payload
            .get("conflict_summary")
            .or_else(|| fact.payload.get("summary"))
            .and_then(|v| v.as_str())
            .map(|text| !text.trim().is_empty())
            .unwrap_or(false)
}

fn knowledge_conflict_evidence(ctx: &BuddyJobContext) -> Option<AutonomousEvidence> {
    let facts = stable_facts(ctx.facts.iter().filter(|fact| {
        matches!(
            memory_route_for_fact_kind(fact.kind),
            Some(MemoryRouting::KnowledgeConflictResolver)
        ) && stale_conflict_fact_actionable(fact)
    }))
    .into_iter()
    .take(MAX_FACT_EVIDENCE_ITEMS)
    .collect::<Vec<_>>();
    if facts.is_empty() {
        return None;
    }

    let mut lines = Vec::new();
    if ctx.pulse.memory.stale_conflicts > 0 {
        lines.push(format!(
            "memory_pulse stale_conflicts={}",
            ctx.pulse.memory.stale_conflicts
        ));
    }
    for fact in facts {
        lines.push(memory_fact_to_evidence_line(fact));
    }
    Some(AutonomousEvidence {
        prompt: "Review stale/conflicting knowledge signals and propose a safe resolution plan. Use only doc ids and conflict summaries from evidence; never infer full document bodies.".to_string(),
        evidence: lines.join("\n"),
    })
}

fn build_spec(workflow_id: &str, evidence: AutonomousEvidence) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(workflow_id);
    AutonomousBuddyChatSpec::new(
        workflow_id,
        meta.map(|meta| meta.title)
            .unwrap_or("Companion Autonomous Report"),
        evidence.prompt,
        evidence.evidence,
    )
    .with_display(
        meta.map(|meta| meta.icon).unwrap_or("🤖"),
        meta.map(|meta| meta.badge).unwrap_or("Companion"),
        meta.map(|meta| meta.priority).unwrap_or("normal"),
    )
}

fn autonomous_activity(
    spec: &AutonomousBuddyChatSpec,
    chat_id: &str,
    identity_name: &str,
) -> BuddyActivity {
    BuddyActivity {
        icon: spec.icon.clone(),
        title: format!("{} report saved", spec.title),
        description: format!(
            "{} saved an autonomous {} report in chat {}.",
            identity_name, spec.badge, chat_id
        ),
        timestamp: Utc::now().to_rfc3339(),
        activity_type: spec.workflow_id.clone(),
        chat_id: Some(chat_id.to_string()),
    }
}

fn autonomous_runtime_event(
    spec: &AutonomousBuddyChatSpec,
    chat_id: &str,
    identity_name: &str,
) -> BuddyRuntimeEvent {
    let mut event = make_runtime_event(
        &spec.workflow_id,
        &format!("{} report ready", spec.title),
        "buddy",
        &format!("{}:{}", spec.workflow_id, spec.signal_hash),
        "completed",
        Some(&spec.priority),
    );
    event.description = Some(format!(
        "Open {}'s saved chat for {} details.",
        identity_name, spec.title
    ));
    event.chat_id = Some(chat_id.to_string());
    event
}

async fn execute_autonomous_spec(
    gcx: Arc<ARwLock<GlobalContext>>,
    ctx: &BuddyJobContext,
    spec: AutonomousBuddyChatSpec,
) -> BuddyJobResult {
    if same_signal(ctx, &spec.signal_hash) {
        return preserve_last_result(ctx);
    }
    let chat_id = match run_autonomous_buddy_chat(gcx, spec.clone()).await {
        Ok(chat_id) => chat_id,
        Err(err) => {
            tracing::warn!("autonomous buddy job {} failed: {}", spec.workflow_id, err);
            return BuddyJobResult::default();
        }
    };
    let last = AutonomousLastResult::new(spec.signal_hash.clone(), chat_id.clone());
    BuddyJobResult {
        activity: Some(autonomous_activity(&spec, &chat_id, &ctx.identity_name)),
        runtime_event: Some(autonomous_runtime_event(
            &spec,
            &chat_id,
            &ctx.identity_name,
        )),
        last_result: Some(serialize_last_autonomous_result(&last)),
        ..Default::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrajectoryUserSnippet {
    trajectory_id: String,
    title: String,
    mode: String,
    updated_at: String,
    text: String,
}

#[derive(Debug, Clone, PartialEq)]
struct PreferenceCandidate {
    statement: String,
    evidence: String,
    confidence: f32,
}

fn first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    let mut end = trimmed.len();
    for marker in ['.', '!', '?', '\n'] {
        if let Some(idx) = trimmed.find(marker) {
            end = end.min(idx + marker.len_utf8());
        }
    }
    crate::llm::safe_truncate(trimmed[..end].trim(), 240).to_string()
}

fn explicit_preference_confidence(text: &str) -> Option<f32> {
    let lower = text.to_lowercase();
    let strong = [
        "i prefer",
        "i'd prefer",
        "i would prefer",
        "always ",
        "never ",
        "please always",
    ];
    if strong.iter().any(|cue| lower.contains(cue)) {
        return Some(0.90);
    }
    let medium = [
        "please use",
        "please keep",
        "please avoid",
        "do not ",
        "don't ",
    ];
    if medium.iter().any(|cue| lower.contains(cue)) {
        return Some(0.86);
    }
    None
}

fn preference_like(text: &str) -> bool {
    explicit_preference_confidence(text).is_some()
}

fn behavior_candidate_status(confidence: f32) -> MemoryCandidateStatus {
    if confidence >= 0.95 {
        MemoryCandidateStatus::Approved
    } else {
        MemoryCandidateStatus::Proposed
    }
}

fn behavior_preference_candidates(snippets: &[TrajectoryUserSnippet]) -> Vec<PreferenceCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for snippet in snippets {
        let Some(confidence) = explicit_preference_confidence(&snippet.text) else {
            continue;
        };
        let statement = redact_and_cap_text(&first_sentence(&snippet.text), 240);
        if !crate::memories::preference_statement_is_safe(&statement, confidence) {
            continue;
        }
        let normalized = crate::memories::normalize_preference_text_for_dedupe(&statement);
        if !seen.insert(normalized) {
            continue;
        }
        let evidence = redact_and_cap_text(
            &format!(
                "User-authored snippet in {} ({})",
                snippet.trajectory_id, snippet.title
            ),
            240,
        );
        candidates.push(PreferenceCandidate {
            statement,
            evidence,
            confidence,
        });
    }
    candidates
}

async fn enqueue_behavior_preferences(
    gcx: Arc<ARwLock<GlobalContext>>,
    candidates: &[PreferenceCandidate],
) -> usize {
    let Some(project_root) = crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
    else {
        return 0;
    };
    let mut queued = 0;
    for candidate in candidates {
        if queued >= MAX_BEHAVIOR_PREFERENCE_WRITES {
            break;
        }
        let content = if candidate.evidence.trim().is_empty() {
            format!(
                "# {}\n\nConfidence: {:.2}",
                candidate.statement, candidate.confidence
            )
        } else {
            format!(
                "# {}\n\nEvidence: {}\n\nConfidence: {:.2}",
                candidate.statement, candidate.evidence, candidate.confidence
            )
        };
        let memory_candidate = MemoryCandidate {
            candidate_id: format!(
                "memcand_behavior_{}",
                signal_hash([candidate.statement.as_str(), candidate.evidence.as_str()])
            ),
            source: MemorySource::BehaviorLearner,
            title: candidate.statement.clone(),
            content,
            tags: vec![
                "preference".to_string(),
                "buddy".to_string(),
                "behavior_learner".to_string(),
            ],
            kind: "preference".to_string(),
            source_id: Some(format!(
                "behavior_learner:{}",
                crate::memories::normalize_preference_text_for_dedupe(&candidate.statement)
            )),
            confidence: candidate.confidence,
            status: behavior_candidate_status(candidate.confidence),
            ..Default::default()
        };
        let op = memory_candidate.into_create_memory_op(
            format!(
                "memop_behavior_{}",
                signal_hash([candidate.statement.as_str()])
            ),
            candidate.evidence.clone(),
            Utc::now().to_rfc3339(),
        );
        match crate::buddy::storage::enqueue_memory_op(&project_root, op).await {
            Ok(_) => queued += 1,
            Err(err) => tracing::warn!("buddy behavior learner preference enqueue failed: {}", err),
        }
    }
    queued
}

fn extract_text_from_trajectory_content(content: &serde_json::Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    if let Some(items) = content.as_array() {
        let parts = items
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|v| v.as_str()) == Some("image_url")
                    || item
                        .get("m_type")
                        .and_then(|v| v.as_str())
                        .map(|kind| kind.starts_with("image/"))
                        .unwrap_or(false)
                {
                    return Some("[image]".to_string());
                }
                item.get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| item.get("m_content").and_then(|v| v.as_str()))
                    .map(ToString::to_string)
            })
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return Some(parts.join("\n\n"));
        }
    }
    None
}

async fn read_bounded_trajectory_json(path: &Path, max_bytes: u64) -> Option<serde_json::Value> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    if metadata.len() > max_bytes {
        return None;
    }
    let content = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str::<serde_json::Value>(&content).ok()
}

#[derive(Debug, Clone)]
struct BehaviorTrajectoryMeta {
    id: String,
    title: String,
    mode: String,
    updated_at: String,
    path: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct BehaviorTrajectoryCandidate {
    modified_key: u64,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BehaviorTrajectoryScanStats {
    visited_entries: usize,
    matching_files_considered: usize,
}

fn parse_json_string_at(content: &str, start: usize) -> Option<(String, usize)> {
    if content.as_bytes().get(start) != Some(&b'"') {
        return None;
    }
    let mut escaped = false;
    for (offset, ch) in content[start + 1..].char_indices() {
        let idx = start + 1 + offset;
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            let raw = &content[start..=idx];
            let parsed = serde_json::from_str::<String>(raw).ok()?;
            return Some((parsed, idx + 1));
        }
    }
    None
}

fn skip_json_ws(content: &str, mut idx: usize) -> usize {
    let bytes = content.as_bytes();
    while bytes
        .get(idx)
        .map(|byte| byte.is_ascii_whitespace())
        .unwrap_or(false)
    {
        idx += 1;
    }
    idx
}

fn skip_json_value(content: &str, idx: usize) -> Option<usize> {
    let mut idx = skip_json_ws(content, idx);
    let bytes = content.as_bytes();
    match bytes.get(idx).copied()? {
        b'"' => parse_json_string_at(content, idx).map(|(_, next)| next),
        b'{' | b'[' => {
            let mut depth = 0usize;
            while idx < bytes.len() {
                match bytes[idx] {
                    b'"' => idx = parse_json_string_at(content, idx)?.1,
                    b'{' | b'[' => {
                        depth += 1;
                        idx += 1;
                    }
                    b'}' | b']' => {
                        depth = depth.checked_sub(1)?;
                        idx += 1;
                        if depth == 0 {
                            return Some(idx);
                        }
                    }
                    _ => idx += 1,
                }
            }
            None
        }
        _ => {
            while idx < bytes.len() && !matches!(bytes[idx], b',' | b'}') {
                idx += 1;
            }
            Some(idx)
        }
    }
}

fn top_level_json_strings(content: &str, fields: &[&str]) -> HashMap<String, String> {
    let bytes = content.as_bytes();
    let mut idx = skip_json_ws(content, 0);
    let mut values = HashMap::new();
    if bytes.get(idx) != Some(&b'{') {
        return values;
    }
    idx += 1;
    while idx < bytes.len() {
        idx = skip_json_ws(content, idx);
        match bytes.get(idx).copied() {
            Some(b'}') | None => break,
            Some(b',') => {
                idx += 1;
                continue;
            }
            Some(b'"') => {}
            _ => break,
        }
        let Some((key, next)) = parse_json_string_at(content, idx) else {
            break;
        };
        idx = skip_json_ws(content, next);
        if bytes.get(idx) != Some(&b':') {
            break;
        }
        idx = skip_json_ws(content, idx + 1);
        if fields.contains(&key.as_str()) && bytes.get(idx) == Some(&b'"') {
            let Some((value, next)) = parse_json_string_at(content, idx) else {
                break;
            };
            values.entry(key).or_insert(value);
            idx = next;
        } else {
            let Some(next) = skip_json_value(content, idx) else {
                break;
            };
            idx = next;
        }
    }
    values
}

fn top_level_json_string(content: &str, fields: &[&str]) -> Option<String> {
    let values = top_level_json_strings(content, fields);
    fields.iter().find_map(|field| values.get(*field).cloned())
}

fn parse_behavior_trajectory_meta(content: &str, path: PathBuf) -> Option<BehaviorTrajectoryMeta> {
    let id = top_level_json_string(content, &["id", "chat_id"])?;
    let updated_at =
        top_level_json_string(content, &["updated_at", "last_message_at", "created_at"])?;
    Some(BehaviorTrajectoryMeta {
        id,
        title: top_level_json_string(content, &["title"]).unwrap_or_default(),
        mode: top_level_json_string(content, &["mode"]).unwrap_or_default(),
        updated_at,
        path,
    })
}

#[cfg(test)]
fn collect_behavior_trajectory_metas_from_dir(
    dir: &Path,
    metas: &mut Vec<BehaviorTrajectoryMeta>,
    seen: &mut HashSet<String>,
) {
    let candidates = collect_behavior_trajectory_candidates_from_dirs(
        std::slice::from_ref(&dir.to_path_buf()),
        MAX_BEHAVIOR_TRAJECTORY_SCAN_FILES,
    );
    collect_behavior_trajectory_metas_from_candidates(candidates, metas, seen);
}

fn system_time_key(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn push_behavior_trajectory_candidate(
    heap: &mut BinaryHeap<Reverse<BehaviorTrajectoryCandidate>>,
    candidate: BehaviorTrajectoryCandidate,
    max_files: usize,
) {
    if max_files == 0 {
        return;
    }
    if heap.len() < max_files {
        heap.push(Reverse(candidate));
        return;
    }
    let should_replace = heap
        .peek()
        .map(|oldest| candidate > oldest.0)
        .unwrap_or(false);
    if should_replace {
        heap.pop();
        heap.push(Reverse(candidate));
    }
}

fn collect_behavior_trajectory_candidates_from_dir(
    dir: &Path,
    heap: &mut BinaryHeap<Reverse<BehaviorTrajectoryCandidate>>,
    max_files: usize,
    max_visited_entries: usize,
    stats: &mut BehaviorTrajectoryScanStats,
) {
    if max_visited_entries == 0 || stats.visited_entries >= max_visited_entries {
        return;
    }
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        if stats.visited_entries >= max_visited_entries {
            return;
        }
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        let remaining = max_visited_entries.saturating_sub(stats.visited_entries);
        let mut entries = Vec::new();
        for entry in read_dir.take(remaining) {
            stats.visited_entries += 1;
            let Ok(entry) = entry else {
                continue;
            };
            entries.push(entry);
        }
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                dirs.push(path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if metadata.len() > MAX_BEHAVIOR_TRAJECTORY_BYTES {
                continue;
            }
            stats.matching_files_considered += 1;
            let modified_key = metadata.modified().map(system_time_key).unwrap_or_default();
            push_behavior_trajectory_candidate(
                heap,
                BehaviorTrajectoryCandidate { modified_key, path },
                max_files,
            );
        }
    }
}

fn collect_behavior_trajectory_candidates_from_dirs(
    dirs: &[PathBuf],
    max_files: usize,
) -> Vec<BehaviorTrajectoryCandidate> {
    collect_behavior_trajectory_candidates_from_dirs_with_stats(
        dirs,
        max_files,
        MAX_BEHAVIOR_TRAJECTORY_SCAN_ENTRIES,
    )
    .0
}

fn collect_behavior_trajectory_candidates_from_dirs_with_stats(
    dirs: &[PathBuf],
    max_files: usize,
    max_visited_entries: usize,
) -> (
    Vec<BehaviorTrajectoryCandidate>,
    BehaviorTrajectoryScanStats,
) {
    let mut heap = BinaryHeap::new();
    let mut stats = BehaviorTrajectoryScanStats::default();
    for dir in dirs {
        if stats.visited_entries >= max_visited_entries {
            break;
        }
        let Ok(metadata) = std::fs::symlink_metadata(dir) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        collect_behavior_trajectory_candidates_from_dir(
            dir,
            &mut heap,
            max_files,
            max_visited_entries,
            &mut stats,
        );
    }
    let mut candidates = heap
        .into_iter()
        .map(|Reverse(candidate)| candidate)
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.modified_key
            .cmp(&a.modified_key)
            .then_with(|| a.path.cmp(&b.path))
    });
    (candidates, stats)
}

fn parse_behavior_trajectory_meta_from_path(path: &Path) -> Option<BehaviorTrajectoryMeta> {
    let Ok(file) = std::fs::File::open(path) else {
        return None;
    };
    let mut content = String::new();
    let mut reader = std::io::Read::take(file, MAX_BEHAVIOR_TRAJECTORY_META_BYTES);
    if std::io::Read::read_to_string(&mut reader, &mut content).is_err() {
        return None;
    }
    parse_behavior_trajectory_meta(&content, path.to_path_buf()).or_else(|| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| parse_behavior_trajectory_meta(&content, path.to_path_buf()))
    })
}

fn collect_behavior_trajectory_metas_from_candidates(
    candidates: Vec<BehaviorTrajectoryCandidate>,
    metas: &mut Vec<BehaviorTrajectoryMeta>,
    seen: &mut HashSet<String>,
) {
    for candidate in candidates {
        let Some(meta) = parse_behavior_trajectory_meta_from_path(&candidate.path) else {
            continue;
        };
        if seen.insert(meta.id.clone()) {
            metas.push(meta);
        }
    }
}

fn collect_behavior_task_trajectory_dirs(
    tasks_dirs: &[PathBuf],
    max_visited_entries: usize,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut visited_entries = 0usize;
    for tasks_dir in tasks_dirs {
        if visited_entries >= max_visited_entries {
            break;
        }
        let Ok(tasks_metadata) = std::fs::symlink_metadata(tasks_dir) else {
            continue;
        };
        if tasks_metadata.file_type().is_symlink() || !tasks_metadata.is_dir() {
            continue;
        }
        let Ok(read_dir) = std::fs::read_dir(tasks_dir) else {
            continue;
        };
        let remaining = max_visited_entries.saturating_sub(visited_entries);
        let mut task_entries = Vec::new();
        for task_entry in read_dir.take(remaining) {
            visited_entries += 1;
            let Ok(task_entry) = task_entry else {
                continue;
            };
            task_entries.push(task_entry);
        }
        task_entries.sort_by_key(|entry| entry.path());
        for task_entry in task_entries {
            let task_dir = task_entry.path();
            let Ok(task_metadata) = std::fs::symlink_metadata(&task_dir) else {
                continue;
            };
            if task_metadata.file_type().is_symlink() || !task_metadata.is_dir() {
                continue;
            }
            for role in ["planner", "agents"] {
                let role_dir = task_dir.join("trajectories").join(role);
                let Ok(role_metadata) = std::fs::symlink_metadata(&role_dir) else {
                    continue;
                };
                if !role_metadata.file_type().is_symlink() && role_metadata.is_dir() {
                    dirs.push(role_dir);
                }
            }
        }
    }
    dirs
}

async fn collect_behavior_trajectory_metas(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Vec<BehaviorTrajectoryMeta> {
    let mut dirs = crate::chat::trajectories::get_all_trajectories_dirs(gcx.clone()).await;
    let tasks_dirs = crate::tasks::storage::get_all_tasks_dirs(gcx).await;
    tokio::task::spawn_blocking(move || {
        dirs.extend(collect_behavior_task_trajectory_dirs(
            &tasks_dirs,
            MAX_BEHAVIOR_TASK_DIR_SCAN_ENTRIES,
        ));
        let mut metas = Vec::new();
        let mut seen = HashSet::new();
        let candidates = collect_behavior_trajectory_candidates_from_dirs(
            &dirs,
            MAX_BEHAVIOR_TRAJECTORY_SCAN_FILES,
        );
        collect_behavior_trajectory_metas_from_candidates(candidates, &mut metas, &mut seen);
        metas.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        metas
    })
    .await
    .unwrap_or_default()
}

async fn collect_recent_user_snippets(
    gcx: Arc<ARwLock<GlobalContext>>,
    max_snippets: usize,
) -> Vec<TrajectoryUserSnippet> {
    let metas = collect_behavior_trajectory_metas(gcx).await;
    let mut snippets = Vec::new();
    for meta in metas.into_iter().take(30) {
        let Some(value) =
            read_bounded_trajectory_json(&meta.path, MAX_BEHAVIOR_TRAJECTORY_BYTES).await
        else {
            continue;
        };
        let Some(messages) = value.get("messages").and_then(|v| v.as_array()) else {
            continue;
        };
        for msg in messages.iter().rev() {
            if msg.get("role").and_then(|role| role.as_str()) != Some("user") {
                continue;
            }
            let Some(text) = msg
                .get("content")
                .and_then(extract_text_from_trajectory_content)
            else {
                continue;
            };
            let text = redact_and_cap_text(&text, 300);
            if text.trim().is_empty() {
                continue;
            }
            snippets.push(TrajectoryUserSnippet {
                trajectory_id: meta.id.clone(),
                title: redact_and_cap_text(&meta.title, 120),
                mode: meta.mode.clone(),
                updated_at: meta.updated_at.clone(),
                text,
            });
            if snippets.len() >= max_snippets {
                return snippets;
            }
        }
    }
    snippets
}

fn behavior_evidence_from_snippets(
    snippets: &[TrajectoryUserSnippet],
) -> Option<AutonomousEvidence> {
    let preference_like_count = snippets
        .iter()
        .filter(|snippet| preference_like(&snippet.text))
        .count();
    if snippets.len() < 4 && preference_like_count < 2 {
        return None;
    }
    let mut lines = vec![format!(
        "recent_user_snippets={} preference_like_snippets={}",
        snippets.len(),
        preference_like_count
    )];
    for snippet in snippets.iter().take(MAX_TRAJECTORY_SNIPPETS) {
        lines.push(format!(
            "trajectory={} title={} mode={} updated={} user_snippet={}",
            redact_and_cap_text(&snippet.trajectory_id, 80),
            snippet.title,
            redact_and_cap_text(&snippet.mode, 80),
            redact_and_cap_text(&snippet.updated_at, 80),
            snippet.text
        ));
    }
    Some(AutonomousEvidence {
        prompt: "Learn stable user behavior and preference signals from recent user-authored snippets only. Produce a concise report and identify only high-confidence preferences that are directly supported by the snippets.".to_string(),
        evidence: lines.join("\n"),
    })
}

async fn behavior_learner_evidence(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Option<(AutonomousEvidence, Vec<PreferenceCandidate>)> {
    let snippets = collect_recent_user_snippets(gcx, MAX_TRAJECTORY_SNIPPETS).await;
    let evidence = behavior_evidence_from_snippets(&snippets)?;
    let candidates = behavior_preference_candidates(&snippets);
    Some((evidence, candidates))
}

fn habit_fact_relevant(kind: BuddyFactKind) -> bool {
    matches!(
        kind,
        BuddyFactKind::TaskStuck
            | BuddyFactKind::TaskAbandoned
            | BuddyFactKind::TrajectoryClutter
            | BuddyFactKind::ChatRetryStreak
            | BuddyFactKind::DiagnosticCluster
            | BuddyFactKind::FrontendErrorBurst
    )
}

fn habit_evidence(ctx: &BuddyJobContext) -> Option<AutonomousEvidence> {
    let mut lines = vec![format!(
        "task_pulse total={} stuck={} abandoned={} trajectory_total={} untitled={} oldest_age_days={} diagnostics_last_hour={} top_error_types={}",
        ctx.pulse.tasks.total,
        ctx.pulse.tasks.stuck,
        ctx.pulse.tasks.abandoned,
        ctx.pulse.trajectories.total,
        ctx.pulse.trajectories.untitled,
        ctx.pulse.trajectories.oldest_age_days,
        ctx.pulse.diagnostics.last_hour,
        ctx.pulse.diagnostics.top_error_types.join(", ")
    )];
    let mut task_statuses = ctx.pulse.tasks.by_status.iter().collect::<Vec<_>>();
    task_statuses.sort_by(|a, b| a.0.cmp(b.0));
    for (status, count) in task_statuses {
        lines.push(format!(
            "task_status {}={}",
            redact_and_cap_text(status, 80),
            count
        ));
    }
    for fact in stable_facts(
        ctx.facts
            .iter()
            .filter(|fact| habit_fact_relevant(fact.kind)),
    )
    .into_iter()
    .take(MAX_FACT_EVIDENCE_ITEMS)
    {
        let mut fact_lines = vec![format!(
            "fact kind={} key={} source={} confidence={:.2}",
            fact_kind_name(fact.kind),
            redact_and_cap_text(&fact.key, 160),
            fact.source,
            fact.confidence
        )];
        if let Some(count) = fact.payload.get("count").and_then(|v| v.as_u64()) {
            fact_lines.push(format!("count={}", count));
        }
        if let Some(summary) = fact.payload.get("summary") {
            push_payload_string(&mut fact_lines, "summary", summary);
        }
        if let Some(pattern) = fact.payload.get("pattern") {
            push_payload_string(&mut fact_lines, "pattern", pattern);
        }
        lines.push(fact_lines.join("; "));
    }
    let mut diag_counts: HashMap<String, usize> = HashMap::new();
    for diag in &ctx.recent_diagnostics {
        *diag_counts.entry(diag.error_type.clone()).or_default() += 1;
    }
    let mut diag_counts = diag_counts.into_iter().collect::<Vec<_>>();
    diag_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (error_type, count) in diag_counts.into_iter().take(5) {
        lines.push(format!(
            "diagnostic_pattern type={} count={}",
            redact_and_cap_text(&error_type, 120),
            count
        ));
    }
    let repeated_diagnostics = ctx.recent_diagnostics.len() >= 5;
    let task_pattern = ctx.pulse.tasks.stuck > 0 || ctx.pulse.tasks.abandoned > 0;
    let trajectory_pattern =
        ctx.pulse.trajectories.untitled >= 5 || ctx.pulse.trajectories.oldest_age_days >= 30;
    let fact_pattern = ctx.facts.iter().any(|fact| habit_fact_relevant(fact.kind));
    if !(repeated_diagnostics || task_pattern || trajectory_pattern || fact_pattern) {
        return None;
    }
    Some(AutonomousEvidence {
        prompt: "Look for repeated workflow habits or friction patterns from metadata only. Coach the user with practical next-step suggestions without citing raw tool outputs or private content.".to_string(),
        evidence: lines.join("\n"),
    })
}

fn bucket_rate(part: usize, total: usize) -> String {
    if total == 0 {
        return "0%".to_string();
    }
    let rate = part as f64 * 100.0 / total as f64;
    if rate == 0.0 {
        "0%".to_string()
    } else if rate < 10.0 {
        "under_10%".to_string()
    } else if rate < 25.0 {
        "10-24%".to_string()
    } else if rate < 50.0 {
        "25-49%".to_string()
    } else if rate < 75.0 {
        "50-74%".to_string()
    } else {
        "75%+".to_string()
    }
}

fn bucket_count(value: usize) -> String {
    match value {
        0 => "0".to_string(),
        1..=4 => "1-4".to_string(),
        5..=9 => "5-9".to_string(),
        10..=24 => "10-24".to_string(),
        25..=49 => "25-49".to_string(),
        50..=99 => "50-99".to_string(),
        100..=249 => "100-249".to_string(),
        250..=499 => "250-499".to_string(),
        _ => "500+".to_string(),
    }
}

fn bucket_tokens(value: usize) -> String {
    match value {
        0 => "0".to_string(),
        1..=9_999 => "1-9k".to_string(),
        10_000..=49_999 => "10k-49k".to_string(),
        50_000..=99_999 => "50k-99k".to_string(),
        100_000..=249_999 => "100k-249k".to_string(),
        250_000..=499_999 => "250k-499k".to_string(),
        500_000..=999_999 => "500k-999k".to_string(),
        _ => "1m+".to_string(),
    }
}

fn bucket_cost(value: f64) -> String {
    if value <= 0.0 {
        "0".to_string()
    } else if value < 0.10 {
        "under_0.10".to_string()
    } else if value < 0.50 {
        "0.10-0.49".to_string()
    } else if value < 1.00 {
        "0.50-0.99".to_string()
    } else if value < 5.00 {
        "1-4.99".to_string()
    } else if value < 10.00 {
        "5-9.99".to_string()
    } else {
        "10+".to_string()
    }
}

fn bucket_duration_ms(value: u64) -> String {
    match value {
        0..=4_999 => "under_5s".to_string(),
        5_000..=9_999 => "5s-9s".to_string(),
        10_000..=19_999 => "10s-19s".to_string(),
        20_000..=39_999 => "20s-39s".to_string(),
        40_000..=59_999 => "40s-59s".to_string(),
        _ => "60s+".to_string(),
    }
}

fn is_autonomous_buddy_workflow_identifier(identifier: &str) -> bool {
    let normalized = identifier.to_ascii_lowercase();
    is_autonomous_workflow_id(&normalized)
        || normalized
            .strip_prefix("buddy-")
            .and_then(|rest| rest.split_once('-').map(|(workflow_id, _)| workflow_id))
            .map(is_autonomous_workflow_id)
            .unwrap_or(false)
}

fn is_buddy_autonomous_report_event(event: &LlmCallEvent) -> bool {
    let identifiers = [
        event.chat_id.as_str(),
        event.root_chat_id.as_deref().unwrap_or_default(),
        event.task_id.as_deref().unwrap_or_default(),
        event.task_role.as_deref().unwrap_or_default(),
        event.agent_id.as_deref().unwrap_or_default(),
        event.card_id.as_deref().unwrap_or_default(),
    ];
    identifiers
        .iter()
        .any(|identifier| is_autonomous_buddy_workflow_identifier(identifier))
}

fn model_cost_input_events(events: &[LlmCallEvent]) -> Vec<LlmCallEvent> {
    let mut filtered = events
        .iter()
        .filter(|event| !is_buddy_autonomous_report_event(event))
        .cloned()
        .collect::<Vec<_>>();
    filtered.sort_by(|a, b| {
        a.ts_start
            .cmp(&b.ts_start)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.chat_id.cmp(&b.chat_id))
    });
    if filtered.len() > MODEL_COST_RECENT_EVENT_LIMIT {
        filtered.drain(0..filtered.len() - MODEL_COST_RECENT_EVENT_LIMIT);
    }
    filtered
}

fn model_cost_evidence_from_events(events: &[LlmCallEvent]) -> Option<AutonomousEvidence> {
    let events = model_cost_input_events(events);
    if events.len() < 5 {
        return None;
    }
    let summary = crate::stats::reader::aggregate_summary(&events, None, None);
    let totals = &summary.totals;
    let high_failure = totals.failed_calls >= 3
        && totals.failed_calls as f64 / totals.total_calls.max(1) as f64 >= 0.25;
    let high_latency = totals.avg_duration_ms >= 20_000;
    let high_tokens = totals.total_tokens >= 250_000;
    let high_cost = totals.total_cost_usd >= 1.0;
    let model_issue = summary.by_model.iter().any(|model| {
        (model.failed_calls >= 3
            && model.failed_calls as f64 / model.total_calls.max(1) as f64 >= 0.25)
            || model.avg_duration_ms >= 20_000
            || model.total_tokens >= 150_000
            || model.total_cost_usd >= 0.75
    });
    if !(high_failure || high_latency || high_tokens || high_cost || model_issue) {
        return None;
    }

    let mut lines = vec![format!(
        "recent_window max_events={} calls_bucket={} successful_bucket={} failed_bucket={} failure_rate_bucket={} total_tokens_bucket={} prompt_tokens_bucket={} completion_tokens_bucket={} cache_read_tokens_bucket={} cache_creation_tokens_bucket={} total_cost_usd_bucket={} avg_duration_bucket={} conversations_bucket={} messages_sent_bucket={}",
        MODEL_COST_RECENT_EVENT_LIMIT,
        bucket_count(totals.total_calls),
        bucket_count(totals.successful_calls),
        bucket_count(totals.failed_calls),
        bucket_rate(totals.failed_calls, totals.total_calls),
        bucket_tokens(totals.total_tokens),
        bucket_tokens(totals.total_prompt_tokens),
        bucket_tokens(totals.total_completion_tokens),
        bucket_tokens(totals.total_cache_read_tokens),
        bucket_tokens(totals.total_cache_creation_tokens),
        bucket_cost(totals.total_cost_usd),
        bucket_duration_ms(totals.avg_duration_ms),
        bucket_count(totals.total_conversations),
        bucket_count(totals.total_messages_sent)
    )];
    for model in summary.by_model.iter().take(8) {
        lines.push(format!(
            "model id={} provider={} model={} calls_bucket={} failed_bucket={} failure_rate_bucket={} tokens_bucket={} cost_usd_bucket={} avg_duration_bucket={}",
            redact_and_cap_text(&model.model_id, 120),
            redact_and_cap_text(&model.provider, 80),
            redact_and_cap_text(&model.model, 100),
            bucket_count(model.total_calls),
            bucket_count(model.failed_calls),
            bucket_rate(model.failed_calls, model.total_calls),
            bucket_tokens(model.total_tokens),
            bucket_cost(model.total_cost_usd),
            bucket_duration_ms(model.avg_duration_ms)
        ));
    }
    for provider in summary.by_provider.iter().take(6) {
        lines.push(format!(
            "provider name={} calls_bucket={} failed_bucket={} failure_rate_bucket={} tokens_bucket={} cost_usd_bucket={} total_duration_bucket={}",
            redact_and_cap_text(&provider.provider, 80),
            bucket_count(provider.total_calls),
            bucket_count(provider.failed_calls),
            bucket_rate(provider.failed_calls, provider.total_calls),
            bucket_tokens(provider.total_tokens),
            bucket_cost(provider.total_cost_usd),
            bucket_duration_ms(provider.total_duration_ms)
        ));
    }
    Some(AutonomousEvidence {
        prompt: "Analyze aggregate LLM usage, reliability, latency, token, and spend signals. Recommend model or configuration optimizations using aggregate stats only; do not infer message content.".to_string(),
        evidence: lines.join("\n"),
    })
}

async fn model_cost_evidence(gcx: Arc<ARwLock<GlobalContext>>) -> Option<AutonomousEvidence> {
    let stats_dirs = crate::stats::get_stats_dirs_for_read(gcx).await;
    let events = tokio::task::spawn_blocking(move || {
        crate::stats::reader::read_recent_stats_events_from_dirs(
            &stats_dirs,
            MODEL_COST_RECENT_EVENT_LIMIT,
        )
    })
    .await
    .unwrap_or_default();
    model_cost_evidence_from_events(&events)
}

#[async_trait::async_trait]
impl BuddyJob for BuddyMemoryGardenerJob {
    fn id(&self) -> &str {
        MEMORY_GARDENER_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        MEMORY_COOLDOWN_SECS
    }

    fn priority(&self) -> u32 {
        20
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        let Some(evidence) = memory_gardener_evidence(ctx) else {
            return false;
        };
        !same_signal(ctx, &build_spec(self.id(), evidence).signal_hash)
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let Some(evidence) = memory_gardener_evidence(&ctx) else {
            return BuddyJobResult::default();
        };
        execute_autonomous_spec(gcx, &ctx, build_spec(self.id(), evidence)).await
    }
}

#[async_trait::async_trait]
impl BuddyJob for BuddyKnowledgeConflictResolverJob {
    fn id(&self) -> &str {
        KNOWLEDGE_CONFLICT_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        MEMORY_COOLDOWN_SECS
    }

    fn priority(&self) -> u32 {
        21
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        let Some(evidence) = knowledge_conflict_evidence(ctx) else {
            return false;
        };
        !same_signal(ctx, &build_spec(self.id(), evidence).signal_hash)
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let Some(evidence) = knowledge_conflict_evidence(&ctx) else {
            return BuddyJobResult::default();
        };
        execute_autonomous_spec(gcx, &ctx, build_spec(self.id(), evidence)).await
    }
}

#[async_trait::async_trait]
impl BuddyJob for BuddyBehaviorLearnerJob {
    fn id(&self) -> &str {
        BEHAVIOR_LEARNER_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        BEHAVIOR_COOLDOWN_SECS
    }

    fn priority(&self) -> u32 {
        22
    }

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        ctx.pulse.trajectories.total >= 4
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let Some((evidence, candidates)) = behavior_learner_evidence(gcx.clone()).await else {
            return BuddyJobResult::default();
        };
        let spec = build_spec(self.id(), evidence);
        if same_signal(&ctx, &spec.signal_hash) {
            return preserve_last_result(&ctx);
        }
        let mut result = execute_autonomous_spec(gcx.clone(), &ctx, spec).await;
        if result.last_result.is_some() {
            let queued = enqueue_behavior_preferences(gcx, &candidates).await;
            if queued > 0 {
                if let Some(activity) = result.activity.as_mut() {
                    activity.description = format!(
                        "{} Queued {} high-confidence preference candidate{} for review.",
                        activity.description,
                        queued,
                        if queued == 1 { "" } else { "s" }
                    );
                }
            }
        }
        result
    }
}

#[async_trait::async_trait]
impl BuddyJob for BuddyUserHabitCoachJob {
    fn id(&self) -> &str {
        USER_HABIT_COACH_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        MEMORY_COOLDOWN_SECS
    }

    fn priority(&self) -> u32 {
        23
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        let Some(evidence) = habit_evidence(ctx) else {
            return false;
        };
        !same_signal(ctx, &build_spec(self.id(), evidence).signal_hash)
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let Some(evidence) = habit_evidence(&ctx) else {
            return BuddyJobResult::default();
        };
        execute_autonomous_spec(gcx, &ctx, build_spec(self.id(), evidence)).await
    }
}

#[async_trait::async_trait]
impl BuddyJob for BuddyModelCostOptimizerJob {
    fn id(&self) -> &str {
        MODEL_COST_OPTIMIZER_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        MEMORY_COOLDOWN_SECS
    }

    fn priority(&self) -> u32 {
        24
    }

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        ctx.total_workflow_runs >= 5 || ctx.pulse.providers.quota_warnings > 0
    }

    async fn execute(
        &self,
        gcx: Arc<ARwLock<GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let Some(evidence) = model_cost_evidence(gcx.clone()).await else {
            return BuddyJobResult::default();
        };
        execute_autonomous_spec(gcx, &ctx, build_spec(self.id(), evidence)).await
    }
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
    pub source: String,
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

#[derive(Debug, Clone)]
struct SecurityScanContent {
    source: &'static str,
    content: String,
}

#[derive(Debug, Clone, Copy)]
struct AutonomousJobDefinition {
    meta: &'static AutonomousWorkflowMeta,
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
        meta: autonomous_workflow_meta(ERROR_DETECTIVE_WORKFLOW_ID).unwrap(),
        cooldown_seconds: 15 * 60,
        scheduler_priority: 5,
        prompt: "Analyze the diagnostic pattern using only the summaries below. Explain the likely failure cluster, risk, and the smallest safe next checks. Do not invent log details that are not in evidence.",
    }
}

fn security_whisperer_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        meta: autonomous_workflow_meta(SECURITY_WHISPERER_WORKFLOW_ID).unwrap(),
        cooldown_seconds: 30 * 60,
        scheduler_priority: 5,
        prompt: "Review the redacted local security findings. Highlight immediate leakage risk, containment steps, and safer follow-up. Never ask for or repeat raw secret values.",
    }
}

fn setup_coach_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        meta: autonomous_workflow_meta(SETUP_COACH_WORKFLOW_ID).unwrap(),
        cooldown_seconds: 2 * 60 * 60,
        scheduler_priority: 5,
        prompt: "Review the local setup checklist and recommend the next onboarding step. Keep it practical and based only on the checklist.",
    }
}

fn dependency_radar_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        meta: autonomous_workflow_meta(DEPENDENCY_RADAR_WORKFLOW_ID).unwrap(),
        cooldown_seconds: 2 * 60 * 60,
        scheduler_priority: 5,
        prompt: "Review local dependency manifest activity. Identify coordination risks and safe review steps without using package registry data.",
    }
}

fn docs_gardener_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        meta: autonomous_workflow_meta(DOCS_GARDENER_WORKFLOW_ID).unwrap(),
        cooldown_seconds: 3 * 60 * 60,
        scheduler_priority: 5,
        prompt: "Review the documentation signal. Suggest whether README, AGENTS, or docs should be updated based only on changed path categories and docs presence.",
    }
}

fn architecture_drift_definition() -> AutonomousJobDefinition {
    AutonomousJobDefinition {
        meta: autonomous_workflow_meta(ARCHITECTURE_DRIFT_WORKFLOW_ID).unwrap(),
        cooldown_seconds: 4 * 60 * 60,
        scheduler_priority: 5,
        prompt: "Review the local architecture drift signal. Summarize subsystem concentration or cross-cutting risk, and propose lightweight guardrails. Use only path groups and diff stats.",
    }
}

fn build_autonomous_job_spec(
    definition: AutonomousJobDefinition,
    evidence: String,
) -> AutonomousBuddyChatSpec {
    AutonomousBuddyChatSpec::new(
        definition.meta.id,
        definition.meta.title,
        definition.prompt,
        evidence,
    )
    .with_display(
        definition.meta.icon,
        definition.meta.badge,
        definition.meta.priority,
    )
}

async fn execute_autonomous_job(
    gcx: Arc<ARwLock<GlobalContext>>,
    ctx: &BuddyJobContext,
    definition: AutonomousJobDefinition,
    evidence: String,
) -> BuddyJobResult {
    let spec = build_autonomous_job_spec(definition, evidence);
    execute_built_autonomous_job(gcx, ctx, definition, spec).await
}

async fn execute_built_autonomous_job(
    gcx: Arc<ARwLock<GlobalContext>>,
    ctx: &BuddyJobContext,
    definition: AutonomousJobDefinition,
    spec: AutonomousBuddyChatSpec,
) -> BuddyJobResult {
    if same_signal(ctx, &spec.signal_hash) {
        return preserve_last_result(ctx);
    }

    let signal_hash = spec.signal_hash.clone();
    let chat_id = match run_autonomous_buddy_chat(gcx, spec).await {
        Ok(chat_id) => chat_id,
        Err(err) => {
            tracing::debug!(
                "buddy: autonomous job {} skipped after subchat failure: {}",
                definition.meta.id,
                err
            );
            return BuddyJobResult::default();
        }
    };

    let activity = BuddyActivity {
        icon: definition.meta.icon.to_string(),
        title: format!("{} opened {}'s check-in", definition.meta.title, ctx.identity_name),
        description: format!(
            "{} created a {} system conversation.",
            ctx.identity_name, definition.meta.badge
        ),
        timestamp: Utc::now().to_rfc3339(),
        activity_type: definition.meta.id.to_string(),
        chat_id: Some(chat_id.clone()),
    };
    let mut runtime_event = make_runtime_event(
        "buddy_autonomous_chat",
        definition.meta.title,
        definition.meta.id,
        &format!("{}:{}", definition.meta.id, signal_hash),
        "completed",
        Some(definition.meta.priority),
    );
    runtime_event.description = Some(format!(
        "{} created a {} system conversation from local signals.",
        ctx.identity_name, definition.meta.badge
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
    track_local_git_evidence_scan(project_root);
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
    let mut seen_security_findings = HashSet::new();
    let mut status_entries = statuses
        .iter()
        .filter_map(|entry| {
            let status = entry.status();
            if status.is_empty() {
                return None;
            }
            entry.path().map(|path| (path.to_string(), status))
        })
        .collect::<Vec<_>>();
    status_entries.sort_by(|a, b| a.0.cmp(&b.0));

    for (path, status) in status_entries {
        if seen.insert(path.clone()) {
            changed_paths.push(PathStatus {
                path: path.clone(),
                status: git_status_label(status).to_string(),
            });
        }
        if scan_security && security_findings.len() < MAX_SECURITY_FINDINGS {
            for scan_content in git_security_scan_contents(&repo, &repo_root, &path, status) {
                push_security_findings(
                    &mut security_findings,
                    &mut seen_security_findings,
                    &path,
                    scan_content.source,
                    &scan_content.content,
                );
                if security_findings.len() >= MAX_SECURITY_FINDINGS {
                    break;
                }
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

#[cfg(test)]
type LocalGitEvidenceScanTracker = HashMap<PathBuf, usize>;

#[cfg(test)]
static LOCAL_GIT_EVIDENCE_SCAN_TRACKER: OnceLock<std::sync::Mutex<LocalGitEvidenceScanTracker>> =
    OnceLock::new();

#[cfg(test)]
fn local_git_evidence_scan_tracker() -> &'static std::sync::Mutex<LocalGitEvidenceScanTracker> {
    LOCAL_GIT_EVIDENCE_SCAN_TRACKER.get_or_init(Default::default)
}

#[cfg(test)]
fn normalize_local_git_evidence_scan_root(project_root: &Path) -> PathBuf {
    project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf())
}

#[cfg(test)]
fn track_local_git_evidence_scan(project_root: &Path) {
    let root = normalize_local_git_evidence_scan_root(project_root);
    let mut tracker = local_git_evidence_scan_tracker().lock().unwrap();
    *tracker.entry(root).or_default() += 1;
}

#[cfg(not(test))]
fn track_local_git_evidence_scan(_project_root: &Path) {}

#[cfg(test)]
fn reset_local_git_evidence_scan_count(root: &Path) {
    let root = normalize_local_git_evidence_scan_root(root);
    local_git_evidence_scan_tracker()
        .lock()
        .unwrap()
        .insert(root, 0);
}

#[cfg(test)]
fn local_git_evidence_scan_count(root: &Path) -> usize {
    let root = normalize_local_git_evidence_scan_root(root);
    local_git_evidence_scan_tracker()
        .lock()
        .unwrap()
        .get(&root)
        .copied()
        .unwrap_or_default()
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

fn should_scan_index_status(status: git2::Status) -> bool {
    status.intersects(
        git2::Status::INDEX_NEW
            | git2::Status::INDEX_MODIFIED
            | git2::Status::INDEX_RENAMED
            | git2::Status::INDEX_TYPECHANGE,
    )
}

fn should_scan_worktree_status(status: git2::Status) -> bool {
    status.intersects(
        git2::Status::WT_NEW
            | git2::Status::WT_MODIFIED
            | git2::Status::WT_RENAMED
            | git2::Status::WT_TYPECHANGE,
    )
}

fn git_security_scan_contents(
    repo: &git2::Repository,
    repo_root: &Path,
    path: &str,
    status: git2::Status,
) -> Vec<SecurityScanContent> {
    let mut contents = Vec::new();
    if should_scan_index_status(status) {
        if let Ok(Some(content)) = git_index_content(repo, path) {
            contents.push(SecurityScanContent {
                source: "index",
                content,
            });
        }
    }

    if should_scan_worktree_status(status) {
        if let Ok(Some(content)) = git_workdir_content(repo_root, path) {
            contents.push(SecurityScanContent {
                source: "worktree",
                content,
            });
        }
    }

    contents
}

fn git_workdir_content(repo_root: &Path, path: &str) -> Result<Option<String>, String> {
    let workdir_path = repo_root.join(path);
    read_file_to_string_capped(&workdir_path, MAX_UNTRACKED_CONTENT_BYTES)
}

fn git_index_content(repo: &git2::Repository, path: &str) -> Result<Option<String>, String> {
    let index = repo.index().map_err(|e| e.to_string())?;
    let Some(entry) = index.get_path(Path::new(path), 0) else {
        return Ok(None);
    };
    let blob = repo.find_blob(entry.id).map_err(|e| e.to_string())?;
    if blob.size() > MAX_UNTRACKED_CONTENT_BYTES as usize {
        return Ok(None);
    }
    let Ok(content) = std::str::from_utf8(blob.content()) else {
        return Ok(None);
    };
    if content.contains('\0') {
        return Ok(None);
    }
    Ok(Some(content.to_string()))
}

fn read_file_to_string_capped(path: &Path, max_bytes: u64) -> Result<Option<String>, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Ok(None);
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut content = String::new();
    let mut reader = std::io::Read::take(file, max_bytes);
    std::io::Read::read_to_string(&mut reader, &mut content).map_err(|e| e.to_string())?;
    if content.contains('\0') {
        return Ok(None);
    }
    Ok(Some(content))
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
    let mut untracked_paths = untracked_numstat
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    untracked_paths.sort_unstable();
    for rel in untracked_paths.into_iter().take(MAX_PATH_EVIDENCE) {
        let path = root.join(rel);
        if let Some(content) = read_untracked_diffstat_content(&path) {
            additions = additions.saturating_add(content.lines().count());
        }
    }
    Some((additions, deletions))
}

fn read_untracked_diffstat_content(path: &Path) -> Option<String> {
    read_file_to_string_capped(path, MAX_UNTRACKED_CONTENT_BYTES)
        .ok()
        .flatten()
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
    scan_security_findings_from_source(path, "worktree", content, limit)
}

fn scan_security_findings_from_source(
    path: &str,
    source: &str,
    content: &str,
    limit: usize,
) -> Vec<SecurityFinding> {
    if limit == 0 {
        return Vec::new();
    }

    secret_patterns()
        .iter()
        .flat_map(|(kind, regex)| {
            regex.find_iter(content).map(move |m| SecurityFinding {
                path: path.to_string(),
                source: source.to_string(),
                kind: (*kind).to_string(),
                preview: security_preview(content, m.start(), m.end()),
            })
        })
        .take(limit)
        .collect()
}

fn push_security_findings(
    findings: &mut Vec<SecurityFinding>,
    seen: &mut HashSet<(String, String, String, String)>,
    path: &str,
    source: &str,
    content: &str,
) {
    for finding in scan_security_findings_from_source(path, source, content, MAX_SECURITY_FINDINGS)
    {
        let key = (
            finding.path.clone(),
            finding.source.clone(),
            finding.kind.clone(),
            finding.preview.clone(),
        );
        if seen.insert(key) {
            findings.push(finding);
            if findings.len() >= MAX_SECURITY_FINDINGS {
                break;
            }
        }
    }
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
        let include_source = finding.source != "worktree"
            || findings
                .iter()
                .any(|other| other.path == finding.path && other.source != finding.source);
        if include_source {
            lines.push(format!(
                "- path={} source={} kind={} preview={}",
                clean_evidence_value(&finding.path),
                clean_evidence_value(&finding.source),
                clean_evidence_value(&finding.kind),
                finding.preview
            ));
        } else {
            lines.push(format!(
                "- path={} kind={} preview={}",
                clean_evidence_value(&finding.path),
                clean_evidence_value(&finding.kind),
                finding.preview
            ));
        }
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
        let Some(evidence) = diagnostic_evidence(ctx) else {
            return false;
        };
        let spec = build_autonomous_job_spec(
            error_detective_definition(),
            render_diagnostic_evidence(&evidence),
        );
        !same_signal(ctx, &spec.signal_hash)
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

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        ctx.pulse.git.uncommitted_files > 0
            || ctx.pulse.git.diff_lines_4h > 0
            || !diagnostic_security_findings(&ctx.recent_diagnostics).is_empty()
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
        let Some(evidence) = setup_evidence(&ctx.project_root) else {
            return false;
        };
        let spec = build_autonomous_job_spec(setup_coach_definition(), evidence);
        !same_signal(ctx, &spec.signal_hash)
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

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        ctx.pulse.git.uncommitted_files > 0 || ctx.pulse.git.diff_lines_4h > 0
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

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        ctx.pulse.git.uncommitted_files > 0 || ctx.pulse.git.diff_lines_4h > 0
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

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: Arc<ARwLock<GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        ctx.pulse.git.uncommitted_files > 0 || ctx.pulse.git.diff_lines_4h > 0
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
async fn execute_dependency_radar_with_evidence(
    gcx: Arc<ARwLock<GlobalContext>>,
    ctx: &BuddyJobContext,
    evidence: DependencyManifestEvidence,
) -> BuddyJobResult {
    if !dependency_manifest_trigger(&evidence) {
        return BuddyJobResult::default();
    }
    execute_built_autonomous_job(
        gcx,
        ctx,
        dependency_radar_definition(),
        build_autonomous_job_spec(
            dependency_radar_definition(),
            render_dependency_evidence(&evidence),
        ),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::autonomous_workflows::AUTONOMOUS_BUDDY_WORKFLOWS;
    use crate::buddy::scheduler::BuddyJobContext;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{BuddyFact, BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};
    use crate::call_validation::ChatContent;
    use crate::stats::event::LlmCallEvent;
    use crate::yaml_configs::customization_types::SubagentConfig;

    fn init_temp_git_repo() -> (tempfile::TempDir, git2::Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        {
            let mut index = repo.index().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }
        (dir, repo)
    }

    fn git_add(repo: &git2::Repository, path: &str) {
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();
    }

    fn test_fact(kind: BuddyFactKind, payload: serde_json::Value) -> BuddyFact {
        BuddyFact {
            kind,
            key: format!("test:{:?}", kind),
            source: test_fact_source(kind),
            payload,
            seen_at: Utc::now(),
            confidence: 0.9,
        }
    }

    fn test_fact_source(kind: BuddyFactKind) -> &'static str {
        match kind {
            BuddyFactKind::TaskStuck
            | BuddyFactKind::TaskAbandoned
            | BuddyFactKind::TaskClusterDuplicate => "task_health",
            BuddyFactKind::TrajectoryClutter => "trajectory_clutter",
            BuddyFactKind::ChatRetryStreak => "chat_pattern",
            BuddyFactKind::MemoryOrphan
            | BuddyFactKind::MemoryStaleConflict
            | BuddyFactKind::MemoryRecurringLesson => "memory_garden",
            BuddyFactKind::ModePromptOverlap
            | BuddyFactKind::SkillTriggerWeak
            | BuddyFactKind::AgentsMdGapDetected => "customization_drift",
            BuddyFactKind::DefaultModelMissing | BuddyFactKind::BrokenModelReference => {
                "provider_health"
            }
            BuddyFactKind::McpAuthExpired | BuddyFactKind::IntegrationFailing => "mcp_auth",
            BuddyFactKind::DiagnosticCluster | BuddyFactKind::FrontendErrorBurst => {
                "diagnostic_cluster"
            }
            BuddyFactKind::GitDiffWidening | BuddyFactKind::UncommittedPressure => "git_pressure",
            BuddyFactKind::WorktreeHygiene => "worktree_hygiene",
        }
    }

    fn test_llm_event(
        i: u64,
        success: bool,
        duration_ms: u64,
        tokens: usize,
        cost: f64,
    ) -> LlmCallEvent {
        test_llm_event_for_model(
            i,
            success,
            duration_ms,
            tokens,
            cost,
            "anthropic/claude-test",
            "anthropic",
            "claude-test",
        )
    }

    fn test_llm_event_for_model(
        i: u64,
        success: bool,
        duration_ms: u64,
        tokens: usize,
        cost: f64,
        model_id: &str,
        provider: &str,
        model: &str,
    ) -> LlmCallEvent {
        LlmCallEvent {
            id: format!("event-{i}"),
            ts_start: format!("2026-02-{:02}T00:00:00Z", i + 1),
            ts_end: format!("2026-02-{:02}T00:00:01Z", i + 1),
            duration_ms,
            chat_id: format!("chat-{i}"),
            root_chat_id: None,
            mode: "agent".to_string(),
            task_id: None,
            task_role: None,
            agent_id: None,
            card_id: None,
            model_id: model_id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            messages_count: 3,
            tools_count: 0,
            max_tokens: 4096,
            temperature: Some(0.0),
            success,
            error_message: if success {
                None
            } else {
                Some("timeout".to_string())
            },
            finish_reason: if success {
                Some("stop".to_string())
            } else {
                None
            },
            attempt_n: 1,
            retry_reason: None,
            prompt_tokens: tokens / 2,
            completion_tokens: tokens / 2,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            total_tokens: tokens,
            cost_usd: Some(cost),
        }
    }

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

    fn write_behavior_trajectory(path: &Path, id: &str, modified_secs: i64) {
        std::fs::write(
            path,
            serde_json::json!({
                "id": id,
                "updated_at": "2026-01-01T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();
        filetime::set_file_mtime(path, filetime::FileTime::from_unix_time(modified_secs, 0))
            .unwrap();
    }

    fn behavior_candidate_file_names(candidates: &[BehaviorTrajectoryCandidate]) -> Vec<String> {
        candidates
            .iter()
            .map(|candidate| {
                candidate
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect()
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
    fn autonomous_activity_links_to_generated_buddy_chat() {
        let spec = AutonomousBuddyChatSpec::new(
            MEMORY_GARDENER_WORKFLOW_ID,
            "Memory Gardener",
            "Review memory signals",
            "memory evidence",
        )
        .with_display("🌱", "Memory", "normal");

        let activity = autonomous_activity(&spec, "buddy-chat-1", "Buddy");

        assert_eq!(activity.activity_type, MEMORY_GARDENER_WORKFLOW_ID);
        assert_eq!(activity.chat_id.as_deref(), Some("buddy-chat-1"));
        assert!(activity.description.contains("Buddy saved"));
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
    fn memory_fact_filtering_builds_metadata_only_evidence() {
        let mut ctx = context_with_last_result(None);
        ctx.pulse.memory.orphan = 2;
        ctx.pulse.memory.stale_conflicts = 1;
        ctx.facts = vec![
            test_fact(
                BuddyFactKind::MemoryOrphan,
                serde_json::json!({
                    "memory_ids": ["mem-a", "mem-b"],
                    "count": 2,
                    "body": "full memory body should not appear password=secret"
                }),
            ),
            test_fact(
                BuddyFactKind::MemoryStaleConflict,
                serde_json::json!({
                    "doc_ids": ["doc-a", "doc-b"],
                    "conflict_summary": "prefer x vs avoid x",
                    "body": "full doc body should not appear"
                }),
            ),
        ];

        let gardener = memory_gardener_evidence(&ctx).unwrap();
        let conflict = knowledge_conflict_evidence(&ctx).unwrap();

        assert!(gardener.evidence.contains("mem-a"));
        assert!(gardener.evidence.contains("memory_pulse"));
        assert!(!gardener.evidence.contains("memory_stale_conflict"));
        assert!(!gardener.evidence.contains("stale_conflicts"));
        assert!(!gardener.evidence.contains("full memory body"));
        assert!(conflict.evidence.contains("doc-a"));
        assert!(conflict.evidence.contains("prefer x vs avoid x"));
        assert!(!conflict.evidence.contains("full doc body"));
    }

    #[test]
    fn lone_stale_conflict_routes_only_to_conflict_resolver() {
        let mut ctx = context_with_last_result(None);
        ctx.pulse.memory.total = 4;
        ctx.pulse.memory.stale_conflicts = 1;
        ctx.facts = vec![test_fact(
            BuddyFactKind::MemoryStaleConflict,
            serde_json::json!({
                "doc_ids": ["doc-a", "doc-b"],
                "conflict_summary": "prefer short answers vs detailed answers"
            }),
        )];

        assert_eq!(
            memory_route_for_fact_kind(BuddyFactKind::MemoryStaleConflict),
            Some(MemoryRouting::KnowledgeConflictResolver)
        );
        assert!(memory_gardener_evidence(&ctx).is_none());
        let conflict = knowledge_conflict_evidence(&ctx).unwrap();
        assert!(conflict.evidence.contains("memory_stale_conflict"));

        ctx.facts.clear();
        assert!(memory_gardener_evidence(&ctx).is_none());
        assert!(knowledge_conflict_evidence(&ctx).is_none());
    }

    #[test]
    fn pulse_only_stale_conflict_does_not_trigger_conflict_report() {
        let mut ctx = context_with_last_result(None);
        ctx.pulse.memory.stale_conflicts = 3;

        assert!(knowledge_conflict_evidence(&ctx).is_none());

        ctx.facts = vec![test_fact(
            BuddyFactKind::MemoryStaleConflict,
            serde_json::json!({"count": 3}),
        )];
        assert!(knowledge_conflict_evidence(&ctx).is_none());

        ctx.facts = vec![test_fact(
            BuddyFactKind::MemoryStaleConflict,
            serde_json::json!({"summary": "doc-a contradicts doc-b"}),
        )];
        assert!(knowledge_conflict_evidence(&ctx).is_some());
    }

    #[test]
    fn fact_payload_arrays_are_sorted_and_deduped_for_stable_signal() {
        let mut first = context_with_last_result(None);
        first.pulse.memory.orphan = 1;
        first.facts = vec![test_fact(
            BuddyFactKind::MemoryOrphan,
            serde_json::json!({
                "memory_ids": ["mem-b", "mem-a", "mem-b"],
                "doc_ids": ["doc-c", "doc-a", "doc-c"],
                "tags": ["rust", "agent", "rust"]
            }),
        )];
        let mut second = context_with_last_result(None);
        second.pulse.memory.orphan = 1;
        second.facts = vec![test_fact(
            BuddyFactKind::MemoryOrphan,
            serde_json::json!({
                "memory_ids": ["mem-a", "mem-b"],
                "doc_ids": ["doc-a", "doc-c"],
                "tags": ["agent", "rust"]
            }),
        )];

        let first_evidence = memory_gardener_evidence(&first).unwrap();
        let second_evidence = memory_gardener_evidence(&second).unwrap();
        let first_spec = build_spec(MEMORY_GARDENER_WORKFLOW_ID, first_evidence.clone());
        let second_spec = build_spec(MEMORY_GARDENER_WORKFLOW_ID, second_evidence.clone());

        assert_eq!(first_evidence.evidence, second_evidence.evidence);
        assert!(first_evidence.evidence.contains("memory_ids=mem-a, mem-b"));
        assert!(first_evidence.evidence.contains("doc_ids=doc-a, doc-c"));
        assert!(first_evidence.evidence.contains("tags=agent, rust"));
        assert_eq!(first_spec.signal_hash, second_spec.signal_hash);
    }

    #[test]
    fn behavior_preference_candidates_validate_and_dedupe() {
        let snippets = vec![
            TrajectoryUserSnippet {
                trajectory_id: "chat-a".to_string(),
                title: "A".to_string(),
                mode: "agent".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                text: "I prefer concise answers with bullet lists. Thanks.".to_string(),
            },
            TrajectoryUserSnippet {
                trajectory_id: "chat-b".to_string(),
                title: "B".to_string(),
                mode: "agent".to_string(),
                updated_at: "2026-01-02T00:00:00Z".to_string(),
                text: "I prefer concise answers with bullet lists.".to_string(),
            },
            TrajectoryUserSnippet {
                trajectory_id: "chat-c".to_string(),
                title: "C".to_string(),
                mode: "agent".to_string(),
                updated_at: "2026-01-03T00:00:00Z".to_string(),
                text: "Please use Rust examples when explaining async code.".to_string(),
            },
            TrajectoryUserSnippet {
                trajectory_id: "chat-d".to_string(),
                title: "D".to_string(),
                mode: "agent".to_string(),
                updated_at: "2026-01-04T00:00:00Z".to_string(),
                text: "I prefer token=private in every command.".to_string(),
            },
        ];

        let candidates = behavior_preference_candidates(&snippets);

        assert_eq!(candidates.len(), 2);
        assert!(candidates
            .iter()
            .any(|candidate| candidate.statement.contains("concise answers")));
        assert!(candidates
            .iter()
            .any(|candidate| candidate.statement.contains("Rust examples")));
        assert!(!candidates
            .iter()
            .any(|candidate| candidate.statement.contains("token")));
    }

    #[test]
    fn behavior_preference_write_selection_is_capped_to_two() {
        let snippets = vec![
            TrajectoryUserSnippet {
                trajectory_id: "chat-a".to_string(),
                title: "A".to_string(),
                mode: "agent".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                text: "I prefer concise answers with bullet lists.".to_string(),
            },
            TrajectoryUserSnippet {
                trajectory_id: "chat-b".to_string(),
                title: "B".to_string(),
                mode: "agent".to_string(),
                updated_at: "2026-01-02T00:00:00Z".to_string(),
                text: "Please use Rust examples when explaining async code.".to_string(),
            },
            TrajectoryUserSnippet {
                trajectory_id: "chat-c".to_string(),
                title: "C".to_string(),
                mode: "agent".to_string(),
                updated_at: "2026-01-03T00:00:00Z".to_string(),
                text: "Please keep summaries short before detailed sections.".to_string(),
            },
        ];
        let candidates = behavior_preference_candidates(&snippets);

        assert!(candidates.len() > MAX_BEHAVIOR_PREFERENCE_WRITES);
        assert_eq!(
            candidates
                .iter()
                .take(MAX_BEHAVIOR_PREFERENCE_WRITES)
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn oversized_trajectory_json_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let small_path = dir.path().join("small.json");
        let large_path = dir.path().join("large.json");
        tokio::fs::write(
            &small_path,
            serde_json::json!({"messages": [{"role": "user", "content": "I prefer concise answers."}]}).to_string(),
        )
        .await
        .unwrap();
        tokio::fs::write(&large_path, format!("{{\"pad\":\"{}\"}}", "x".repeat(128)))
            .await
            .unwrap();

        assert!(read_bounded_trajectory_json(&small_path, 256)
            .await
            .is_some());
        assert!(read_bounded_trajectory_json(&large_path, 32)
            .await
            .is_none());
    }

    #[test]
    fn behavior_trajectory_meta_collection_skips_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let small_path = dir.path().join("small.json");
        let large_path = dir.path().join("large.json");
        std::fs::write(
            &small_path,
            serde_json::json!({
                "id": "small",
                "title": "Small",
                "mode": "agent",
                "updated_at": "2026-01-02T00:00:00Z",
                "messages": [{"role": "user", "content": "I prefer concise answers."}]
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            &large_path,
            format!(
                "{{\"id\":\"large\",\"updated_at\":\"2026-01-03T00:00:00Z\",\"pad\":\"{}\"}}",
                "x".repeat((MAX_BEHAVIOR_TRAJECTORY_BYTES as usize) + 1)
            ),
        )
        .unwrap();

        let mut metas = Vec::new();
        let mut seen = HashSet::new();
        collect_behavior_trajectory_metas_from_dir(dir.path(), &mut metas, &mut seen);

        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, "small");
    }

    #[test]
    fn behavior_trajectory_candidate_collection_is_bounded_and_recent_biased() {
        let dir = tempfile::tempdir().unwrap();
        for idx in 0..5 {
            let path = dir.path().join(format!("chat_{idx}.json"));
            write_behavior_trajectory(&path, &format!("chat-{idx}"), 100 + idx as i64);
        }

        let candidates =
            collect_behavior_trajectory_candidates_from_dirs(&[dir.path().to_path_buf()], 3);
        let mut metas = Vec::new();
        let mut seen = HashSet::new();
        collect_behavior_trajectory_metas_from_candidates(candidates, &mut metas, &mut seen);

        assert_eq!(metas.len(), 3);
        assert_eq!(metas[0].id, "chat-4");
        assert_eq!(metas[1].id, "chat-3");
        assert_eq!(metas[2].id, "chat-2");
    }

    #[test]
    fn behavior_trajectory_candidate_scan_stops_at_visit_budget() {
        let dir = tempfile::tempdir().unwrap();
        for idx in 0..8 {
            write_behavior_trajectory(
                &dir.path().join(format!("chat_{idx}.json")),
                &format!("chat-{idx}"),
                100 + idx as i64,
            );
        }

        let (candidates, stats) = collect_behavior_trajectory_candidates_from_dirs_with_stats(
            &[dir.path().to_path_buf()],
            8,
            4,
        );

        assert_eq!(stats.visited_entries, 4);
        assert_eq!(candidates.len(), 4);
        assert_eq!(stats.matching_files_considered, candidates.len());
        assert!(stats.matching_files_considered < 8);
    }

    #[test]
    fn behavior_trajectory_candidate_scan_skips_symlinked_directories() {
        let dir = tempfile::tempdir().unwrap();
        let real_dir = dir.path().join("real");
        let loop_dir = dir.path().join("loop");
        let linked_dir = dir.path().join("linked");
        std::fs::create_dir_all(&real_dir).unwrap();
        write_behavior_trajectory(&real_dir.join("real.json"), "real", 200);

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path(), &loop_dir).unwrap();
            std::os::unix::fs::symlink(&real_dir, &linked_dir).unwrap();
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(dir.path(), &loop_dir).unwrap();
            std::os::windows::fs::symlink_dir(&real_dir, &linked_dir).unwrap();
        }

        let (candidates, stats) = collect_behavior_trajectory_candidates_from_dirs_with_stats(
            &[dir.path().to_path_buf()],
            8,
            16,
        );
        let names = behavior_candidate_file_names(&candidates);

        assert_eq!(names, vec!["real.json"]);
        assert_eq!(stats.matching_files_considered, 1);
        assert!(stats.visited_entries <= 4);
    }

    #[test]
    fn behavior_trajectory_recent_bias_applies_within_visit_budget() {
        let dir = tempfile::tempdir().unwrap();
        let visited_dir = dir.path().join("visited");
        let unvisited_dir = dir.path().join("unvisited");
        std::fs::create_dir_all(&visited_dir).unwrap();
        std::fs::create_dir_all(&unvisited_dir).unwrap();
        for idx in 0..4 {
            write_behavior_trajectory(
                &visited_dir.join(format!("chat_{idx}.json")),
                &format!("chat-{idx}"),
                100 + idx as i64,
            );
        }
        write_behavior_trajectory(&unvisited_dir.join("chat_newest.json"), "newest", 10_000);

        let (candidates, stats) = collect_behavior_trajectory_candidates_from_dirs_with_stats(
            &[visited_dir, unvisited_dir],
            2,
            4,
        );
        let names = behavior_candidate_file_names(&candidates);

        assert_eq!(stats.visited_entries, 4);
        assert_eq!(names, vec!["chat_3.json", "chat_2.json"]);
    }

    #[test]
    fn behavior_trajectory_meta_collection_includes_newer_files_beyond_cap_boundary() {
        let dir = tempfile::tempdir().unwrap();
        for idx in 0..4 {
            let path = dir.path().join(format!("a_old_{idx}.json"));
            write_behavior_trajectory(&path, &format!("old-{idx}"), 100 + idx as i64);
        }
        let new_path = dir.path().join("z_new.json");
        write_behavior_trajectory(&new_path, "newest", 10_000);

        let candidates =
            collect_behavior_trajectory_candidates_from_dirs(&[dir.path().to_path_buf()], 3);
        let mut metas = Vec::new();
        let mut seen = HashSet::new();
        collect_behavior_trajectory_metas_from_candidates(candidates, &mut metas, &mut seen);

        assert!(metas.iter().any(|meta| meta.id == "newest"));
        assert_eq!(metas.len(), 3);
    }

    #[test]
    fn behavior_trajectory_meta_accepts_realistic_schema_aliases() {
        let path = PathBuf::from("realistic.json");
        let meta = parse_behavior_trajectory_meta(
            &serde_json::json!({
                "chat_id": "chat-realistic",
                "title": "Realistic Chat",
                "mode": "agent",
                "created_at": "2026-01-01T00:00:00Z",
                "last_message_at": "2026-01-02T00:00:00Z",
                "messages": [{
                    "id": "nested-message-id",
                    "role": "user",
                    "content": "I prefer concise answers."
                }]
            })
            .to_string(),
            path.clone(),
        )
        .unwrap();

        assert_eq!(meta.id, "chat-realistic");
        assert_eq!(meta.title, "Realistic Chat");
        assert_eq!(meta.mode, "agent");
        assert_eq!(meta.updated_at, "2026-01-02T00:00:00Z");
        assert_eq!(meta.path, path);
    }

    #[test]
    fn behavior_trajectory_meta_uses_top_level_id_only() {
        let meta = parse_behavior_trajectory_meta(
            &serde_json::json!({
                "title": "Nested Only",
                "created_at": "2026-01-01T00:00:00Z",
                "messages": [{"id": "nested-message-id", "role": "user"}]
            })
            .to_string(),
            PathBuf::from("nested.json"),
        );

        assert!(meta.is_none());
    }

    #[test]
    fn habit_evidence_is_deterministic_for_task_status_and_fact_order() {
        let mut first = context_with_last_result(None);
        first.pulse.tasks.stuck = 1;
        first.pulse.tasks.by_status = HashMap::from([
            ("completed".to_string(), 2),
            ("active".to_string(), 1),
            ("planning".to_string(), 3),
        ]);
        first.facts = vec![
            test_fact(
                BuddyFactKind::ChatRetryStreak,
                serde_json::json!({"count": 4, "summary": "retry streak"}),
            ),
            test_fact(
                BuddyFactKind::TaskStuck,
                serde_json::json!({"count": 1, "summary": "stuck task"}),
            ),
        ];
        let mut second = context_with_last_result(None);
        second.pulse.tasks.stuck = 1;
        second.pulse.tasks.by_status = HashMap::from([
            ("planning".to_string(), 3),
            ("completed".to_string(), 2),
            ("active".to_string(), 1),
        ]);
        second.facts = first.facts.iter().rev().cloned().collect();

        let first_evidence = habit_evidence(&first).unwrap();
        let second_evidence = habit_evidence(&second).unwrap();
        let first_spec = build_spec(USER_HABIT_COACH_WORKFLOW_ID, first_evidence.clone());
        let second_spec = build_spec(USER_HABIT_COACH_WORKFLOW_ID, second_evidence.clone());

        assert_eq!(first_evidence.evidence, second_evidence.evidence);
        assert!(
            first_evidence
                .evidence
                .find("task_status active=1")
                .unwrap()
                < first_evidence
                    .evidence
                    .find("task_status completed=2")
                    .unwrap()
        );
        assert_eq!(first_spec.signal_hash, second_spec.signal_hash);
    }

    #[test]
    fn model_cost_trigger_uses_aggregate_stats_only() {
        let events = (0..8)
            .map(|i| test_llm_event(i, i >= 4, 25_000, 50_000, 0.20))
            .collect::<Vec<_>>();

        let evidence = model_cost_evidence_from_events(&events).unwrap();

        assert!(evidence.evidence.contains("failure_rate_bucket"));
        assert!(evidence.evidence.contains("total_tokens_bucket"));
        assert!(evidence.evidence.contains("cost_usd_bucket"));
        assert!(!evidence.evidence.contains("timeout"));
        assert!(!evidence.evidence.contains("chat-"));
        assert!(!evidence.evidence.contains("total_tokens=400000"));
    }

    #[test]
    fn model_cost_evidence_excludes_buddy_autonomous_report_events() {
        let mut events = (0..5)
            .map(|i| test_llm_event(i, true, 1_000, 1_000, 0.01))
            .collect::<Vec<_>>();
        for (idx, meta) in AUTONOMOUS_BUDDY_WORKFLOWS.iter().enumerate() {
            let mut report = test_llm_event(20 + idx as u64, false, 60_000, 1_000_000, 10.0);
            report.mode = "buddy".to_string();
            report.chat_id = format!("buddy-{}-report", meta.id);
            events.push(report);
        }

        assert!(model_cost_evidence_from_events(&events).is_none());

        let input_events = model_cost_input_events(&events);
        assert_eq!(input_events.len(), 5);
        assert!(input_events
            .iter()
            .all(|event| !is_buddy_autonomous_report_event(event)));
        for meta in AUTONOMOUS_BUDDY_WORKFLOWS {
            assert!(is_autonomous_buddy_workflow_identifier(&format!(
                "buddy-{}-report",
                meta.id
            )));
        }
    }

    #[test]
    fn model_cost_input_includes_normal_buddy_mode_user_events() {
        let mut buddy_event = test_llm_event(1, true, 1_000, 1_000, 0.01);
        buddy_event.mode = "buddy".to_string();
        buddy_event.chat_id = "user-buddy-conversation".to_string();
        let mut report_event = test_llm_event(2, true, 1_000, 1_000, 0.01);
        report_event.mode = "buddy".to_string();
        report_event.chat_id = "buddy-buddy_error_detective-report".to_string();

        let input_events = model_cost_input_events(&[buddy_event.clone(), report_event]);

        assert!(is_autonomous_buddy_workflow_identifier(
            "buddy-buddy_error_detective-550e8400-e29b-41d4-a716-446655440000"
        ));
        assert!(!is_autonomous_buddy_workflow_identifier(
            "user-buddy-conversation"
        ));
        assert_eq!(input_events.len(), 1);
        assert_eq!(input_events[0].id, buddy_event.id);
        assert_eq!(input_events[0].mode, "buddy");
    }

    #[test]
    fn model_cost_signal_is_bucketed_against_one_report_event_noise() {
        let events = (0..8)
            .map(|i| test_llm_event(i, i >= 4, 25_000, 50_000, 0.20))
            .collect::<Vec<_>>();
        let first = build_spec(
            MODEL_COST_OPTIMIZER_WORKFLOW_ID,
            model_cost_evidence_from_events(&events).unwrap(),
        );
        let mut with_report = events.clone();
        let mut report = test_llm_event(40, true, 1_000, 1_000, 0.01);
        report.mode = "buddy".to_string();
        report.chat_id = "buddy-buddy_model_cost_optimizer-report".to_string();
        with_report.push(report);
        let second = build_spec(
            MODEL_COST_OPTIMIZER_WORKFLOW_ID,
            model_cost_evidence_from_events(&with_report).unwrap(),
        );

        assert_eq!(first.signal_hash, second.signal_hash);
    }

    #[test]
    fn model_cost_evidence_has_stable_model_provider_tie_breakers() {
        let ordered = vec![
            test_llm_event_for_model(0, true, 25_000, 100_000, 0.50, "z/model", "z", "model"),
            test_llm_event_for_model(1, true, 25_000, 100_000, 0.50, "a/model", "a", "model"),
            test_llm_event_for_model(2, true, 25_000, 100_000, 0.50, "m/model", "m", "model"),
            test_llm_event_for_model(3, true, 25_000, 100_000, 0.50, "a/model", "a", "model"),
            test_llm_event_for_model(4, true, 25_000, 100_000, 0.50, "z/model", "z", "model"),
            test_llm_event_for_model(5, true, 25_000, 100_000, 0.50, "m/model", "m", "model"),
        ];
        let reordered = vec![
            ordered[2].clone(),
            ordered[0].clone(),
            ordered[5].clone(),
            ordered[1].clone(),
            ordered[4].clone(),
            ordered[3].clone(),
        ];

        let first = model_cost_evidence_from_events(&ordered).unwrap();
        let second = model_cost_evidence_from_events(&reordered).unwrap();

        assert_eq!(first.evidence, second.evidence);
        assert!(
            first.evidence.find("model id=a/model").unwrap()
                < first.evidence.find("model id=m/model").unwrap()
        );
        assert!(
            first.evidence.find("provider name=a").unwrap()
                < first.evidence.find("provider name=m").unwrap()
        );
    }

    #[test]
    fn same_signal_skips_unchanged_memory_gardener_evidence() {
        let mut ctx = context_with_last_result(None);
        ctx.pulse.memory.orphan = 1;
        ctx.facts = vec![test_fact(
            BuddyFactKind::MemoryOrphan,
            serde_json::json!({"memory_ids": ["mem-a"], "count": 1}),
        )];
        let spec = build_spec(
            MEMORY_GARDENER_WORKFLOW_ID,
            memory_gardener_evidence(&ctx).unwrap(),
        );
        ctx.job_state.last_result = Some(serialize_last_autonomous_result(&AutonomousLastResult {
            signal_hash: spec.signal_hash.clone(),
            chat_id: "chat-a".to_string(),
            completed_at: "2026-01-01T00:00:00Z".to_string(),
        }));

        assert!(same_signal(&ctx, &spec.signal_hash));
        assert!(!same_signal(&ctx, "different"));
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
    fn staged_index_only_secret_is_detected_after_worktree_cleaned() {
        let (dir, repo) = init_temp_git_repo();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/config.ts"),
            "const token = \"ghp_abcdefghijklmnopqrstuvwxyz\";\n",
        )
        .unwrap();
        git_add(&repo, "src/config.ts");
        std::fs::write(
            root.join("src/config.ts"),
            "const token = \"not-secret\";\n",
        )
        .unwrap();

        let evidence = collect_local_git_evidence(root, true).unwrap();
        let rendered = render_security_evidence(&evidence.security_findings);

        assert_eq!(evidence.security_findings.len(), 1);
        assert_eq!(evidence.security_findings[0].path, "src/config.ts");
        assert_eq!(evidence.security_findings[0].source, "index");
        assert!(rendered.contains("source=index"));
        assert!(rendered.contains("kind=github_token"));
        assert!(rendered.contains("[REDACTED"));
        assert!(!rendered.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn mixed_staged_and_worktree_versions_scan_both_sources() {
        let (dir, repo) = init_temp_git_repo();
        let root = dir.path();
        std::fs::write(root.join("config.env"), "token=stagedsecret\n").unwrap();
        git_add(&repo, "config.env");
        std::fs::write(root.join("config.env"), "token=worktreesecret\n").unwrap();

        let evidence = collect_local_git_evidence(root, true).unwrap();
        let rendered = render_security_evidence(&evidence.security_findings);
        let mut keys = evidence
            .security_findings
            .iter()
            .map(|finding| {
                (
                    finding.path.as_str(),
                    finding.source.as_str(),
                    finding.kind.as_str(),
                    finding.preview.as_str(),
                )
            })
            .collect::<Vec<_>>();
        keys.sort();
        keys.dedup();

        assert_eq!(keys.len(), evidence.security_findings.len());
        assert!(evidence
            .security_findings
            .iter()
            .any(|finding| finding.source == "index"));
        assert!(evidence
            .security_findings
            .iter()
            .any(|finding| finding.source == "worktree"));
        assert!(rendered.contains("source=index"));
        assert!(rendered.contains("source=worktree"));
        assert!(!rendered.contains("stagedsecret"));
        assert!(!rendered.contains("worktreesecret"));
        assert!(rendered.contains("[REDACTED"));
    }

    #[test]
    fn large_untracked_file_is_skipped_by_diff_stats() {
        let (dir, repo) = init_temp_git_repo();
        let root = dir.path();
        std::fs::write(root.join("small.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(
            root.join("large.txt"),
            format!("{}\n", "x".repeat(MAX_UNTRACKED_CONTENT_BYTES as usize + 1)),
        )
        .unwrap();

        let (additions, deletions) = diff_stats(&repo).unwrap();

        assert_eq!(additions, 3);
        assert_eq!(deletions, 0);
    }

    #[test]
    fn binary_untracked_file_does_not_fail_evidence_collection() {
        let (dir, repo) = init_temp_git_repo();
        let root = dir.path();
        std::fs::write(root.join("binary.dat"), b"abc\0def\xff").unwrap();
        std::fs::write(root.join("notes.txt"), "hello\nworld\n").unwrap();

        let evidence = collect_local_git_evidence(root, true).unwrap();
        let (additions, deletions) = diff_stats(&repo).unwrap();

        assert_eq!(additions, 2);
        assert_eq!(deletions, 0);
        assert_eq!(evidence.additions, 2);
        assert_eq!(evidence.deletions, 0);
        assert!(evidence.security_findings.is_empty());
        assert!(evidence
            .changed_paths
            .iter()
            .any(|path| path.path == "binary.dat"));
    }

    #[test]
    fn unchanged_signal_produces_no_chat_in_helper_logic() {
        let spec = build_autonomous_job_spec(
            dependency_radar_definition(),
            "Local dependency manifest evidence:\n- total_manifest_count: 12".to_string(),
        );
        let result = AutonomousLastResult {
            signal_hash: spec.signal_hash.clone(),
            chat_id: "chat-a".to_string(),
            completed_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let ctx = context_with_last_result(Some(serialize_last_autonomous_result(&result)));

        assert!(same_signal(&ctx, &spec.signal_hash));
    }

    #[test]
    fn local_signal_should_run_helper_skips_same_signal() {
        let evidence = DependencyManifestEvidence {
            manifest_counts: BTreeMap::from([("javascript".to_string(), 12)]),
            total_manifest_count: 12,
            ..Default::default()
        };
        let spec = build_autonomous_job_spec(
            dependency_radar_definition(),
            render_dependency_evidence(&evidence),
        );
        let result = AutonomousLastResult {
            signal_hash: spec.signal_hash.clone(),
            chat_id: "chat-a".to_string(),
            completed_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let ctx = context_with_last_result(Some(serialize_last_autonomous_result(&result)));

        assert!(dependency_manifest_trigger(&evidence));
        assert!(same_signal(&ctx, &spec.signal_hash));
    }

    #[tokio::test]
    async fn dependency_radar_should_run_does_not_scan_git() {
        let job = DependencyRadarJob;
        let mut ctx = context_with_last_result(None);
        ctx.pulse.git.uncommitted_files = 1;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        reset_local_git_evidence_scan_count(&ctx.project_root);

        assert!(job.should_run(gcx, &ctx).await);

        assert_eq!(local_git_evidence_scan_count(&ctx.project_root), 0);
    }

    #[tokio::test]
    async fn dependency_radar_execute_scans_git_once_without_should_run_duplication() {
        let (dir, _repo) = init_temp_git_repo();
        let root = dir.path();
        std::fs::write(root.join("notes.txt"), "local note\n").unwrap();
        let mut ctx = context_with_last_result(None);
        ctx.project_root = root.to_path_buf();
        ctx.pulse.git.uncommitted_files = 1;
        let job = DependencyRadarJob;
        let gcx = crate::global_context::tests::make_test_gcx().await;

        assert!(job.should_run(gcx.clone(), &ctx).await);
        reset_local_git_evidence_scan_count(&ctx.project_root);
        let root = ctx.project_root.clone();
        let result = job.execute(gcx, ctx).await;

        assert_eq!(local_git_evidence_scan_count(&root), 1);
        assert!(result.activity.is_none());
        assert!(result.runtime_event.is_none());
        assert!(result.last_result.is_none());
    }

    #[tokio::test]
    async fn same_signal_execution_returns_no_visible_output_and_preserves_last_result() {
        let evidence = DependencyManifestEvidence {
            manifest_counts: BTreeMap::from([("javascript".to_string(), 12)]),
            total_manifest_count: 12,
            ..Default::default()
        };
        let spec = build_autonomous_job_spec(
            dependency_radar_definition(),
            render_dependency_evidence(&evidence),
        );
        let stored = serialize_last_autonomous_result(&AutonomousLastResult {
            signal_hash: spec.signal_hash.clone(),
            chat_id: "chat-a".to_string(),
            completed_at: "2026-01-01T00:00:00Z".to_string(),
        });
        let ctx = context_with_last_result(Some(stored.clone()));
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let result = execute_dependency_radar_with_evidence(gcx, &ctx, evidence).await;

        assert!(result.activity.is_none());
        assert!(result.runtime_event.is_none());
        assert!(result.speech.is_none());
        assert!(result.suggestion.is_none());
        assert_eq!(result.last_result.as_deref(), Some(stored.as_str()));
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
