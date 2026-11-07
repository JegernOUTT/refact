use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::app_state::AppState;
pub use refact_buddy_core::runtime_writer::{run_runtime_queue_writer, RuntimeQueueWriteOp};
use refact_buddy_core::user_action::UserAction;
use super::drafts::{validate_draft_payload, DraftStore, DraftTarget, DraftValidationError};
use super::events::BuddyEvent;
use super::facts::FactStore;
use super::humor::{HumorPlan, HumorService};
use super::memory_lifecycle::{
    detect_memory_lifecycle_ops_from_knowledge_dirs, memory_lifecycle_op_counts, MemoryOpsState,
};
use super::observers::{build_observer_registry, BuddyObserver, ObserverContext};
use super::opportunities::{primary_fact_kind_for_opportunity, OpportunityDetector, OpportunityQueue};
use super::policy::{evaluate_with_mutes, PolicyDecision};
use super::runtime_queue::RuntimeQueue;
use super::settings::BuddySettings;
use super::snapshot::BuddySnapshot;
use super::storage::RuntimeQueueRecord;
use super::types::{
    BuddyActivity, BuddyChatPhraseBank, BuddyDraft, BuddyFact, BuddyFactKind, BuddyOpportunity,
    BuddyPersonalityProfile, BuddyPulse, BuddyQuest, BuddyRuntimeEvent, BuddySpeechItem,
    BuddyState, BuddySuggestion, OpportunityStatus,
};
use super::voice_service::{SpeechIntent, SpeechIntentWireToken, VoiceCtx, VoiceIntent, voice_service};

const SUGGESTION_RATE_LIMIT_SECS: u64 = 300;
const SUGGESTION_EXPIRY_SECS: i64 = 300;
const QUEST_SUGGESTION_EXPIRY_SECS: i64 = 24 * 3600;
const PET_DECAY_INTERVAL_SECS: u64 = 15;
const OBSERVER_CONCURRENCY: usize = 4;
const MEMORY_OPS_ARCHIVE_THRESHOLD_BYTES: u64 = 32 * 1024 * 1024;
const MEMORY_OPS_COMPACT_INTERVAL_SECS: u64 = 6 * 60 * 60;
const MEMORY_OPS_DRAIN_INTERVAL_SECS: u64 = 600;
const MEMORY_OPS_DRAIN_MAX_APPLIES: usize = 200;
const BUDDY_SPEECH_MAX_CHARS: usize = 280;
const BUDDY_TITLE_MAX_CHARS: usize = 120;
const BUDDY_DESCRIPTION_MAX_CHARS: usize = 500;
const BUDDY_STATIC_SPEECH_FALLBACK: &str = "Tiny gremlin update: something needs attention.";
const BUDDY_STATIC_TITLE_FALLBACK: &str = "Buddy update";
const BUDDY_STATIC_DESCRIPTION_FALLBACK: &str = "Buddy has an update ready.";
const OPERATOR_REOPEN_DELAYS: [Duration; 5] = [
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(2 * 60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
];
const OPERATOR_MANUAL_REOPEN_COOLDOWN: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperatorOpenFailureKind {
    AlreadyOwned,
    IoError,
    ProjectMismatch,
    NotAttempted,
}

impl OperatorOpenFailureKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyOwned => "already_owned",
            Self::IoError => "io_error",
            Self::ProjectMismatch => "project_mismatch",
            Self::NotAttempted => "not_attempted",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OperatorUnavailableStatus {
    pub(crate) kind: OperatorOpenFailureKind,
    pub(crate) detail: String,
    pub(crate) retry_in_seconds: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct OperatorOpenFailure {
    pub(crate) reason: OperatorOpenFailureReason,
    retry_stage: usize,
    next_retry_at: tokio::time::Instant,
    attempts: u64,
}

#[derive(Clone, Debug)]
pub(crate) enum OperatorOpenFailureReason {
    AlreadyOwned(Option<super::operator_store::OperatorLockOwner>),
    Other {
        kind: OperatorOpenFailureKind,
        detail: String,
    },
}

impl std::fmt::Display for OperatorOpenFailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyOwned(Some(owner)) => write!(
                f,
                "operator store is already owned by another writer ({})",
                owner.describe()
            ),
            Self::AlreadyOwned(None) => {
                f.write_str("operator store is already owned by another writer")
            }
            Self::Other {
                detail, ..
            } => f.write_str(detail),
        }
    }
}

impl OperatorOpenFailureReason {
    fn kind(&self) -> OperatorOpenFailureKind {
        match self {
            Self::AlreadyOwned(_) => OperatorOpenFailureKind::AlreadyOwned,
            Self::Other {
                kind, ..
            } => *kind,
        }
    }
}

impl From<super::operator_runtime::OperatorRuntimeOpenError> for OperatorOpenFailureReason {
    fn from(error: super::operator_runtime::OperatorRuntimeOpenError) -> Self {
        match error {
            super::operator_runtime::OperatorRuntimeOpenError::Store(
                super::operator_store::OperatorStoreError::AlreadyOwned(owner),
            ) => Self::AlreadyOwned(owner),
            super::operator_runtime::OperatorRuntimeOpenError::ProjectIdentity(detail) => {
                Self::Other {
                    kind: OperatorOpenFailureKind::ProjectMismatch,
                    detail,
                }
            }
            super::operator_runtime::OperatorRuntimeOpenError::Store(
                super::operator_store::OperatorStoreError::ProjectMismatch,
            ) => Self::Other {
                kind: OperatorOpenFailureKind::ProjectMismatch,
                detail: "operator store belongs to a different project".to_string(),
            },
            error => Self::Other {
                kind: OperatorOpenFailureKind::IoError,
                detail: error.to_string(),
            },
        }
    }
}

impl OperatorOpenFailure {
    fn new(reason: OperatorOpenFailureReason, now: tokio::time::Instant) -> Self {
        Self {
            reason,
            retry_stage: 0,
            next_retry_at: now + OPERATOR_REOPEN_DELAYS[0],
            attempts: 1,
        }
    }

    fn is_due(&self, now: tokio::time::Instant) -> bool {
        now >= self.next_retry_at
    }

    fn record_failure(
        &mut self,
        reason: OperatorOpenFailureReason,
        now: tokio::time::Instant,
    ) -> bool {
        self.reason = reason;
        self.attempts = self.attempts.saturating_add(1);
        let previous_stage = self.retry_stage;
        self.retry_stage = (self.retry_stage + 1).min(OPERATOR_REOPEN_DELAYS.len() - 1);
        self.next_retry_at = now + OPERATOR_REOPEN_DELAYS[self.retry_stage];
        self.retry_stage > previous_stage
    }

    fn replace_reason_without_rescheduling(&mut self, reason: OperatorOpenFailureReason) {
        self.reason = reason;
    }
}

pub(crate) fn chat_phrase_bank_is_fresh(bank: &BuddyChatPhraseBank, now: DateTime<Utc>) -> bool {
    bank.day == now.date_naive().to_string()
}

pub(crate) async fn observe_buddy_facts_parallel(
    due_observers: Vec<Arc<dyn BuddyObserver>>,
    gcx: AppState,
    project_root: std::path::PathBuf,
    now: DateTime<Utc>,
) -> Vec<(u64, Vec<BuddyFact>)> {
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut pending = FuturesUnordered::new();
    let mut all_facts = Vec::new();
    for obs in due_observers {
        let gcx = gcx.clone();
        let project_root = project_root.clone();
        pending.push(async move {
            let ctx = ObserverContext {
                project_root,
                now,
            };
            let refresh_ttl = obs.emission_refresh_ttl_seconds();
            let facts =
                tokio::time::timeout(tokio::time::Duration::from_secs(5), obs.observe(gcx, &ctx))
                    .await
                    .unwrap_or_default();
            (refresh_ttl, facts)
        });
        if pending.len() >= OBSERVER_CONCURRENCY {
            if let Some(group) = pending.next().await {
                all_facts.push(group);
            }
        }
    }
    while let Some(group) = pending.next().await {
        all_facts.push(group);
    }
    all_facts
}

pub(crate) fn validate_workflow_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Redact common credential shapes. Uses regex with case-insensitive matching
/// so **all** occurrences are scrubbed regardless of capitalization. Mirrors
/// the secret patterns used by the GUI's `reportBuddyFrontendError.ts` so the
/// backend can't leak something the frontend would have masked.
pub(crate) fn redact_sensitive(text: &str) -> String {
    refact_core::string_utils::redact_sensitive(text)
}

fn parse_commit_activity_from_log(output: &str) -> Vec<UserAction> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            let sha = parts.next()?.trim();
            let _subject_slug = parts.next()?;
            let message = parts.next()?.trim();
            if sha.is_empty() {
                return None;
            }
            Some(UserAction::CommitMade {
                sha: sha.to_string(),
                message_first_line: message.chars().take(80).collect(),
                files: 0,
                ts: Utc::now(),
            })
        })
        .collect()
}

async fn poll_commit_activity_once(gcx: AppState, project_root: PathBuf) {
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("git")
            .arg("log")
            .arg("-10")
            .arg("--since=1h ago")
            .arg("--pretty=format:%H|%f|%s")
            .current_dir(project_root)
            .output()
    })
    .await;
    let Ok(Ok(output)) = output else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let actions = parse_commit_activity_from_log(&text);
    if actions.is_empty() {
        return;
    }
    let user_activity = gcx.buddy.user_activity.clone();
    if let Ok(mut ring) = user_activity.try_lock() {
        let mut existing = ring
            .snapshot()
            .iter()
            .filter_map(|action| match action {
                UserAction::CommitMade {
                    sha, ..
                } => Some(sha.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        for action in actions {
            if let UserAction::CommitMade {
                sha, ..
            } = &action
            {
                if existing.contains(sha) {
                    continue;
                }
                existing.insert(sha.clone());
            }
            ring.push(action);
        }
    };
}

async fn commit_activity_poller(
    gcx: AppState,
    project_root: PathBuf,
    shutdown_flag: Arc<std::sync::atomic::AtomicBool>,
) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(300)) => {
                if shutdown_flag.load(Ordering::SeqCst) {
                    break;
                }
                poll_commit_activity_once(gcx.clone(), project_root.clone()).await;
            }
            _ = wait_for_shutdown(shutdown_flag.clone()) => {
                break;
            }
        }
    }
}

async fn wait_for_shutdown(shutdown_flag: Arc<std::sync::atomic::AtomicBool>) {
    while !shutdown_flag.load(Ordering::SeqCst) {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

/// Poll the single owned maintenance future alongside native actor ticks and stop.
/// Cancellation drops the future (not a detached task); repository-owned permits
/// remain held until an already-started database operation completes.
async fn drive_maintenance(
    buddy: &Arc<tokio::sync::Mutex<Option<BuddyService>>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    maintenance: impl std::future::Future<Output = ()>,
) {
    tokio::pin!(maintenance);
    let mut native_tick =
        tokio::time::interval(std::time::Duration::from_secs(PET_DECAY_INTERVAL_SECS));
    loop {
        tokio::select! {
            _ = wait_for_shutdown(shutdown.clone()) => break,
            _ = &mut maintenance => break,
            _ = native_tick.tick() => {
                let mut guard = buddy.lock().await;
                if let Some(svc) = guard.as_mut() { svc.apply_pet_tick(PET_DECAY_INTERVAL_SECS); }
            }
        }
    }
}

pub(crate) fn redact_diagnostic_metadata(value: &str) -> Option<String> {
    let redacted = redact_sensitive(value).trim().to_string();
    if redacted.is_empty() {
        None
    } else {
        Some(redacted)
    }
}

pub struct BuddyService {
    admission_owner: Arc<()>,
    pub state: BuddyState,
    pub settings: BuddySettings,
    pub outcome_runtime: Option<super::outcome_runtime::OutcomeRuntime>,
    pub(crate) operator_open_failure: Option<OperatorOpenFailure>,
    operator_manual_reopen_at: Option<tokio::time::Instant>,
    operator_reopen_task: Option<
        JoinHandle<
            Result<
                super::outcome_runtime::OutcomeRuntime,
                super::operator_runtime::OperatorRuntimeOpenError,
            >,
        >,
    >,
    pub operator_features: super::operator_runtime::OperatorFeatures,
    /// Read-only compatibility cache; never saved into legacy state.
    pub operator_opportunity_projection: Vec<BuddyOpportunity>,
    operator_legacy_seeded: bool,
    /// How many historical cards already committed. Bounds each migration pass
    /// without dropping a backlog larger than one batch.
    operator_legacy_seed_cursor: usize,
    operator_legacy_history_seeded: bool,
    operator_receipt_seed: super::legacy_work::HistorySeed<super::receipts::BuddyReceipt>,
    operator_activity_seed: super::legacy_work::HistorySeed<BuddyActivity>,
    persisted_operator_drafts: HashSet<String>,
    /// Serializes bounded durable admission, never native actor decisions.
    admission_gate: Arc<tokio::sync::Mutex<()>>,
    pub events_tx: broadcast::Sender<BuddyEvent>,
    pub project_root: std::path::PathBuf,
    pub last_suggestion_at: Option<Instant>,
    pub recent_diagnostics: Vec<super::diagnostics::DiagnosticContext>,
    pub memory_ops: MemoryOpsState,
    pub last_issue_at: Option<Instant>,
    pub recent_issue_errors: Vec<(String, DateTime<Utc>)>,
    pub runtime_queue: RuntimeQueue,
    pub dismissed_runtime_keys: HashMap<String, DateTime<Utc>>,
    pub dirty: bool,
    pub active_speech: Option<BuddySpeechItem>,
    pub queue_writer: Option<mpsc::UnboundedSender<RuntimeQueueWriteOp>>,
    pub fact_store: FactStore,
    pub opportunity_queue: OpportunityQueue,
    pub opportunity_accept_claims: HashSet<String>,
    pub humor_service: Arc<tokio::sync::Mutex<HumorService>>,
    pub pulse: BuddyPulse,
    pub draft_store: DraftStore,
    pub drafts_dirty: bool,
    pub last_observer_tick: HashMap<&'static str, DateTime<Utc>>,
    pub observers: Vec<Arc<dyn BuddyObserver>>,
    pub provider_rate_limit_window: super::observers::provider_health::ProviderRateLimitWindow,
    pub background_tasks: Vec<JoinHandle<()>>,
    pub chat_reaction_limiter: crate::buddy::chat_reactions::ChatReactionLimiter,
    pub chat_reaction_debug: crate::buddy::chat_reactions::ChatReactionDebugState,
    pub auto_quiet_window: Option<(u8, u8)>,
    #[cfg(test)]
    pub force_next_runtime_enqueue_drop: bool,
}

/// A captured service admission. Dropping it releases backpressure without an ack.
/// No uncommitted draft or memory replacement is installed in the actor.
pub struct BuddyAdmission<T> {
    value: T,
    pending: Option<AdmissionWrite>,
}

struct AdmissionWrite {
    runtime: super::outcome_runtime::OutcomeRuntime,
    owner: Arc<()>,
    permit: Option<tokio::sync::OwnedMutexGuard<()>>,
    opportunities: Vec<BuddyOpportunity>,
    drafts: Vec<BuddyDraft>,
    memory: Option<(MemoryOpsState, MemoryOpsState)>,
}

impl<T> BuddyAdmission<T> {
    fn ready(value: T) -> Self {
        Self {
            value,
            pending: None,
        }
    }

    pub async fn commit(
        self,
        buddy: &Arc<tokio::sync::Mutex<Option<BuddyService>>>,
    ) -> Result<T, String> {
        let Some(mut pending) = self.pending else {
            return Ok(self.value);
        };
        let opportunities = pending.opportunities.clone();
        let drafts = pending.drafts.clone();
        let worker = pending.runtime.worker();
        // The repository closure owns capacity even if the caller is cancelled:
        // spawn_blocking work cannot be cancelled once it has started.
        let permit = pending.permit.take();
        let (_permit, result) = pending
            .runtime
            .with_store(move |store| {
                let result =
                    super::legacy_work::migrate_batch(store, &worker, &opportunities, &drafts)
                        .and_then(|_| super::legacy_work::projection_page(store, None, 200));
                Ok((permit, result))
            })
            .await
            .map_err(|error| error.to_string())?;
        let (projection, _) = result.map_err(|error| error.to_string())?;
        let mut guard = buddy.lock().await;
        let svc = guard
            .as_mut()
            .ok_or("Buddy operator unavailable after commit")?;
        require_publication_owner(svc, &pending.runtime)?;
        if !Arc::ptr_eq(&svc.admission_owner, &pending.owner) {
            return Err("Buddy actor replaced during admission".into());
        }
        if pending.opportunities.iter().any(|opportunity| {
            svc.state
                .muted_rules
                .contains(&refact_buddy_core::verdicts::rule_key_of(
                    &opportunity.cooldown_key,
                ))
        }) {
            return Err("Buddy admission preference changed; committed work retained".into());
        }
        if let Some((previous, replacement)) = pending.memory {
            if svc.memory_ops != previous {
                return Err("Buddy memory input changed during admission".into());
            }
            svc.memory_ops = replacement;
        }
        svc.operator_opportunity_projection = projection;
        for draft in pending.drafts {
            svc.persisted_operator_drafts.insert(draft.id.clone());
            svc.draft_store.restore(vec![draft.clone()]);
            let _ = svc.events_tx.send(BuddyEvent::DraftCreated {
                draft,
            });
        }
        Ok(self.value)
    }
}

fn normalize_generated_buddy_text(raw: &str) -> String {
    raw.replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string()
}

fn contains_redaction_marker(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("[redacted") || lower.contains("<redacted") || lower.contains("&lt;redacted")
}

fn cap_generated_buddy_text(text: &str, max_chars: usize) -> String {
    crate::llm::safe_truncate(text, max_chars)
        .trim()
        .to_string()
}

fn safe_buddy_candidate(text: &str, max_chars: usize) -> Option<String> {
    let normalized = normalize_generated_buddy_text(text);
    let redacted = redact_sensitive(&normalized);
    if normalized.is_empty() || redacted != normalized || contains_redaction_marker(&normalized) {
        None
    } else {
        Some(cap_generated_buddy_text(&normalized, max_chars))
    }
}

fn safe_generated_buddy_text(
    generated: &str,
    fallback: String,
    static_fallback: &'static str,
    max_chars: usize,
) -> String {
    safe_buddy_candidate(generated, max_chars)
        .or_else(|| safe_buddy_candidate(&fallback, max_chars))
        .unwrap_or_else(|| cap_generated_buddy_text(static_fallback, max_chars))
}

fn safe_generated_buddy_text_opt(
    generated: Option<String>,
    fallback: Option<String>,
    static_fallback: &'static str,
    max_chars: usize,
) -> Option<String> {
    generated
        .as_deref()
        .and_then(|text| safe_buddy_candidate(text, max_chars))
        .or_else(|| {
            fallback
                .as_deref()
                .and_then(|text| safe_buddy_candidate(text, max_chars))
        })
        .or_else(|| Some(cap_generated_buddy_text(static_fallback, max_chars)))
}

pub async fn render_buddy_speech(
    gcx: AppState,
    persona: BuddyPersonalityProfile,
    identity_name: String,
    pulse: BuddyPulse,
    workflow_id: Option<String>,
    workflow_summary: String,
    intent: SpeechIntent,
    fallback_text: String,
) -> BuddySpeechItem {
    let pulse_one_liner = format!(
        "{} pending ops, {} recent stuck task alerts",
        pulse.memory.pending_ops,
        pulse.tasks.recent_stuck_alert_count_1h()
    );
    let voice_ctx = VoiceCtx {
        persona: &persona,
        identity_name: identity_name.as_str(),
        pulse_one_liner,
        workflow_id: workflow_id.as_deref(),
        workflow_summary: Some(workflow_summary.as_str()),
    };
    let mut speech = voice_service()
        .await
        .render_speech(gcx, voice_ctx, intent)
        .await;
    speech.text = safe_generated_buddy_text(
        &speech.text,
        fallback_text,
        BUDDY_STATIC_SPEECH_FALLBACK,
        BUDDY_SPEECH_MAX_CHARS,
    );
    speech
}

pub async fn render_buddy_runtime_event(
    gcx: AppState,
    persona: BuddyPersonalityProfile,
    identity_name: String,
    pulse: BuddyPulse,
    workflow_id: Option<String>,
    workflow_summary: String,
    status: &str,
    fallback_title: String,
    fallback_description: Option<String>,
) -> (String, Option<String>) {
    let pulse_one_liner = format!(
        "{} pending ops, {} recent stuck task alerts",
        pulse.memory.pending_ops,
        pulse.tasks.recent_stuck_alert_count_1h()
    );
    let voice_ctx = VoiceCtx {
        persona: &persona,
        identity_name: identity_name.as_str(),
        pulse_one_liner,
        workflow_id: workflow_id.as_deref(),
        workflow_summary: Some(workflow_summary.as_str()),
    };
    let (title, description) = voice_service()
        .await
        .render_runtime_event(gcx, voice_ctx, status)
        .await;
    let title_is_safe = safe_buddy_candidate(&title, BUDDY_TITLE_MAX_CHARS).is_some();
    (
        safe_generated_buddy_text(
            &title,
            fallback_title,
            BUDDY_STATIC_TITLE_FALLBACK,
            BUDDY_TITLE_MAX_CHARS,
        ),
        safe_generated_buddy_text_opt(
            if title_is_safe { description } else { None },
            fallback_description,
            BUDDY_STATIC_DESCRIPTION_FALLBACK,
            BUDDY_DESCRIPTION_MAX_CHARS,
        ),
    )
}

pub async fn render_buddy_activity_title(
    gcx: AppState,
    persona: BuddyPersonalityProfile,
    identity_name: String,
    pulse: BuddyPulse,
    workflow_id: Option<String>,
    workflow_summary: String,
    intent: VoiceIntent,
    fallback_title: String,
) -> String {
    let pulse_one_liner = format!(
        "{} pending ops, {} recent stuck task alerts",
        pulse.memory.pending_ops,
        pulse.tasks.recent_stuck_alert_count_1h()
    );
    let voice_ctx = VoiceCtx {
        persona: &persona,
        identity_name: identity_name.as_str(),
        pulse_one_liner,
        workflow_id: workflow_id.as_deref(),
        workflow_summary: Some(workflow_summary.as_str()),
    };
    let title = voice_service()
        .await
        .render_activity_title(gcx, voice_ctx, intent)
        .await;
    safe_generated_buddy_text(
        &title,
        fallback_title,
        BUDDY_STATIC_TITLE_FALLBACK,
        BUDDY_TITLE_MAX_CHARS,
    )
}

impl BuddyService {
    pub(crate) fn admission_identity(&self) -> Arc<()> {
        self.admission_owner.clone()
    }
    pub(crate) fn has_admission_identity(&self, owner: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.admission_owner, owner)
    }

    pub fn new(
        project_root: std::path::PathBuf,
        mut state: BuddyState,
        settings: BuddySettings,
        recent_diagnostics: Vec<super::diagnostics::DiagnosticContext>,
        runtime_queue: RuntimeQueue,
        events_tx: broadcast::Sender<BuddyEvent>,
        queue_writer: Option<mpsc::UnboundedSender<RuntimeQueueWriteOp>>,
    ) -> Self {
        let opportunity_queue = OpportunityQueue::from_state(
            state.opportunities.clone(),
            state.dismissed_history.clone(),
        );
        let dismissed_runtime_keys = runtime_queue
            .items
            .iter()
            .chain(runtime_queue.now_playing.iter())
            .filter(|event| event.dismissed)
            .filter_map(|event| event.dedupe_key.clone().map(|key| (key, Utc::now())))
            .collect();
        let opportunity_snapshot = opportunity_queue.snapshot();
        let dismissed_snapshot = opportunity_queue.dismissed_history_snapshot();
        let state_changed = state.opportunities.len() != opportunity_snapshot.len()
            || state.dismissed_history.len() != dismissed_snapshot.len();
        state.opportunities = opportunity_snapshot;
        state.dismissed_history = dismissed_snapshot;
        Self {
            admission_owner: Arc::new(()),
            project_root,
            state,
            settings,
            outcome_runtime: None,
            operator_open_failure: None,
            operator_manual_reopen_at: None,
            operator_reopen_task: None,
            operator_features: Default::default(),
            operator_opportunity_projection: Vec::new(),
            operator_legacy_seeded: false,
            operator_legacy_seed_cursor: 0,
            operator_legacy_history_seeded: false,
            operator_receipt_seed: Default::default(),
            operator_activity_seed: Default::default(),
            persisted_operator_drafts: HashSet::new(),
            admission_gate: Arc::new(tokio::sync::Mutex::new(())),
            events_tx,
            last_suggestion_at: None,
            recent_diagnostics,
            memory_ops: MemoryOpsState::default(),
            last_issue_at: None,
            recent_issue_errors: Vec::new(),
            runtime_queue,
            dismissed_runtime_keys,
            dirty: state_changed,
            active_speech: None,
            queue_writer,
            fact_store: FactStore::new(),
            opportunity_queue,
            opportunity_accept_claims: HashSet::new(),
            humor_service: Arc::new(tokio::sync::Mutex::new(HumorService::new())),
            pulse: BuddyPulse::default(),
            draft_store: DraftStore::new(),
            drafts_dirty: false,
            last_observer_tick: HashMap::new(),
            observers: build_observer_registry(),
            provider_rate_limit_window: Default::default(),
            background_tasks: Vec::new(),
            chat_reaction_limiter: crate::buddy::chat_reactions::ChatReactionLimiter::new(),
            chat_reaction_debug: crate::buddy::chat_reactions::ChatReactionDebugState::new(),
            auto_quiet_window: None,
            #[cfg(test)]
            force_next_runtime_enqueue_drop: false,
        }
    }

    /// Production rollout entry point. Capabilities follow
    /// `BuddySettings::operator_enabled`, which defaults to on; a store that cannot
    /// be opened leaves `outcome_runtime` at `None` so the status route reports
    /// `unavailable` instead of `enabled`, and nothing existing is rewritten.
    pub fn enable_operator_from_settings(&mut self) {
        self.operator_features =
            super::operator_runtime::OperatorFeatures::from_settings(&self.settings);
        if !self.operator_features.storage {
            self.operator_open_failure = None;
            return;
        }
        match super::operator_runtime::open_project_runtime(&self.project_root) {
            Ok(runtime) => {
                self.outcome_runtime = Some(runtime);
                self.operator_open_failure = None;
            }
            Err(error) => {
                self.record_operator_open_failure(error, tokio::time::Instant::now());
            }
        }
    }

    fn should_reopen_operator(&self, now: tokio::time::Instant) -> bool {
        self.operator_features.storage
            && self.outcome_runtime.is_none()
            && self
                .operator_open_failure
                .as_ref()
                .is_some_and(|failure| failure.is_due(now))
    }

    fn record_operator_open_failure(
        &mut self,
        error: super::operator_runtime::OperatorRuntimeOpenError,
        now: tokio::time::Instant,
    ) {
        self.record_operator_open_failure_reason(OperatorOpenFailureReason::from(error), now);
    }

    fn record_operator_open_failure_reason(
        &mut self,
        error: OperatorOpenFailureReason,
        now: tokio::time::Instant,
    ) {
        let should_log = match &mut self.operator_open_failure {
            Some(failure) => failure.record_failure(error, now),
            None => {
                self.operator_open_failure = Some(OperatorOpenFailure::new(error, now));
                true
            }
        };
        if should_log {
            let reason = self
                .operator_open_failure
                .as_ref()
                .map(|failure| failure.reason.to_string())
                .unwrap_or_else(|| "operator store unavailable".to_string());
            warn!("buddy: operator store unavailable, operator status stays unavailable: {reason}");
        }
    }

    pub(crate) fn operator_unavailable_status(&self) -> OperatorUnavailableStatus {
        let Some(failure) = self.operator_open_failure.as_ref() else {
            return OperatorUnavailableStatus {
                kind: OperatorOpenFailureKind::NotAttempted,
                detail: "Buddy has not opened saved work for this project yet.".to_string(),
                retry_in_seconds: None,
            };
        };
        let detail =
            crate::llm::safe_truncate(&redact_sensitive(&failure.reason.to_string()), 1_000)
                .trim()
                .to_string();
        OperatorUnavailableStatus {
            kind: failure.reason.kind(),
            detail: if detail.is_empty() {
                "Buddy could not open saved work.".to_string()
            } else {
                detail
            },
            retry_in_seconds: Some(
                failure
                    .next_retry_at
                    .saturating_duration_since(tokio::time::Instant::now())
                    .as_secs()
                    .max(1),
            ),
        }
    }

    fn manual_operator_reopen_root(&mut self, now: tokio::time::Instant) -> Option<PathBuf> {
        if !self.operator_features.storage
            || self.outcome_runtime.is_some()
            || self.operator_reopen_task.is_some()
            || self.operator_manual_reopen_at.is_some_and(|previous| {
                now.duration_since(previous) < OPERATOR_MANUAL_REOPEN_COOLDOWN
            })
        {
            return None;
        }
        self.operator_manual_reopen_at = Some(now);
        Some(self.project_root.clone())
    }

    fn record_manual_operator_open_failure(
        &mut self,
        error: super::operator_runtime::OperatorRuntimeOpenError,
        now: tokio::time::Instant,
    ) {
        let reason = OperatorOpenFailureReason::from(error);
        if let Some(failure) = self.operator_open_failure.as_mut() {
            failure.replace_reason_without_rescheduling(reason);
        } else {
            self.record_operator_open_failure_reason(reason, now);
        }
    }

    fn install_reopened_operator_runtime(
        &mut self,
        project_root: &Path,
        runtime: super::outcome_runtime::OutcomeRuntime,
    ) -> bool {
        if !self.operator_features.storage
            || self.outcome_runtime.is_some()
            || self.project_root != project_root
        {
            return false;
        }
        self.outcome_runtime = Some(runtime);
        self.operator_open_failure = None;
        true
    }

    fn begin_operator_reopen(&mut self, now: tokio::time::Instant) {
        if !self.should_reopen_operator(now) || self.operator_reopen_task.is_some() {
            return;
        }
        self.operator_reopen_task = Some(tokio::spawn(
            super::operator_runtime::open_project_runtime_async(self.project_root.clone()),
        ));
    }

    async fn shutdown_operator_reopen(&mut self) {
        if let Some(task) = self.operator_reopen_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    /// Push a record into the writer queue. The writer task applies them in
    /// strict order — see [`run_runtime_queue_writer`].
    fn persist_record(&self, record: RuntimeQueueRecord) {
        if let Some(tx) = &self.queue_writer {
            // send only fails if the receiver was dropped (shutdown). In that
            // case the data is no longer needed; nothing to do.
            let _ = tx.send(RuntimeQueueWriteOp::Append(record));
        }
    }

    fn persist_event(&self, event: BuddyRuntimeEvent) {
        self.persist_record(RuntimeQueueRecord::Event {
            event,
        });
    }

    fn persist_removal(&self, id: String) {
        self.persist_record(RuntimeQueueRecord::Removed {
            id,
        });
    }

    fn persist_now_playing(&self, slot: Option<BuddyRuntimeEvent>) {
        self.persist_record(RuntimeQueueRecord::NowPlaying {
            event: slot,
        });
    }

    pub fn track_background_task(&mut self, handle: JoinHandle<()>) {
        self.background_tasks.retain(|task| !task.is_finished());
        self.background_tasks.push(handle);
    }

    pub fn take_background_tasks(&mut self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.background_tasks)
    }

    /// Public state always projects Work authority; legacy fields on `state` are
    /// retained only as frozen migration/rollback input, never overwritten by this view.
    pub fn presentation_state(&self) -> BuddyState {
        let mut state = self.state.clone();
        if self.operator_features.storage {
            state.opportunities = self.operator_opportunity_projection.clone();
            state.suggestion_state.clear();
            state.dismissed_history.clear();
        }
        state
    }

    pub fn snapshot(&self) -> BuddySnapshot {
        let opportunities = if self.operator_features.storage {
            self.operator_opportunity_projection.clone()
        } else {
            self.opportunity_queue.snapshot()
        };
        let mut state = self.state.clone();
        state.opportunities = opportunities.clone();
        if self.operator_features.storage {
            state.suggestion_state.clear();
            state.dismissed_history.clear();
        }
        let mut pulse = self.pulse.clone();
        self.apply_memory_ops_to_pulse(&mut pulse);
        BuddySnapshot {
            state,
            settings: self.settings.clone(),
            enabled: self.settings.enabled,
            storage: Some(super::settings::storage_metadata(&self.project_root)),
            recent_diagnostics: self.recent_diagnostics.clone(),
            runtime_queue: self.runtime_queue.items.iter().cloned().collect(),
            now_playing: self.runtime_queue.now_playing.clone(),
            active_speech: self.active_speech.clone(),
            pulse,
            opportunities,
            active_drafts: self.draft_store.snapshot(),
            chat_reaction_debug: Some(self.chat_reaction_debug.snapshot()),
        }
    }

    pub fn expire_opportunities(&mut self) {
        if self.operator_features.storage {
            return;
        }
        let now = Utc::now();
        let expiring: Vec<String> = self
            .opportunity_queue
            .iter()
            .filter(|o| {
                o.expires_at <= now
                    && matches!(o.status, OpportunityStatus::New | OpportunityStatus::Shown)
            })
            .map(|o| o.id.clone())
            .collect();
        let changed = self.opportunity_queue.expire_old(now);
        if !changed {
            return;
        }
        for id in expiring {
            let _ = self.events_tx.send(BuddyEvent::OpportunityResolved {
                opportunity_id: id,
                status: OpportunityStatus::Expired,
            });
        }
        self.state.opportunities = self.opportunity_queue.snapshot();
        self.state.dismissed_history = self.opportunity_queue.dismissed_history_snapshot();
        self.dirty = true;
    }

    #[cfg(test)]
    pub fn add_opportunity(&mut self, opp: BuddyOpportunity) {
        self.add_opportunity_with_cooldown(
            opp,
            super::opportunities::DEFAULT_COOLDOWN.num_seconds() as u64,
        );
    }

    pub fn add_opportunity_with_cooldown(
        &mut self,
        opp: BuddyOpportunity,
        cooldown_secs: u64,
    ) -> bool {
        if self.operator_features.storage {
            tracing::warn!("operator producer requires durable async admission");
            return false;
        }
        if opp.cooldown_key == "provider:rate_limited:global" {
            let refreshed = if let Some(existing) = self
                .opportunity_queue
                .items
                .iter_mut()
                .find(|existing| existing.cooldown_key == opp.cooldown_key)
            {
                existing.summary = opp.summary.clone();
                existing.priority = opp.priority;
                existing.confidence = opp.confidence;
                existing.fact_keys = opp.fact_keys.clone();
                existing.proposed_actions = opp.proposed_actions.clone();
                existing.related = opp.related.clone();
                existing.expires_at = opp.expires_at;
                Some(existing.clone())
            } else {
                None
            };
            if let Some(opportunity) = refreshed {
                self.state.opportunities = self.opportunity_queue.snapshot();
                self.dirty = true;
                let _ = self.events_tx.send(BuddyEvent::OpportunityProduced {
                    opportunity,
                });
                return true;
            }
        }
        self.opportunity_queue
            .push_with_cooldown(opp.clone(), cooldown_secs);
        self.state.opportunities = self.opportunity_queue.snapshot();
        self.state.dismissed_history = self.opportunity_queue.dismissed_history_snapshot();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::OpportunityProduced {
            opportunity: opp,
        });
        true
    }

    pub fn surface_opportunity_with_cooldown(
        &mut self,
        mut opp: BuddyOpportunity,
        cooldown_secs: u64,
    ) -> bool {
        if self.operator_features.storage {
            tracing::warn!("operator producer requires durable async admission");
            return false;
        }
        if self.operator_features.editorial {
            opp.humor = None;
            opp.humor_allowed = false;
        }
        match evaluate_with_mutes(
            &opp,
            &self.settings,
            &self.opportunity_queue,
            &self.state.muted_rules,
        ) {
            PolicyDecision::Drop {
                reason,
            } => {
                tracing::debug!("buddy: opportunity dropped by policy: {}", reason);
                false
            }
            PolicyDecision::Surface {
                humor_allowed,
            } => {
                opp.humor_allowed = humor_allowed && !self.operator_features.editorial;
                let rule_key = refact_buddy_core::verdicts::rule_key_of(&opp.cooldown_key);
                let multiplier = refact_buddy_core::verdicts::cooldown_multiplier_for_rule(
                    &self.state.verdicts,
                    &rule_key,
                );
                self.add_opportunity_with_cooldown(opp, cooldown_secs.saturating_mul(multiplier))
            }
        }
    }

    /// Capture under the actor lock. The returned admission owns bounded capacity,
    /// but never the actor mutex; commit must be awaited after releasing that mutex.
    fn admission<T>(&self, value: T) -> Result<BuddyAdmission<T>, String> {
        let runtime = self
            .outcome_runtime
            .clone()
            .ok_or("Buddy operator store unavailable")?;
        if runtime.project().canonical_root != self.project_root.to_string_lossy() {
            return Err("Buddy operator project identity changed".into());
        }
        let permit = self
            .admission_gate
            .clone()
            .try_lock_owned()
            .map_err(|_| "Buddy operator admission busy; retry input")?;
        Ok(BuddyAdmission {
            value,
            pending: Some(AdmissionWrite {
                runtime,
                owner: self.admission_owner.clone(),
                permit: Some(permit),
                opportunities: Vec::new(),
                drafts: Vec::new(),
                memory: None,
            }),
        })
    }

    pub fn prepare_opportunity(
        &mut self,
        mut opportunity: BuddyOpportunity,
        cooldown_secs: u64,
    ) -> Result<BuddyAdmission<bool>, String> {
        if !self.operator_features.storage {
            return Ok(BuddyAdmission::ready(
                self.surface_opportunity_with_cooldown(opportunity, cooldown_secs),
            ));
        }
        let rule = refact_buddy_core::verdicts::rule_key_of(&opportunity.cooldown_key);
        if self.state.muted_rules.contains(&rule) {
            return Ok(BuddyAdmission::ready(false));
        }
        if serde_json::to_vec(&opportunity)
            .map_err(|error| error.to_string())?
            .len()
            > 32_768
        {
            return Err("operator opportunity exceeds bounded payload capacity".into());
        }
        if opportunity.cooldown_key.trim().is_empty() {
            return Err("operator source identity missing".into());
        }
        opportunity.id = format!(
            "producer:{}",
            super::work_items::stable_hash(&[&opportunity.cooldown_key])
        );
        opportunity.humor = None;
        opportunity.humor_allowed = false;
        let mut admission = self.admission(true)?;
        admission
            .pending
            .as_mut()
            .unwrap()
            .opportunities
            .push(opportunity);
        Ok(admission)
    }

    pub fn prepare_suggestion(
        &mut self,
        suggestion: BuddySuggestion,
    ) -> Result<BuddyAdmission<bool>, String> {
        if !self.operator_features.storage {
            return Ok(BuddyAdmission::ready(self.maybe_add_suggestion(suggestion)));
        }
        self.prepare_opportunity(operator_suggestion(&suggestion), SUGGESTION_RATE_LIMIT_SECS)
    }

    pub fn prepare_draft(
        &mut self,
        kind: super::types::DraftKind,
        title: String,
        yaml_or_json: String,
        explanation: String,
    ) -> Result<BuddyAdmission<BuddyDraft>, String> {
        if !self.operator_features.storage {
            return self
                .create_draft_legacy(kind, title, yaml_or_json, explanation)
                .map(BuddyAdmission::ready)
                .map_err(|error| error.to_string());
        }
        validate_draft_payload(&title, &yaml_or_json, &explanation)
            .map_err(|error| error.to_string())?;
        if yaml_or_json
            .len()
            .saturating_add(explanation.len())
            .saturating_add(2)
            > 32_768
        {
            return Err("operator proposal exceeds bounded payload capacity".into());
        }
        if self.draft_store.snapshot().len() >= super::legacy_work::MAX_MIGRATION_BATCH - 1 {
            return Err("operator draft admission capacity exhausted".into());
        }
        let now = Utc::now();
        let draft = BuddyDraft {
            id: Uuid::new_v4().to_string(),
            kind,
            title,
            yaml_or_json,
            explanation,
            created_at: now,
            expires_at: now + super::drafts::DRAFT_TTL,
        };
        let mut admission = self.admission(draft.clone())?;
        admission.pending.as_mut().unwrap().drafts.push(draft);
        Ok(admission)
    }

    pub async fn surface_opportunity_durable(
        buddy: &Arc<tokio::sync::Mutex<Option<Self>>>,
        opportunity: BuddyOpportunity,
        cooldown_secs: u64,
    ) -> Result<bool, String> {
        let admission = buddy
            .lock()
            .await
            .as_mut()
            .ok_or("Buddy unavailable")?
            .prepare_opportunity(opportunity, cooldown_secs)?;
        admission.commit(buddy).await
    }

    pub async fn create_draft_durable(
        buddy: &Arc<tokio::sync::Mutex<Option<Self>>>,
        kind: super::types::DraftKind,
        title: String,
        yaml_or_json: String,
        explanation: String,
    ) -> Result<BuddyDraft, String> {
        let admission = buddy
            .lock()
            .await
            .as_mut()
            .ok_or("Buddy unavailable")?
            .prepare_draft(kind, title, yaml_or_json, explanation)?;
        admission.commit(buddy).await
    }

    pub fn record_verdict(
        &mut self,
        cooldown_key: &str,
        kind: super::types::BuddyOpportunityKind,
        action_kind: &str,
        outcome: refact_buddy_core::verdicts::VerdictOutcome,
    ) {
        let verdict = refact_buddy_core::verdicts::BuddyVerdict {
            rule_key: refact_buddy_core::verdicts::rule_key_of(cooldown_key),
            kind: serde_json::to_value(kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
            action_kind: action_kind.to_string(),
            verdict: outcome,
            at: Utc::now(),
        };
        refact_buddy_core::verdicts::record_verdict(&mut self.state.verdicts, verdict);
        self.dirty = true;
    }

    pub fn mute_rule_for_cooldown_key(&mut self, cooldown_key: &str) -> bool {
        let changed =
            refact_buddy_core::verdicts::mute_rule(&mut self.state.muted_rules, cooldown_key);
        if changed {
            self.dirty = true;
        }
        changed
    }

    pub fn unmute_rule(&mut self, rule_key: &str) -> bool {
        let changed =
            refact_buddy_core::verdicts::unmute_rule(&mut self.state.muted_rules, rule_key);
        if changed {
            self.dirty = true;
        }
        changed
    }

    pub fn set_memory_ops(&mut self, memory_ops: MemoryOpsState) {
        self.memory_ops = memory_ops;
        if !self.operator_features.storage {
            for opp in self.memory_batch_opportunities() {
                self.surface_opportunity_with_cooldown(opp, 24 * 60 * 60);
            }
        }
    }

    pub fn prepare_memory_ops(
        &mut self,
        memory_ops: MemoryOpsState,
    ) -> Result<BuddyAdmission<()>, String> {
        if !self.operator_features.storage {
            self.set_memory_ops(memory_ops);
            return Ok(BuddyAdmission::ready(()));
        }
        let mut admission = self.admission(())?;
        let pending = admission.pending.as_mut().unwrap();
        pending.opportunities = Self::opportunities_for_memory(&memory_ops);
        pending.opportunities.retain(|opportunity| {
            !self
                .state
                .muted_rules
                .contains(&refact_buddy_core::verdicts::rule_key_of(
                    &opportunity.cooldown_key,
                ))
        });
        pending.memory = Some((self.memory_ops.clone(), memory_ops));
        Ok(admission)
    }

    pub async fn set_memory_ops_durable(
        buddy: &Arc<tokio::sync::Mutex<Option<Self>>>,
        memory_ops: MemoryOpsState,
    ) -> Result<(), String> {
        let admission = buddy
            .lock()
            .await
            .as_mut()
            .ok_or("Buddy unavailable")?
            .prepare_memory_ops(memory_ops)?;
        admission.commit(buddy).await
    }

    fn memory_batch_opportunities(&self) -> Vec<BuddyOpportunity> {
        Self::opportunities_for_memory(&self.memory_ops)
    }

    fn opportunities_for_memory(memory_ops: &MemoryOpsState) -> Vec<BuddyOpportunity> {
        let mut opportunities = Vec::new();
        let batches = refact_buddy_core::memory_lifecycle_model::memory_op_batches(&memory_ops.ops);
        let now = Utc::now();
        for batch in batches {
            if batch.count == 0 {
                continue;
            }
            let label = refact_buddy_core::memory_lifecycle_model::memory_op_batch_label(
                &batch.batch_key,
                batch.count,
            );
            let summary = if batch.preview.is_empty() {
                label
            } else {
                format!("{} — e.g. {}", label, batch.preview.join("; "))
            };
            let opp = BuddyOpportunity {
                id: Uuid::new_v4().to_string(),
                kind: super::types::BuddyOpportunityKind::MemoryOpsBatch,
                summary,
                priority: super::types::BuddyPriority::Normal,
                confidence: 0.9,
                fact_keys: vec![],
                cooldown_key: format!("memory_ops_batch:{}", batch.batch_key),
                cooldown_secs: 24 * 60 * 60,
                status: OpportunityStatus::New,
                proposed_actions: vec![
                    super::types::BuddyAction::ApplyMemoryBatch {
                        batch_key: batch.batch_key.clone(),
                        count_hint: batch.count,
                    },
                    super::types::BuddyAction::Dismiss,
                ],
                humor: None,
                humor_allowed: false,
                related: super::types::BuddyOpportunityLinks::default(),
                created_at: now,
                expires_at: now + chrono::Duration::hours(24),
                resolved_at: None,
            };
            opportunities.push(opp);
        }
        opportunities
    }

    pub fn accept_quest_from_suggestion(
        &mut self,
        suggestion_id: &str,
    ) -> Result<(BuddySuggestion, BuddyQuest), String> {
        if self.operator_features.storage {
            return Err("Legacy suggestion actions are unavailable; use Work Desk".into());
        }
        let suggestion = self
            .state
            .suggestion_state
            .iter()
            .find(|suggestion| suggestion.id == suggestion_id)
            .cloned()
            .ok_or_else(|| format!("suggestion not found: {}", suggestion_id))?;
        let quest = suggestion
            .quest
            .clone()
            .ok_or_else(|| format!("suggestion is not a quest: {}", suggestion_id))?;
        self.dismiss_suggestion(suggestion_id);
        super::state::activate_quest(&mut self.state, quest.clone());
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.presentation_state(),
        });
        Ok((suggestion, quest))
    }

    pub fn claim_opportunity_accept(&mut self, id: &str) -> bool {
        self.opportunity_accept_claims.insert(id.to_string())
    }

    pub fn clear_opportunity_accept_claim(&mut self, id: &str) {
        self.opportunity_accept_claims.remove(id);
    }

    pub fn is_opportunity_accept_claimed(&self, id: &str) -> bool {
        self.opportunity_accept_claims.contains(id)
    }

    pub fn resolve_opportunity(&mut self, id: &str, status: OpportunityStatus) -> bool {
        if self.operator_features.storage {
            return false;
        }
        let changed = if matches!(status, OpportunityStatus::Dismissed) {
            self.opportunity_queue.dismiss(id)
        } else {
            self.opportunity_queue.mark_status(id, status)
        };
        if !changed {
            return false;
        }
        self.state.opportunities = self.opportunity_queue.snapshot();
        self.state.dismissed_history = self.opportunity_queue.dismissed_history_snapshot();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::OpportunityResolved {
            opportunity_id: id.to_string(),
            status,
        });
        true
    }

    pub fn set_pulse(&mut self, pulse: BuddyPulse) {
        let mut pulse = pulse;
        self.apply_memory_ops_to_pulse(&mut pulse);
        self.pulse = pulse.clone();
        let _ = self.events_tx.send(BuddyEvent::PulseUpdated {
            pulse,
        });
    }

    fn apply_memory_ops_to_pulse(&self, pulse: &mut BuddyPulse) {
        pulse.memory.pending_ops = self.memory_ops.pending_count + self.memory_ops.approved_count;
        pulse.memory.applied_ops = self.memory_ops.applied_count;
        pulse.memory.failed_ops = self.memory_ops.failed_count;
        let counts = memory_lifecycle_op_counts(&self.memory_ops.ops);
        pulse.memory.duplicate_candidates = counts.duplicate_candidates;
        pulse.memory.merge_candidates = counts.merge_candidates;
        pulse.memory.archive_candidates = counts.archive_candidates;
        pulse.memory.review_candidates = counts.review_candidates;
        pulse.memory.conflict_candidates = counts.conflict_candidates;
    }

    #[cfg(test)]
    pub fn create_draft(
        &mut self,
        kind: super::types::DraftKind,
        title: String,
        yaml_or_json: String,
        explanation: String,
    ) -> Result<BuddyDraft, String> {
        self.create_draft_legacy(kind, title, yaml_or_json, explanation)
    }

    fn create_draft_legacy(
        &mut self,
        kind: super::types::DraftKind,
        title: String,
        yaml_or_json: String,
        explanation: String,
    ) -> Result<BuddyDraft, String> {
        if self.operator_features.storage {
            return Err("operator draft requires durable async admission".into());
        }
        validate_draft_payload(&title, &yaml_or_json, &explanation)
            .map_err(|error| error.to_string())?;
        let draft = self
            .draft_store
            .create(kind, title, yaml_or_json, explanation);
        self.drafts_dirty = true;
        if !self.operator_features.storage {
            let _ = self.events_tx.send(BuddyEvent::DraftCreated {
                draft: draft.clone(),
            });
        }
        Ok(draft)
    }

    pub fn delete_draft(&mut self, id: &str) -> Option<BuddyDraft> {
        if self.operator_features.storage && !self.persisted_operator_drafts.contains(id) {
            return None;
        }
        let draft = self.draft_store.delete(id)?;
        self.persisted_operator_drafts.remove(id);
        self.drafts_dirty = true;
        let _ = self.events_tx.send(BuddyEvent::DraftRemoved {
            draft_id: id.to_string(),
        });
        Some(draft)
    }

    pub fn consume_draft(&mut self, id: &str) -> Option<BuddyDraft> {
        if self.operator_features.storage && !self.persisted_operator_drafts.contains(id) {
            return None;
        }
        let draft = self.draft_store.consume(id)?;
        self.persisted_operator_drafts.remove(id);
        self.drafts_dirty = true;
        let _ = self.events_tx.send(BuddyEvent::DraftConsumed {
            draft_id: id.to_string(),
        });
        Some(draft)
    }

    pub fn expire_drafts(&mut self, now: DateTime<Utc>) -> Vec<String> {
        let expired = if self.operator_features.storage {
            let ids: Vec<_> = self
                .draft_store
                .snapshot()
                .into_iter()
                .filter(|draft| {
                    draft.expires_at <= now && self.persisted_operator_drafts.contains(&draft.id)
                })
                .map(|draft| draft.id)
                .collect();
            for id in &ids {
                self.draft_store.delete(id);
                self.persisted_operator_drafts.remove(id);
            }
            ids
        } else {
            self.draft_store.expire_old(now)
        };
        if !expired.is_empty() {
            self.drafts_dirty = true;
        }
        for id in &expired {
            let _ = self.events_tx.send(BuddyEvent::DraftRemoved {
                draft_id: id.clone(),
            });
        }
        expired
    }

    pub fn consume_validated_draft(
        &mut self,
        id: &str,
        expected_kind: super::types::DraftKind,
        target: DraftTarget<'_>,
    ) -> Result<BuddyDraft, DraftValidationError> {
        self.draft_store.get_validated(id, expected_kind, target)?;
        self.consume_draft(id).ok_or(DraftValidationError::NotFound)
    }

    #[cfg(test)]
    pub fn detect_and_surface(&mut self) {
        let candidates = OpportunityDetector::new().detect(
            &self.fact_store,
            &self.pulse,
            &self.opportunity_queue,
        );
        for (opp, cooldown_secs) in candidates {
            self.surface_opportunity_with_cooldown(opp, cooldown_secs);
        }
    }

    pub fn update_speech(&mut self, speech: BuddySpeechItem) -> bool {
        self.update_speech_gated(speech, false)
    }

    pub fn update_speech_user_initiated(&mut self, speech: BuddySpeechItem) -> bool {
        self.update_speech_gated(speech, true)
    }

    fn update_speech_gated(&mut self, speech: BuddySpeechItem, user_initiated: bool) -> bool {
        if self.operator_features.editorial
            && !user_initiated
            && !speech.editorial_delivery_required
        {
            return false;
        }
        let intent = speech
            .speech_intent
            .as_deref()
            .and_then(super::speech_policy::parse_intent_key);
        let now = Utc::now();
        let local_hour = chrono::Timelike::hour(&chrono::Local::now());
        let decision = if user_initiated {
            super::speech_policy::gate_speech_user_initiated(
                &self.settings,
                intent,
                speech.chat_id.as_deref(),
            )
        } else {
            super::speech_policy::gate_speech(
                &self.settings,
                &self.state.speech_rotation,
                intent,
                speech.chat_id.as_deref(),
                local_hour,
                self.auto_quiet_window,
                now,
            )
        };
        self.record_speech_decision(&speech, intent, decision);
        if !decision.allowed {
            tracing::debug!("buddy: speech dropped by arbiter: {}", decision.reason);
            return false;
        }
        if let Some(intent) = intent {
            super::speech_policy::record_emission(&mut self.state.speech_rotation, intent, now);
            self.dirty = true;
        }
        if let Some(key) = &speech.dedupe_key {
            if let Some(existing) = &self.active_speech {
                if existing.dedupe_key.as_deref() == Some(key.as_str()) {
                    self.active_speech = Some(speech.clone());
                    let _ = self.events_tx.send(BuddyEvent::SpeechUpdated {
                        speech,
                    });
                    return true;
                }
            }
        }
        self.active_speech = Some(speech.clone());
        let _ = self.events_tx.send(BuddyEvent::SpeechUpdated {
            speech,
        });
        true
    }

    fn record_speech_decision(
        &mut self,
        speech: &BuddySpeechItem,
        intent: Option<SpeechIntent>,
        decision: super::speech_policy::SpeechGateDecision,
    ) {
        let record = super::types::SpeechDecisionRecord {
            at: Utc::now(),
            intent: intent.map(|i| super::speech_policy::intent_key(i).to_string()),
            allowed: decision.allowed,
            reason: decision.reason.to_string(),
            preview: crate::llm::safe_truncate(&speech.text, 120).to_string(),
            source: speech
                .dedupe_key
                .clone()
                .unwrap_or_else(|| speech.id.clone()),
        };
        self.state.speech_decisions.push(record);
        let overflow = self.state.speech_decisions.len().saturating_sub(50);
        if overflow > 0 {
            self.state.speech_decisions.drain(..overflow);
        }
        self.dirty = true;
    }

    pub fn send_navigation(&self, page: super::types::BuddyPage) {
        let _ = self.events_tx.send(BuddyEvent::NavigationRequest {
            page,
        });
    }

    pub fn enqueue_runtime_event(&mut self, event: BuddyRuntimeEvent) {
        let _ = self.enqueue_runtime_event_with_stored(event);
    }

    fn runtime_event_intent(signal_type: &str) -> Option<SpeechIntent> {
        if let Some(token) = signal_type.strip_prefix("speech_") {
            return super::speech_policy::parse_intent_key(token);
        }
        if signal_type == "chat_bug_candidate" {
            return Some(SpeechIntent::ErrorAlert);
        }
        None
    }

    fn gate_runtime_event_speech(&mut self, mut event: BuddyRuntimeEvent) -> BuddyRuntimeEvent {
        if self.operator_features.editorial {
            // Preserve native warnings and controls, not a second cosmetic delivery channel.
            event.speech_text = None;
            return event;
        }
        let Some(speech_text) = event.speech_text.clone() else {
            return event;
        };
        let intent = Self::runtime_event_intent(&event.signal_type);
        let local_hour = chrono::Timelike::hour(&chrono::Local::now());
        let decision = super::speech_policy::gate_speech(
            &self.settings,
            &self.state.speech_rotation,
            intent,
            event.chat_id.as_deref(),
            local_hour,
            self.auto_quiet_window,
            Utc::now(),
        );
        if !decision.allowed && decision.reason != "intent_budget" {
            let preview_item = BuddySpeechItem {
                editorial_delivery_required: false,
                id: event.id.clone(),
                text: speech_text,
                mood: "neutral".to_string(),
                scope: "global".to_string(),
                persistent: false,
                ttl_seconds: 10,
                dedupe_key: event.dedupe_key.clone(),
                speech_intent: intent.map(|i| super::speech_policy::intent_key(i).to_string()),
                created_at: event.created_at.clone(),
                controls: vec![],
                chat_id: event.chat_id.clone(),
            };
            self.record_speech_decision(&preview_item, intent, decision);
            event.speech_text = None;
        }
        event
    }

    pub fn gate_chat_reaction_event(&mut self, event: &BuddyRuntimeEvent) -> bool {
        let Some(speech_text) = event.speech_text.clone() else {
            return true;
        };
        let intent = Self::runtime_event_intent(&event.signal_type);
        let local_hour = chrono::Timelike::hour(&chrono::Local::now());
        let now = Utc::now();
        let decision = super::speech_policy::gate_speech(
            &self.settings,
            &self.state.speech_rotation,
            intent,
            event.chat_id.as_deref(),
            local_hour,
            self.auto_quiet_window,
            now,
        );
        let preview_item = BuddySpeechItem {
            editorial_delivery_required: false,
            id: event.id.clone(),
            text: speech_text,
            mood: "neutral".to_string(),
            scope: "global".to_string(),
            persistent: false,
            ttl_seconds: 10,
            dedupe_key: event.dedupe_key.clone(),
            speech_intent: intent.map(|i| super::speech_policy::intent_key(i).to_string()),
            created_at: event.created_at.clone(),
            controls: vec![],
            chat_id: event.chat_id.clone(),
        };
        self.record_speech_decision(&preview_item, intent, decision);
        if !decision.allowed {
            tracing::debug!(
                "buddy: chat reaction speech dropped by arbiter: {}",
                decision.reason
            );
            return false;
        }
        if let Some(intent) = intent {
            super::speech_policy::record_emission(&mut self.state.speech_rotation, intent, now);
            self.dirty = true;
        }
        true
    }

    pub fn enqueue_runtime_event_with_stored(
        &mut self,
        event: BuddyRuntimeEvent,
    ) -> Option<BuddyRuntimeEvent> {
        #[cfg(test)]
        if self.force_next_runtime_enqueue_drop {
            self.force_next_runtime_enqueue_drop = false;
            return None;
        }
        let event = self.gate_runtime_event_speech(event);
        let event = self.apply_runtime_dismissal_memory(event);
        let dedupe_key = event.dedupe_key.clone();
        let input_id = event.id.clone();
        let evicted = self.runtime_queue.enqueue(event);
        // Broadcast and persist the coalesced/stored event so SSE consumers and
        // dismiss_runtime_event_by_id agree on the id after dedupe.
        let to_persist = if let Some(key) = dedupe_key.as_deref() {
            self.runtime_queue
                .items
                .iter()
                .find(|e| e.dedupe_key.as_deref() == Some(key))
                .cloned()
                .or_else(|| {
                    self.runtime_queue
                        .now_playing
                        .as_ref()
                        .filter(|e| e.dedupe_key.as_deref() == Some(key))
                        .cloned()
                })
        } else {
            self.runtime_queue
                .items
                .iter()
                .find(|e| e.id == input_id)
                .cloned()
        };
        if let Some(ev) = to_persist.as_ref() {
            let _ = self.events_tx.send(BuddyEvent::RuntimeEvent {
                event: ev.clone(),
            });
            self.persist_event(ev.clone());
        }
        if dedupe_key.is_some()
            && self
                .runtime_queue
                .now_playing
                .as_ref()
                .map(|np| np.dedupe_key == dedupe_key)
                .unwrap_or(false)
        {
            self.persist_now_playing(self.runtime_queue.now_playing.clone());
        }
        // Tombstone every evicted id so replay matches in-memory state.
        for id in evicted {
            self.persist_removal(id);
        }
        to_persist
    }

    pub fn expire_runtime_events_at(&mut self, now: DateTime<Utc>) -> Vec<String> {
        let removed = self.runtime_queue.prune_expired_at(now);
        for id in &removed {
            self.persist_removal(id.clone());
        }
        removed
    }

    fn apply_runtime_dismissal_memory(
        &mut self,
        mut event: BuddyRuntimeEvent,
    ) -> BuddyRuntimeEvent {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::hours(24);
        self.dismissed_runtime_keys
            .retain(|_, dismissed_at| *dismissed_at >= cutoff);
        if let Some(key) = event.dedupe_key.as_deref() {
            if self
                .dismissed_runtime_keys
                .get(key)
                .map(|dismissed_at| *dismissed_at >= cutoff)
                .unwrap_or(false)
            {
                event.dismissed = true;
            }
        }
        event
    }

    pub fn complete_runtime_event(&mut self, dedupe_key: &str, status: &str) {
        self.runtime_queue.complete(dedupe_key, status);
        if let Some(e) = self
            .runtime_queue
            .items
            .iter()
            .find(|e| e.dedupe_key.as_deref() == Some(dedupe_key))
            .cloned()
        {
            let _ = self.events_tx.send(BuddyEvent::RuntimeEvent {
                event: e.clone(),
            });
            self.persist_event(e);
        }
        if self
            .runtime_queue
            .now_playing
            .as_ref()
            .and_then(|np| np.dedupe_key.as_deref())
            == Some(dedupe_key)
        {
            self.persist_now_playing(self.runtime_queue.now_playing.clone());
        }
    }

    /// Mark a runtime event as dismissed by its `id` (frontend-visible identifier).
    /// The event stays in the queue with `dismissed: true` so the dismissal
    /// persists across snapshot reloads. Emits a RuntimeEvent so all clients
    /// see the updated flag immediately.
    /// Returns true if a matching event was found and updated.
    pub fn dismiss_runtime_event_by_id(&mut self, id: &str) -> bool {
        let mut found = false;
        let mut updated_event: Option<BuddyRuntimeEvent> = None;
        if let Some(e) = self.runtime_queue.items.iter_mut().find(|e| e.id == id) {
            e.dismissed = true;
            updated_event = Some(e.clone());
            found = true;
        }
        if let Some(ref mut np) = self.runtime_queue.now_playing {
            if np.id == id {
                np.dismissed = true;
                updated_event = Some(np.clone());
                found = true;
            }
        }
        if let Some(event) = updated_event {
            if let Some(key) = event.dedupe_key.as_ref() {
                self.dismissed_runtime_keys.insert(key.clone(), Utc::now());
            }
            self.dirty = true;
            let _ = self.events_tx.send(BuddyEvent::RuntimeEvent {
                event: event.clone(),
            });
            self.persist_event(event);
            // The dismiss path may also have flipped the `dismissed` flag on
            // now_playing; record the slot's current state so replay sees it.
            if self
                .runtime_queue
                .now_playing
                .as_ref()
                .map(|np| np.id == id)
                .unwrap_or(false)
            {
                self.persist_now_playing(self.runtime_queue.now_playing.clone());
            }
        }
        found
    }

    pub fn add_activity(&mut self, activity: BuddyActivity) {
        super::state::add_activity(&mut self.state, activity.clone());
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::ActivityAdded {
            activity,
        });
    }

    pub fn refresh_active_quest(&mut self) {
        let progressed = super::state::refresh_active_quest_progress(&mut self.state);
        let completed = self
            .state
            .active_quest
            .as_ref()
            .map(|quest| quest.status == "active" && quest.progress >= quest.goal)
            .unwrap_or(false);

        if !completed {
            if progressed {
                self.dirty = true;
                let _ = self.events_tx.send(BuddyEvent::StateUpdated {
                    state: self.presentation_state(),
                });
            }
            return;
        }

        let Some(quest) = super::state::complete_active_quest(&mut self.state) else {
            return;
        };

        let title = quest.title.clone();
        let icon = quest.icon.clone();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.presentation_state(),
        });

        self.add_activity(BuddyActivity {
            icon,
            title: format!("Quest complete: {title}"),
            description: format!(
                "{} wrapped up '{title}' and earned a growth boost.",
                self.state.identity.name
            ),
            timestamp: Utc::now().to_rfc3339(),
            activity_type: "quest_completed".to_string(),
            chat_id: None,
            failure_category: None,
            failure_summary: None,
        });
        self.update_speech_user_initiated(BuddySpeechItem {
            editorial_delivery_required: false,
            id: format!("quest-complete-{}", quest.id),
            text: format!("Quest complete: {title}! Tiny victory dance?"),
            mood: "happy".to_string(),
            scope: "global".to_string(),
            persistent: false,
            ttl_seconds: 12,
            dedupe_key: Some(format!("quest_complete_{}", quest.quest_type)),
            speech_intent: Some(SpeechIntent::QuestComplete.wire_token().to_string()),
            created_at: Utc::now().to_rfc3339(),
            controls: vec![],
            chat_id: None,
        });
        self.enqueue_runtime_event(BuddyRuntimeEvent {
            speech_text: Some(format!("Quest complete: {title}")),
            scene: Some("celebrate".to_string()),
            duration_hint: Some(10),
            persistent: false,
            controls: vec![],
            chat_id: None,
            failure_category: None,
            failure_summary: None,
            ..make_runtime_event(
                "task_completed",
                &format!("Quest complete: {title}"),
                "buddy_quest",
                &format!("quest_complete_{}", quest.quest_type),
                "completed",
                Some("high"),
            )
        });
    }

    pub fn dismiss_quest(&mut self) {
        super::state::clear_active_quest(&mut self.state);
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.presentation_state(),
        });
    }

    pub fn grant_xp(&mut self, amount: u64) {
        self.grant_xp_from(amount, super::state::XpSource::ActivityVolume);
    }

    pub fn grant_play_xp(&mut self, amount: u64) {
        self.grant_xp_from(amount, super::state::XpSource::IntentionalPlay);
    }

    pub fn grant_xp_from(&mut self, amount: u64, source: super::state::XpSource) {
        if !source.rewards_progression() {
            return;
        }
        super::state::grant_xp_from(&mut self.state, amount, source);
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.presentation_state(),
        });
    }

    pub fn apply_pet_tick(&mut self, elapsed_seconds: u64) {
        if !self.settings.enabled {
            return;
        }
        if !super::state::apply_pet_tick(&mut self.state, elapsed_seconds) {
            return;
        }
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.presentation_state(),
        });
    }

    pub fn add_suggestion(&mut self, suggestion: BuddySuggestion) {
        if self.operator_features.storage {
            self.add_opportunity_with_cooldown(
                operator_suggestion(&suggestion),
                SUGGESTION_RATE_LIMIT_SECS,
            );
            return;
        }
        if self.state.suggestion_state.len() >= 50 {
            if let Some(pos) = self.state.suggestion_state.iter().position(|s| s.dismissed) {
                self.state.suggestion_state.remove(pos);
            }
        }
        if let Some(existing) = self
            .state
            .suggestion_state
            .iter_mut()
            .find(|existing| existing.id == suggestion.id)
        {
            *existing = suggestion.clone();
        } else {
            self.state.suggestion_state.push(suggestion.clone());
        }
        self.last_suggestion_at = Some(Instant::now());
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::SuggestionAdded {
            suggestion,
        });
    }

    pub fn maybe_add_suggestion(&mut self, suggestion: BuddySuggestion) -> bool {
        if self.operator_features.storage {
            return self.surface_opportunity_with_cooldown(
                operator_suggestion(&suggestion),
                SUGGESTION_RATE_LIMIT_SECS,
            );
        }
        if let Some(last) = self.last_suggestion_at {
            if last.elapsed().as_secs() < SUGGESTION_RATE_LIMIT_SECS {
                return false;
            }
        }
        let dupe = self.state.suggestion_state.iter().any(|s| {
            !s.dismissed
                && s.suggestion_type == suggestion.suggestion_type
                && s.title == suggestion.title
        });
        if dupe {
            return false;
        }
        self.add_suggestion(suggestion);
        true
    }

    pub fn dismiss_suggestion(&mut self, id: &str) {
        if self.operator_features.storage {
            return;
        }
        if let Some(s) = self.state.suggestion_state.iter_mut().find(|s| s.id == id) {
            s.dismissed = true;
        }
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::SuggestionDismissed {
            suggestion_id: id.to_string(),
        });
    }

    pub fn workflow_failed(&mut self, workflow_id: &str, activity: super::types::BuddyActivity) {
        self.add_activity(activity);
        let now = Utc::now().to_rfc3339();
        if let Some(ws) = self
            .state
            .workflow_summaries
            .iter_mut()
            .find(|w| w.workflow_id == workflow_id)
        {
            ws.last_run = Some(now);
            ws.run_count += 1;
            ws.last_outcome = Some("failed".to_string());
        } else {
            self.state
                .workflow_summaries
                .push(super::types::BuddyWorkflowSummary {
                    workflow_id: workflow_id.to_string(),
                    last_run: Some(now),
                    run_count: 1,
                    failure_category: None,
                    failure_summary: None,
                    last_outcome: Some("failed".to_string()),
                    ..Default::default()
                });
        }
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.presentation_state(),
        });
    }

    pub fn record_provider_rate_limit_result(
        &mut self,
        workflow_id: &str,
        category: Option<&refact_buddy_core::workflows::WorkflowFailureCategory>,
    ) {
        if category.is_some_and(|category| {
            *category == refact_buddy_core::workflows::WorkflowFailureCategory::ProviderRateLimit
        }) {
            self.provider_rate_limit_window
                .record_failure(workflow_id, Utc::now());
        }
    }

    pub fn record_provider_rate_limit_recovery(
        &mut self,
    ) -> Option<super::outcome_runtime::OutcomeRuntime> {
        if !self.provider_rate_limit_window.record_success() {
            return None;
        }
        let id = self
            .opportunity_queue
            .iter()
            .find(|opportunity| opportunity.cooldown_key == "provider:rate_limited:global")
            .map(|opportunity| opportunity.id.clone());
        if let Some(id) = id {
            self.opportunity_queue
                .mark_status(&id, OpportunityStatus::Completed);
            self.state.opportunities = self.opportunity_queue.snapshot();
            self.dirty = true;
        }
        self.outcome_runtime.clone()
    }

    pub fn record_workflow_telemetry(
        &mut self,
        workflow_id: &str,
        llm_calls: u64,
        tokens_in: u64,
        tokens_out: u64,
        produced_output: bool,
    ) {
        if llm_calls == 0 && tokens_in == 0 && tokens_out == 0 && !produced_output {
            return;
        }
        let summary = if let Some(idx) = self
            .state
            .workflow_summaries
            .iter()
            .position(|w| w.workflow_id == workflow_id)
        {
            &mut self.state.workflow_summaries[idx]
        } else {
            self.state
                .workflow_summaries
                .push(super::types::BuddyWorkflowSummary {
                    workflow_id: workflow_id.to_string(),
                    ..Default::default()
                });
            self.state.workflow_summaries.last_mut().unwrap()
        };
        summary.llm_calls = summary.llm_calls.saturating_add(llm_calls);
        summary.tokens_in = summary.tokens_in.saturating_add(tokens_in);
        summary.tokens_out = summary.tokens_out.saturating_add(tokens_out);
        if produced_output {
            summary.outputs = summary.outputs.saturating_add(1);
            summary.last_output_at = Some(Utc::now().to_rfc3339());
        }
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.state
            .llm_spend
            .record(&today, llm_calls, tokens_in, tokens_out);
        self.dirty = true;
    }

    pub fn llm_budget_exhausted(&self) -> bool {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.state
            .llm_spend
            .is_over_budget(&today, self.settings.daily_llm_token_budget)
    }

    pub fn record_workflow_failure_report(
        &mut self,
        report: super::workflows::WorkflowFailureReport,
    ) -> Option<(std::path::PathBuf, super::workflows::WorkflowFailureReport)> {
        self.record_provider_rate_limit_result(&report.workflow_id, Some(&report.category));
        if !validate_workflow_id(&report.workflow_id) {
            tracing::warn!(
                "buddy: refusing to record failure report with invalid workflow_id '{}'",
                report.workflow_id
            );
            return None;
        }
        let title = format!(
            "{} failed: {}",
            super::workflows::workflow_label(&report.workflow_id),
            report.category.title()
        );
        let summary = redact_sensitive(&report.summary);
        let detail = redact_sensitive(&report.detail);
        let description = if summary.trim().is_empty() {
            detail.clone()
        } else {
            summary.clone()
        };
        let activity = BuddyActivity {
            icon: "⚠️".to_string(),
            title: title.clone(),
            description: crate::llm::safe_truncate(&description, 240).to_string(),
            timestamp: Utc::now().to_rfc3339(),
            activity_type: report.workflow_id.clone(),
            chat_id: report.chat_id.clone(),
            failure_category: Some(report.category.as_str().to_string()),
            failure_summary: Some(summary.clone()),
        };
        self.add_activity(activity);

        let now = Utc::now().to_rfc3339();
        let outcome = format!("failed:{}", report.category.as_str());
        if let Some(ws) = self
            .state
            .workflow_summaries
            .iter_mut()
            .find(|w| w.workflow_id == report.workflow_id)
        {
            ws.last_run = Some(now.clone());
            ws.run_count += 1;
            ws.failure_category = Some(report.category.as_str().to_string());
            ws.failure_summary = Some(summary.clone());
            ws.last_outcome = Some(outcome.clone());
        } else {
            self.state
                .workflow_summaries
                .push(super::types::BuddyWorkflowSummary {
                    workflow_id: report.workflow_id.clone(),
                    last_run: Some(now),
                    run_count: 1,
                    failure_category: Some(report.category.as_str().to_string()),
                    failure_summary: Some(summary.clone()),
                    last_outcome: Some(outcome),
                    ..Default::default()
                });
        }
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.presentation_state(),
        });

        let mut event = make_runtime_event(
            &format!("{}_failed", report.workflow_id),
            &title,
            "buddy",
            &format!(
                "workflow_failure:{}:{}",
                report.workflow_id,
                report.category.as_str()
            ),
            "failed",
            Some(report.category.priority()),
        );
        event.description = Some(crate::llm::safe_truncate(&detail, 600).to_string());
        event.failure_category = Some(report.category.as_str().to_string());
        event.failure_summary = Some(summary.clone());
        event.speech_text = Some(if summary.trim().is_empty() {
            format!(
                "{}: {}",
                report.category.title(),
                crate::llm::safe_truncate(&detail, 160)
            )
        } else {
            summary.clone()
        });
        event.scene = Some("alert".to_string());
        event.duration_hint = Some(12);
        event.persistent = true;
        event.bubble_policy = Some(super::types::BuddyBubblePolicy::Durable);
        event.chat_id = report.chat_id.clone();
        self.enqueue_runtime_event(event);

        self.workflow_failure_transcript_write(&report)
    }

    pub fn add_diagnostic(&mut self, mut ctx: super::diagnostics::DiagnosticContext) {
        ctx.error_message = redact_sensitive(&ctx.error_message);
        ctx.source_file = ctx
            .source_file
            .as_deref()
            .and_then(redact_diagnostic_metadata);
        ctx.tool_name = ctx
            .tool_name
            .as_deref()
            .and_then(redact_diagnostic_metadata);
        let signature = super::diagnostics::diagnostic_signature(&ctx);
        let duplicate = self
            .recent_diagnostics
            .iter()
            .rev()
            .any(|existing| super::diagnostics::diagnostic_signature(existing) == signature);
        if duplicate {
            self.enqueue_diagnostic_runtime_event(&ctx);
            return;
        }
        self.recent_diagnostics.push(ctx.clone());
        if self.recent_diagnostics.len() > 100 {
            self.recent_diagnostics.remove(0);
        }
        let project_root = self.project_root.clone();
        let ctx_for_disk = ctx.clone();
        let handle = tokio::spawn(async move {
            if let Err(err) = super::storage::append_diagnostic(&project_root, &ctx_for_disk).await
            {
                warn!("buddy: failed to persist diagnostic history: {}", err);
            }
        });
        self.track_background_task(handle);
        let _ = self.events_tx.send(BuddyEvent::DiagnosticAdded {
            diagnostic: ctx.clone(),
        });

        // Surface every diagnostic as a runtime event so it lands in the
        // "Recent Errors" panel and is persisted to runtime_queue.jsonl.
        // This catches frontend errors (POST /v1/buddy/diagnostics/collect)
        // and backend report_error paths (chrome / mcp tools) which would
        // otherwise be invisible to the panel.
        self.enqueue_diagnostic_runtime_event(&ctx);
    }

    fn enqueue_diagnostic_runtime_event(&mut self, ctx: &super::diagnostics::DiagnosticContext) {
        use super::diagnostics::DiagnosticSeverity;
        let priority = match ctx.severity {
            DiagnosticSeverity::Critical => "critical",
            DiagnosticSeverity::High => "high",
            DiagnosticSeverity::Medium => "normal",
            DiagnosticSeverity::Low => "low",
        };
        let redacted = redact_sensitive(&ctx.error_message);
        let truncated: String = redacted.chars().take(80).collect();
        let title = if ctx.error_type.is_empty() {
            truncated.clone()
        } else {
            format!("{}: {}", ctx.error_type, truncated)
        };
        let dedupe_key = format!("diag:{}", super::diagnostics::diagnostic_signature(ctx));
        let source = ctx
            .source_file
            .as_deref()
            .or(ctx.tool_name.as_deref())
            .unwrap_or("buddy");
        let mut ev = make_runtime_event(
            "error",
            &title,
            source,
            &dedupe_key,
            "failed",
            Some(priority),
        );
        ev.description = Some(redacted);
        ev.chat_id = ctx.chat_id.clone();
        self.enqueue_runtime_event(ev);
    }

    pub fn diagnostic_by_collected_at(
        &self,
        collected_at: &str,
    ) -> Option<super::diagnostics::DiagnosticContext> {
        self.recent_diagnostics
            .iter()
            .find(|diag| diag.collected_at == collected_at)
            .cloned()
    }

    pub fn diagnostic_by_id(&self, id: &str) -> Option<super::diagnostics::DiagnosticContext> {
        self.recent_diagnostics
            .iter()
            .find(|diag| super::diagnostics::diagnostic_id(diag) == id)
            .cloned()
    }

    pub fn record_issue_created(&mut self, error_message: String) {
        self.last_issue_at = Some(Instant::now());
        let now = chrono::Utc::now();
        self.recent_issue_errors.push((error_message, now));
        self.recent_issue_errors
            .retain(|(_, ts)| now.signed_duration_since(*ts).num_seconds() < 86400);
        if self.recent_issue_errors.len() > 200 {
            let excess = self.recent_issue_errors.len() - 200;
            self.recent_issue_errors.drain(0..excess);
        }
    }

    pub async fn append_workflow_transcript(
        &self,
        project_root: &std::path::Path,
        workflow_id: &str,
        output_summary: &str,
        success: bool,
    ) {
        Self::append_workflow_transcript_to_path(
            project_root,
            workflow_id,
            output_summary,
            success,
        )
        .await;
    }

    pub async fn append_workflow_transcript_to_path(
        project_root: &std::path::Path,
        workflow_id: &str,
        output_summary: &str,
        success: bool,
    ) {
        if !validate_workflow_id(workflow_id) {
            warn!("buddy: rejecting invalid workflow_id: {:?}", workflow_id);
            return;
        }
        let path = project_root.join(format!(
            ".refact/buddy/chats/workflows/{}.json",
            workflow_id
        ));
        if let Some(parent) = path.parent() {
            if let Err(error) = tokio::fs::create_dir_all(parent).await {
                warn!(
                    "buddy: failed to create workflow transcript dir {:?}: {}",
                    parent, error
                );
                return;
            }
        }
        super::workflows::append_workflow_entry(&path, output_summary, success).await;
    }

    pub async fn append_workflow_failure_transcript(
        path: &std::path::Path,
        report: &super::workflows::WorkflowFailureReport,
    ) {
        if let Some(parent) = path.parent() {
            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                warn!(
                    "buddy: failed to create workflow transcript dir {:?}: {}",
                    parent, err
                );
                return;
            }
        }
        super::workflows::append_workflow_entry_with_failure(
            path,
            &report.detail,
            false,
            Some((&report.category, report.summary.as_str())),
        )
        .await;
    }

    fn workflow_failure_transcript_write(
        &self,
        report: &super::workflows::WorkflowFailureReport,
    ) -> Option<(std::path::PathBuf, super::workflows::WorkflowFailureReport)> {
        if !validate_workflow_id(&report.workflow_id) {
            warn!(
                "buddy: rejecting invalid workflow_id: {:?}",
                report.workflow_id
            );
            return None;
        }
        let path = self.project_root.join(format!(
            ".refact/buddy/chats/workflows/{}.json",
            report.workflow_id
        ));
        Some((path, report.clone()))
    }

    pub fn report_error(
        &mut self,
        error_type: &str,
        error_msg: &str,
        source: Option<&str>,
        chat_id: Option<&str>,
    ) {
        self.report_error_with_model(error_type, error_msg, source, chat_id, None);
    }

    pub fn report_error_with_model(
        &mut self,
        error_type: &str,
        error_msg: &str,
        source: Option<&str>,
        chat_id: Option<&str>,
        model_id: Option<&str>,
    ) {
        let severity = refact_buddy_core::diagnostics::classify_diagnostic_severity(error_msg);
        let ctx = super::diagnostics::DiagnosticContext {
            error_type: error_type.to_string(),
            error_message: error_msg.to_string(),
            source_file: source.map(|s| s.to_string()),
            tool_name: None,
            chat_id: chat_id.map(|s| s.to_string()),
            model_id: model_id.map(|s| s.to_string()),
            collected_at: Utc::now().to_rfc3339(),
            severity,
            occurrences: None,
        };
        self.add_diagnostic(ctx);
        let redacted = redact_sensitive(error_msg);
        let truncated: String = redacted.chars().take(80).collect();
        self.add_activity(BuddyActivity {
            icon: "⚠️".to_string(),
            title: format!("{}: {}", error_type, truncated),
            description: redacted,
            timestamp: Utc::now().to_rfc3339(),
            activity_type: "error".to_string(),
            chat_id: chat_id.map(|s| s.to_string()),
            failure_category: None,
            failure_summary: None,
        });
        self.dirty = true;
    }

    pub fn expire_suggestions(&mut self) {
        if self.operator_features.storage {
            return;
        }
        let now = chrono::Utc::now();
        let mut changed = false;
        for s in self.state.suggestion_state.iter_mut() {
            if s.dismissed {
                continue;
            }
            let expiry_secs = if s.quest.is_some() {
                QUEST_SUGGESTION_EXPIRY_SECS
            } else {
                SUGGESTION_EXPIRY_SECS
            };
            if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&s.created_at) {
                let age = now.signed_duration_since(created).num_seconds();
                if age > expiry_secs {
                    s.dismissed = true;
                    changed = true;
                }
            }
        }
        let before = self.state.suggestion_state.len();
        self.state.suggestion_state.retain(|s| {
            if !s.dismissed {
                return true;
            }
            let retention_secs = if s.quest.is_some() {
                QUEST_SUGGESTION_EXPIRY_SECS + 3600
            } else {
                3600
            };
            if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&s.created_at) {
                now.signed_duration_since(created).num_seconds() < retention_secs
            } else {
                false
            }
        });
        if changed || self.state.suggestion_state.len() != before {
            self.dirty = true;
            let _ = self.events_tx.send(BuddyEvent::StateUpdated {
                state: self.presentation_state(),
            });
        }
    }
}

fn operator_suggestion(suggestion: &BuddySuggestion) -> BuddyOpportunity {
    let created_at = DateTime::parse_from_rfc3339(&suggestion.created_at)
        .map(|time| time.with_timezone(&Utc))
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    BuddyOpportunity {
        id: format!("suggestion:{}", suggestion.id),
        kind: super::types::BuddyOpportunityKind::JobFinding,
        summary: format!("{}\n\n{}", suggestion.title, suggestion.description),
        priority: super::types::BuddyPriority::Normal,
        confidence: 0.5,
        fact_keys: vec![],
        cooldown_key: format!("suggestion:{}", suggestion.id),
        cooldown_secs: SUGGESTION_RATE_LIMIT_SECS,
        status: if suggestion.dismissed {
            OpportunityStatus::Dismissed
        } else {
            OpportunityStatus::New
        },
        // Old controls and quests are reference material, never new action authority.
        proposed_actions: vec![],
        humor: None,
        humor_allowed: false,
        related: Default::default(),
        created_at,
        expires_at: DateTime::<Utc>::MAX_UTC,
        resolved_at: None,
    }
}

/// Copy producer inputs under the actor lock, commit on the store worker, then
/// acknowledge only those exact inputs. Failed batches remain available for retry.
pub(crate) async fn publish_operator_inputs(
    buddy: &Arc<tokio::sync::Mutex<Option<BuddyService>>>,
) -> Result<(), String> {
    let (runtime, project_root, opportunities, drafts, seed_cursor) = {
        let mut guard = buddy.lock().await;
        let Some(svc) = guard.as_mut() else {
            return Ok(());
        };
        if !svc.operator_features.storage {
            return Ok(());
        }
        let runtime = svc
            .outcome_runtime
            .clone()
            .ok_or("Buddy operator store unavailable")?;
        if runtime.project().canonical_root != svc.project_root.to_string_lossy() {
            return Err("Buddy operator project identity changed".into());
        }
        let mut drafts: Vec<_> = svc
            .draft_store
            .snapshot()
            .into_iter()
            .filter(|draft| !svc.persisted_operator_drafts.contains(&draft.id))
            .collect();
        drafts.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        // Finish proposal content first, even across batches. Otherwise an editor
        // link could import a placeholder before its real draft is persisted.
        let opportunities = if drafts.is_empty() && !svc.operator_legacy_seeded {
            // A long historical backlog is paginated, never refused: the queue
            // is frozen once storage owns the lifecycle, so this deterministic
            // order lets a later pass resume exactly where this one stopped.
            let mut inputs = svc.opportunity_queue.snapshot();
            inputs.extend(svc.state.suggestion_state.iter().map(operator_suggestion));
            inputs.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
            inputs.split_off(svc.operator_legacy_seed_cursor.min(inputs.len()))
        } else {
            Vec::new()
        };
        (
            runtime,
            svc.project_root.clone(),
            opportunities,
            drafts,
            svc.operator_legacy_seed_cursor,
        )
    };
    let worker = runtime.worker();
    let pending_opportunities = opportunities.len();
    let committed_drafts = drafts.clone();
    let (chunk, migration_error) = runtime
        .with_store(move |store| {
            Ok(super::legacy_work::migrate_chunk_partial(
                store,
                &worker,
                &opportunities,
                &drafts,
            ))
        })
        .await
        .map_err(|error| error.to_string())?;
    let committed_drafts: Vec<_> = committed_drafts
        .into_iter()
        .take(chunk.consumed_drafts)
        .collect();
    let consumed_opportunities = chunk.consumed_opportunities;
    let seeded_complete = pending_opportunities > 0 && chunk.remaining == 0;
    // Independent history errors cannot erase the committed work prefix.
    let history = publish_operator_history(buddy, &runtime, &project_root).await;
    // Publication is separate from projection refresh: a read failure must not
    // lose committed inputs, nor claim that uncommitted proposals were saved.
    let projection = runtime
        .with_store(|store| super::legacy_work::projection_page(store, None, 200))
        .await;
    let mut guard = buddy.lock().await;
    let Some(svc) = guard.as_mut() else {
        return Ok(());
    };
    require_publication_owner(svc, &runtime)?;
    // Concurrent publishers must not double-advance the same captured prefix.
    if svc.operator_legacy_seed_cursor != seed_cursor {
        return Err("Buddy operator migration cursor changed during publication".into());
    }
    svc.operator_legacy_seeded |= seeded_complete;
    // Only advance the cursor by what actually committed, so an interrupted
    // migration resumes rather than skipping or re-admitting historical cards.
    svc.operator_legacy_seed_cursor = svc
        .operator_legacy_seed_cursor
        .saturating_add(consumed_opportunities);
    for draft in committed_drafts {
        if svc.persisted_operator_drafts.insert(draft.id.clone()) {
            let _ = svc.events_tx.send(BuddyEvent::DraftCreated {
                draft,
            });
        }
    }
    if let Some(error) = migration_error {
        return Err(error.to_string());
    }
    history?;
    let (projection, _) = projection.map_err(|error| error.to_string())?;
    for opportunity in &projection {
        if !svc
            .operator_opportunity_projection
            .iter()
            .any(|old| old.id == opportunity.id)
        {
            let _ = svc.events_tx.send(BuddyEvent::OpportunityProduced {
                opportunity: opportunity.clone(),
            });
        }
    }
    svc.operator_opportunity_projection = projection;
    Ok(())
}

/// Seed immutable source snapshots in bounded pages. A changing activity feed
/// cannot reset the cursor and starve its tail; restart replays idempotently.
async fn publish_operator_history(
    buddy: &Arc<tokio::sync::Mutex<Option<BuddyService>>>,
    runtime: &super::outcome_runtime::OutcomeRuntime,
    project_root: &Path,
) -> Result<(), String> {
    let need_receipts = {
        let guard = buddy.lock().await;
        let svc = guard.as_ref().ok_or("Buddy operator unavailable")?;
        if svc.operator_legacy_history_seeded {
            return Ok(());
        }
        svc.operator_receipt_seed.needs_source()
    };
    let receipts = if need_receipts {
        super::legacy_work::load_receipt_history(project_root).await
    } else {
        Ok(Vec::new())
    };
    let load_error = receipts.as_ref().err().cloned();
    let (receipt_page, activity_page, receipt_start, activity_start) = {
        let mut guard = buddy.lock().await;
        let svc = guard.as_mut().ok_or("Buddy operator unavailable")?;
        require_publication_owner(svc, runtime)?;
        if let Ok(receipts) = receipts {
            svc.operator_receipt_seed.stage(receipts);
        }
        if svc.operator_activity_seed.needs_source() {
            let mut activities = svc.state.recent_activities.clone();
            activities.sort_by_key(|activity| serde_json::to_string(activity).unwrap_or_default());
            svc.operator_activity_seed.stage(activities);
        }
        (
            svc.operator_receipt_seed
                .page(super::legacy_work::MAX_MIGRATION_BATCH / 2),
            svc.operator_activity_seed
                .page(super::legacy_work::MAX_MIGRATION_BATCH / 2),
            svc.operator_receipt_seed.cursor,
            svc.operator_activity_seed.cursor,
        )
    };
    let (receipts_committed, activities_committed, import_error) = runtime
        .with_store(move |store| {
            Ok(super::legacy_work::migrate_history_chunk(
                store,
                &receipt_page,
                &activity_page,
            ))
        })
        .await
        .map_err(|error| error.to_string())?;
    let mut guard = buddy.lock().await;
    let svc = guard.as_mut().ok_or("Buddy operator unavailable")?;
    require_publication_owner(svc, runtime)?;
    svc.operator_receipt_seed
        .acknowledge(receipt_start, receipts_committed);
    svc.operator_activity_seed
        .acknowledge(activity_start, activities_committed);
    svc.operator_legacy_history_seeded =
        svc.operator_receipt_seed.complete() && svc.operator_activity_seed.complete();
    match load_error.or(import_error) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn require_publication_owner(
    svc: &BuddyService,
    runtime: &super::outcome_runtime::OutcomeRuntime,
) -> Result<(), String> {
    if svc.operator_features.storage
        && svc.outcome_runtime.as_ref().is_some_and(|current| {
            current.same_owner(runtime)
                && !current.is_closed()
                && current.project() == runtime.project()
                && current.worker() == runtime.worker()
                && current.project().canonical_root == svc.project_root.to_string_lossy()
        })
    {
        Ok(())
    } else {
        Err("Buddy operator owner changed during publication".into())
    }
}

pub(crate) async fn reopen_operator_now(
    buddy: &Arc<tokio::sync::Mutex<Option<BuddyService>>>,
) -> bool {
    let Some(project_root) = ({
        let mut guard = buddy.lock().await;
        guard
            .as_mut()
            .and_then(|service| service.manual_operator_reopen_root(tokio::time::Instant::now()))
    }) else {
        return buddy
            .lock()
            .await
            .as_ref()
            .is_some_and(|service| service.outcome_runtime.is_some());
    };
    let result = super::operator_runtime::open_project_runtime_async(project_root.clone()).await;
    match result {
        Ok(runtime) => {
            let installed = {
                let mut guard = buddy.lock().await;
                guard.as_mut().is_some_and(|service| {
                    service.install_reopened_operator_runtime(&project_root, runtime.clone())
                })
            };
            if !installed {
                runtime.shutdown().await;
            }
            installed
        }
        Err(error) => {
            let mut guard = buddy.lock().await;
            if let Some(service) = guard.as_mut() {
                service.record_manual_operator_open_failure(error, tokio::time::Instant::now());
            }
            false
        }
    }
}

pub fn make_runtime_event(
    signal_type: &str,
    title: &str,
    source: &str,
    dedupe_key: &str,
    status: &str,
    priority: Option<&str>,
) -> BuddyRuntimeEvent {
    BuddyRuntimeEvent {
        id: Uuid::new_v4().to_string(),
        signal_type: signal_type.to_string(),
        title: title.to_string(),
        description: None,
        source: source.to_string(),
        status: status.to_string(),
        failure_category: None,
        failure_summary: None,
        progress: None,
        dedupe_key: Some(dedupe_key.to_string()),
        priority: priority.unwrap_or("normal").to_string(),
        created_at: Utc::now().to_rfc3339(),
        ttl_ms: None,
        bubble_policy: None,
        speech_text: None,
        scene: None,
        duration_hint: None,
        persistent: false,
        controls: Vec::new(),
        chat_id: None,
        dismissed: false,
    }
}

async fn tick_operator_reopen(buddy: &Arc<tokio::sync::Mutex<Option<BuddyService>>>) {
    let completed = {
        let mut guard = buddy.lock().await;
        let Some(service) = guard.as_mut() else {
            return;
        };
        if service
            .operator_reopen_task
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
        {
            service.operator_reopen_task.take()
        } else {
            service.begin_operator_reopen(tokio::time::Instant::now());
            None
        }
    };
    let Some(task) = completed else {
        return;
    };
    let result = task.await.unwrap_or_else(|_| {
        Err(super::operator_runtime::OperatorRuntimeOpenError::Task(
            "operator store reopen task failed".to_string(),
        ))
    });
    let project_root = {
        let guard = buddy.lock().await;
        guard.as_ref().map(|service| service.project_root.clone())
    };
    let Some(project_root) = project_root else {
        if let Ok(runtime) = result {
            runtime.shutdown().await;
        }
        return;
    };
    match result {
        Ok(runtime) => {
            let installed = {
                let mut guard = buddy.lock().await;
                guard.as_mut().is_some_and(|service| {
                    service.install_reopened_operator_runtime(&project_root, runtime.clone())
                })
            };
            if installed {
                info!("buddy: operator store reopened after contention");
            } else {
                runtime.shutdown().await;
            }
        }
        Err(error) => {
            let mut guard = buddy.lock().await;
            if let Some(service) = guard.as_mut() {
                service.record_operator_open_failure(error, tokio::time::Instant::now());
            }
        }
    }
}

pub async fn buddy_complete_event(gcx: AppState, dedupe_key: &str, status: &str) {
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.complete_runtime_event(dedupe_key, status);
    }
}

pub async fn buddy_enqueue_event(gcx: AppState, event: BuddyRuntimeEvent) {
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.enqueue_runtime_event(event);
    }
}

pub async fn resolve_buddy_chat_model(gcx: AppState) -> String {
    let caps_state = gcx.model.caps.read().await;
    let Some(caps) = caps_state.caps.as_ref() else {
        return String::new();
    };
    let buddy = caps.defaults.chat_buddy_model.trim();
    if !buddy.is_empty() && crate::caps::resolve_model(&caps.chat_models, buddy).is_ok() {
        buddy.to_string()
    } else {
        String::new()
    }
}

pub async fn buddy_snapshot(gcx: AppState) -> Option<BuddySnapshot> {
    let buddy_arc = gcx.buddy.buddy.clone();
    let lock = buddy_arc.lock().await;
    lock.as_ref().map(|svc| svc.snapshot())
}

pub async fn buddy_record_workflow_telemetry(
    gcx: AppState,
    workflow_id: &str,
    llm_calls: u64,
    tokens_in: u64,
    tokens_out: u64,
    produced_output: bool,
) {
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.record_workflow_telemetry(
            workflow_id,
            llm_calls,
            tokens_in,
            tokens_out,
            produced_output,
        );
    }
}

pub async fn buddy_update_speech(gcx: AppState, speech: BuddySpeechItem) {
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.update_speech(speech);
    }
}

pub async fn buddy_llm_budget_exhausted(gcx: &AppState) -> bool {
    let buddy_arc = gcx.buddy.buddy.clone();
    let lock = buddy_arc.lock().await;
    lock.as_ref()
        .map(|svc| svc.llm_budget_exhausted())
        .unwrap_or(false)
}

pub async fn buddy_update_speech_user(gcx: AppState, speech: BuddySpeechItem) {
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.update_speech_user_initiated(speech);
    }
}

pub struct CompletedQuestVoice {
    pub mutation: BuddyMutation,
    pub speech: BuddySpeechItem,
}

pub async fn complete_quest_with_voice(
    gcx: AppState,
    quest: BuddyQuest,
    persona: BuddyPersonalityProfile,
    identity_name: String,
    pulse: BuddyPulse,
) -> CompletedQuestVoice {
    let title = quest.title.clone();
    let fallback_speech = format!("Quest complete: {title}! Tiny victory dance?");
    let fallback_event = format!("Quest complete: {title}");
    let mut speech = render_buddy_speech(
        gcx.clone(),
        persona.clone(),
        identity_name.clone(),
        pulse.clone(),
        Some(quest.quest_type.clone()),
        fallback_speech.clone(),
        SpeechIntent::QuestComplete,
        fallback_speech,
    )
    .await;
    speech.id = format!("quest-complete-{}", quest.id);
    speech.ttl_seconds = 12;
    speech.dedupe_key = Some(format!("quest_complete_{}", quest.quest_type));
    speech.controls = vec![];

    let (event_title, event_speech) = render_buddy_runtime_event(
        gcx,
        persona,
        identity_name.clone(),
        pulse,
        Some(quest.quest_type.clone()),
        fallback_event.clone(),
        "completed",
        fallback_event.clone(),
        Some(fallback_event.clone()),
    )
    .await;

    CompletedQuestVoice {
        mutation: BuddyMutation {
            activity: Some(BuddyActivity {
                icon: quest.icon.clone(),
                title: event_title.clone(),
                description: format!(
                    "{} wrapped up '{title}' and earned a growth boost.",
                    identity_name
                ),
                timestamp: Utc::now().to_rfc3339(),
                activity_type: "quest_completed".to_string(),
                chat_id: None,
                failure_category: None,
                failure_summary: None,
            }),
            runtime_event: Some(BuddyRuntimeEvent {
                speech_text: event_speech,
                scene: Some("celebrate".to_string()),
                duration_hint: Some(10),
                persistent: false,
                controls: vec![],
                chat_id: None,
                ..make_runtime_event(
                    "task_completed",
                    &event_title,
                    "buddy_quest",
                    &format!("quest_complete_{}", quest.quest_type),
                    "completed",
                    Some("high"),
                )
            }),
            ..Default::default()
        },
        speech,
    }
}

pub async fn report_error_persisted(
    gcx: AppState,
    error_type: &str,
    error_msg: &str,
    source: Option<&str>,
    chat_id: Option<&str>,
) {
    report_error_persisted_with_model(gcx, error_type, error_msg, source, chat_id, None).await;
}

pub async fn report_error_persisted_with_model(
    gcx: AppState,
    error_type: &str,
    error_msg: &str,
    source: Option<&str>,
    chat_id: Option<&str>,
    model_id: Option<&str>,
) {
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.report_error_with_model(error_type, error_msg, source, chat_id, model_id);
    }
}

pub async fn latest_project_root(gcx: AppState) -> Result<std::path::PathBuf, String> {
    crate::files_correction::get_project_dirs(gcx.gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "no project root".to_string())
}

pub async fn load_diagnostics_for_service(
    project_root: &Path,
) -> Vec<super::diagnostics::DiagnosticContext> {
    match super::storage::load_recent_diagnostics(project_root, 100).await {
        Ok(diags) => diags,
        Err(err) => {
            warn!("buddy: failed to load diagnostic history: {}", err);
            Vec::new()
        }
    }
}

pub async fn resolve_diagnostic(
    gcx: AppState,
    diagnostic_index: Option<usize>,
    diagnostic_id: Option<&str>,
    collected_at: Option<&str>,
    fallback: Option<super::diagnostics::DiagnosticContext>,
) -> Result<super::diagnostics::DiagnosticContext, String> {
    let project_root = latest_project_root(gcx.clone()).await?;
    let buddy_arc = gcx.buddy.buddy.clone();
    let lock = buddy_arc.lock().await;
    let svc = lock
        .as_ref()
        .ok_or_else(|| "buddy service not initialized".to_string())?;
    let by_id = diagnostic_id.and_then(|id| svc.diagnostic_by_id(id));
    let by_time = collected_at.and_then(|ts| svc.diagnostic_by_collected_at(ts));
    let recent = svc.recent_diagnostics.clone();
    drop(lock);

    if let Some(id) = diagnostic_id {
        if let Some(ctx) = by_id {
            return Ok(ctx);
        }
        let diags = super::storage::load_diagnostics(&project_root).await?;
        if let Some(ctx) = diags
            .into_iter()
            .find(|diag| super::diagnostics::diagnostic_id(diag) == id)
        {
            return Ok(ctx);
        }
        return Err("diagnostic id not found".to_string());
    }

    if let Some(ts) = collected_at {
        if let Some(ctx) = by_time {
            return Ok(ctx);
        }
        let diags = super::storage::load_diagnostics(&project_root).await?;
        if let Some(ctx) = diags.into_iter().find(|diag| diag.collected_at == ts) {
            return Ok(ctx);
        }
        return Err("diagnostic timestamp not found".to_string());
    }

    if let Some(idx) = diagnostic_index {
        let diags = if recent.is_empty() {
            super::storage::load_recent_diagnostics(&project_root, 100).await?
        } else {
            recent
        };
        return diags
            .get(idx)
            .cloned()
            .ok_or_else(|| "diagnostic index out of range".to_string());
    }

    fallback.ok_or_else(|| "provide diagnostic reference or error".to_string())
}

pub fn same_day_log_filter(line: &str, collected_at: &str) -> bool {
    let Some(prefix) = line.get(0..6) else {
        return false;
    };
    let Some(target) = chrono::DateTime::parse_from_rfc3339(collected_at)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
    else {
        return true;
    };
    let Ok(time) = chrono::NaiveTime::parse_from_str(prefix, "%H%M%S") else {
        return false;
    };
    let candidate = target.date_naive().and_time(time).and_utc();
    let diff = target.signed_duration_since(candidate).num_seconds();
    diff >= 0 && diff <= 24 * 3600
}

pub struct BuddyMutation {
    pub runtime_event: Option<BuddyRuntimeEvent>,
    pub xp: u64,
    pub activity: Option<super::types::BuddyActivity>,
    pub mood: Option<String>,
}

impl Default for BuddyMutation {
    fn default() -> Self {
        Self {
            runtime_event: None,
            xp: 0,
            activity: None,
            mood: None,
        }
    }
}

pub async fn buddy_apply(gcx: AppState, m: BuddyMutation) {
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let Some(svc) = lock.as_mut() else { return };
    if let Some(ev) = m.runtime_event {
        svc.enqueue_runtime_event(ev);
    }
    if m.xp > 0 {
        svc.grant_xp_from(m.xp, super::state::XpSource::ActivityVolume);
    }
    if let Some(activity) = m.activity {
        svc.add_activity(activity);
    }
    if let Some(mood) = m.mood {
        svc.state.semantic.mood = mood;
        svc.dirty = true;
        let _ = svc.events_tx.send(BuddyEvent::StateUpdated {
            state: svc.presentation_state(),
        });
    }
}

async fn check_operator_store_growth(buddy: &Arc<tokio::sync::Mutex<Option<BuddyService>>>) {
    let runtime = buddy
        .lock()
        .await
        .as_ref()
        .filter(|service| service.operator_features.storage)
        .and_then(|service| service.outcome_runtime.clone());
    let Some(runtime) = runtime else { return };
    let warnings = match runtime
        .with_store(|store| {
            store
                .diagnostic_summary_with_growth_warnings()
                .map(|(_, warnings)| warnings)
        })
        .await
    {
        Ok(warnings) => warnings,
        Err(error) => {
            warn!(%error, "buddy: operator store growth check failed");
            return;
        }
    };
    if warnings.is_empty() {
        return;
    }
    let mut guard = buddy.lock().await;
    let Some(service) = guard.as_mut() else {
        return;
    };
    if !service
        .outcome_runtime
        .as_ref()
        .is_some_and(|current| current.same_owner(&runtime))
    {
        return;
    }
    publish_operator_store_growth_warnings(service, warnings);
}

pub(crate) fn operator_store_growth_check_due(tick: u64) -> bool {
    tick != 0 && tick % 3600 == 0
}

pub(crate) fn publish_operator_store_growth_warnings(
    service: &mut BuddyService,
    warnings: Vec<String>,
) {
    for warning in warnings {
        let collected_at = Utc::now().to_rfc3339();
        warn!("{warning}");
        service.add_diagnostic(super::diagnostics::DiagnosticContext {
            error_type: "operator_store_growth".into(),
            error_message: format!("{warning} (observed {collected_at})"),
            source_file: Some("buddy/operator_store.rs".into()),
            tool_name: None,
            chat_id: None,
            model_id: None,
            collected_at,
            severity: super::diagnostics::DiagnosticSeverity::Medium,
            occurrences: None,
        });
    }
}

struct BuddyBackgroundContext {
    project_root: PathBuf,
    buddy_arc: Arc<tokio::sync::Mutex<Option<BuddyService>>>,
    scheduler: super::scheduler::BuddyScheduler,
    shutdown_flag: Arc<std::sync::atomic::AtomicBool>,
    writer_handle: JoinHandle<()>,
    commit_poller_handle: JoinHandle<()>,
}

async fn initialize_buddy_background(gcx: AppState) -> Option<BuddyBackgroundContext> {
    let project_root = loop {
        if gcx.runtime.shutdown_flag.load(Ordering::SeqCst) {
            return None;
        }
        let dirs = crate::files_correction::get_project_dirs(gcx.gcx.clone()).await;
        if let Some(root) = dirs.into_iter().next() {
            break root;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    };

    if let Err(e) = super::storage::bootstrap_buddy_storage(&project_root).await {
        warn!("buddy: failed to bootstrap storage: {}", e);
        return None;
    }
    super::storage::gc_stale_memory_ops_tmp_files(&project_root).await;
    match super::storage::archive_memory_ops_if_oversized(
        &project_root,
        MEMORY_OPS_ARCHIVE_THRESHOLD_BYTES,
    )
    .await
    {
        Ok(true) => info!("buddy: archived oversized memory ops queue"),
        Ok(false) => {}
        Err(err) => warn!(
            "buddy: failed to archive oversized memory ops queue: {}",
            err
        ),
    }

    let state = super::state::load_state(&project_root).await;
    let settings = super::settings::load_settings(&project_root).await;
    let recent_diagnostics = load_diagnostics_for_service(&project_root).await;
    let mut memory_ops = super::storage::load_memory_ops_repairing(&project_root).await;
    let before_memory_ops = memory_ops.pending_count + memory_ops.approved_count;
    if before_memory_ops > 5000 {
        match super::storage::compact_memory_ops(&project_root).await {
            Ok(compacted) => {
                let after_memory_ops = compacted.pending_count + compacted.approved_count;
                info!(
                    "buddy: compacted large memory ops queue at startup: {} -> {} pending/approved ops",
                    before_memory_ops, after_memory_ops
                );
                memory_ops = compacted;
            }
            Err(err) => warn!(
                "buddy: failed to compact large memory ops queue at startup: {}",
                err
            ),
        }
    }
    let runtime_queue = super::storage::load_runtime_queue(&project_root).await;

    let events_tx = gcx.buddy.buddy_events_tx.clone();
    let (queue_tx, queue_rx) = mpsc::unbounded_channel::<RuntimeQueueWriteOp>();
    let writer_root = project_root.clone();
    let writer_handle = tokio::spawn(async move {
        run_runtime_queue_writer(writer_root, queue_rx).await;
    });

    let mut service = BuddyService::new(
        project_root.clone(),
        state,
        settings,
        recent_diagnostics,
        runtime_queue,
        events_tx,
        Some(queue_tx),
    );
    service.enable_operator_from_settings();
    service.set_memory_ops(memory_ops);
    service
        .draft_store
        .restore(super::storage::load_drafts(&project_root).await);

    let buddy_arc = gcx.buddy.buddy.clone();
    *buddy_arc.lock().await = Some(service);
    let initial_pulse =
        super::pulse::build_pulse(gcx.clone(), &project_root, &FactStore::new()).await;
    {
        let mut buddy = buddy_arc.lock().await;
        if let Some(svc) = buddy.as_mut() {
            svc.set_pulse(initial_pulse);
            if svc
                .state
                .chat_phrase_bank
                .as_ref()
                .is_some_and(|bank| !chat_phrase_bank_is_fresh(bank, Utc::now()))
            {
                svc.state.chat_phrase_bank = None;
                svc.dirty = true;
            }
        }
    }

    info!("buddy: service started for {:?}", project_root);

    let scheduler = super::scheduler::BuddyScheduler::new();
    let shutdown_flag = gcx.runtime.shutdown_flag.clone();
    let commit_poller_handle = tokio::spawn(commit_activity_poller(
        gcx,
        project_root.clone(),
        shutdown_flag.clone(),
    ));
    Some(BuddyBackgroundContext {
        project_root,
        buddy_arc,
        scheduler,
        shutdown_flag,
        writer_handle,
        commit_poller_handle,
    })
}

async fn handle_startup_suggestion(
    project_root: &Path,
    buddy_arc: &Arc<tokio::sync::Mutex<Option<BuddyService>>>,
) {
    let agents_md = project_root.join("AGENTS.md");
    let setup_done = tokio::fs::try_exists(&agents_md).await.unwrap_or(false);
    if !setup_done {
        let mut guard = buddy_arc.lock().await;
        if let Some(svc) = guard.as_mut() {
            let already = svc
                .state
                .suggestion_state
                .iter()
                .any(|s| s.suggestion_type == "setup");
            if !already {
                let suggestion = BuddySuggestion {
                    id: "setup".to_string(),
                    suggestion_type: "setup".to_string(),
                    title: "Set up this project".to_string(),
                    description:
                        "Run setup to generate guidelines, integrations, and toolbox commands."
                            .to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    dismissed: false,
                    controls: vec![],
                    quest: None,
                };
                let admission = svc.prepare_suggestion(suggestion);
                drop(guard);
                match admission {
                    Ok(admission) => {
                        if let Err(error) = admission.commit(buddy_arc).await {
                            tracing::warn!(%error, "operator suggestion admission failed");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "operator suggestion admission failed")
                    }
                }
            }
        }
    }
}

async fn handle_operator_maintenance(
    gcx: AppState,
    context: &BuddyBackgroundContext,
    expiry_tick: u64,
) {
    tick_operator_reopen(&context.buddy_arc).await;
    if operator_store_growth_check_due(expiry_tick) && !context.shutdown_flag.load(Ordering::SeqCst)
    {
        check_operator_store_growth(&context.buddy_arc).await;
    }
    if let Err(error) = publish_operator_inputs(&context.buddy_arc).await {
        if expiry_tick % 60 == 0 {
            warn!("buddy: operator proposals remain pending: {}", error);
        }
    }
    if expiry_tick % 60 == 0 {
        let runtime = {
            let buddy = context.buddy_arc.lock().await;
            buddy
                .as_ref()
                .filter(|svc| svc.operator_features.storage && svc.operator_features.supervision)
                .and_then(|svc| svc.outcome_runtime.clone())
        };
        if let Some(runtime) = runtime {
            if let Err(error) = super::supervision::reconcile_active(&gcx, &runtime).await {
                warn!("buddy: supervision reconciliation unavailable: {}", error);
            }
        }
    }
}

async fn handle_expire_suggestions(context: &BuddyBackgroundContext, expiry_tick: u64) {
    if expiry_tick % 60 == 0 {
        let mut buddy = context.buddy_arc.lock().await;
        if let Some(svc) = buddy.as_mut() {
            svc.expire_suggestions();
        }
    }
}

async fn handle_refresh_pulse(gcx: AppState, context: &BuddyBackgroundContext, expiry_tick: u64) {
    // Pulse refresh + opportunity expiry every 60s
    if expiry_tick % 60 == 0 {
        let now = Utc::now();
        let fact_snap = {
            let buddy = context.buddy_arc.lock().await;
            buddy
                .as_ref()
                .map(|svc| svc.fact_store.iter().cloned().collect::<Vec<_>>())
        };
        if let Some(facts) = fact_snap {
            let mut tmp_store = FactStore::new();
            for f in facts {
                tmp_store.ingest(f);
            }
            let knowledge_dirs = crate::files_correction::get_project_dirs(gcx.gcx.clone())
                .await
                .into_iter()
                .map(|dir| dir.join(crate::file_filter::KNOWLEDGE_FOLDER_NAME))
                .filter(|dir| dir.exists())
                .collect::<Vec<_>>();
            let lifecycle_ops =
                detect_memory_lifecycle_ops_from_knowledge_dirs(&knowledge_dirs, now).await;
            if !lifecycle_ops.is_empty() {
                let mut memory_ops = super::storage::load_memory_ops(&context.project_root).await;
                for op in lifecycle_ops {
                    match super::storage::enqueue_memory_op(&context.project_root, op).await {
                        Ok(updated) => memory_ops = updated,
                        Err(err) => {
                            warn!("buddy: failed to enqueue memory lifecycle op: {}", err)
                        }
                    }
                }
                if let Err(error) =
                    BuddyService::set_memory_ops_durable(&context.buddy_arc, memory_ops).await
                {
                    tracing::warn!(%error, "operator memory admission failed");
                }
            }
            let new_pulse =
                super::pulse::build_pulse(gcx.clone(), &context.project_root, &tmp_store).await;
            let mut buddy = context.buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                svc.set_pulse(new_pulse);
                svc.expire_opportunities();
                svc.opportunity_queue.refresh_cooldowns(now);
                svc.expire_drafts(now);
                svc.expire_runtime_events_at(now);
            }
        }
    }
}

async fn handle_refresh_quiet_window(gcx: AppState, context: &BuddyBackgroundContext) {
    {
        let window = {
            let actions = {
                let ring = gcx.buddy.user_activity.lock().await;
                ring.snapshot()
            };
            super::speech_policy::auto_quiet_window_from_actions(&actions)
        };
        let mut buddy = context.buddy_arc.lock().await;
        if let Some(svc) = buddy.as_mut() {
            svc.auto_quiet_window = window;
        }
    }
}

async fn handle_run_observers(gcx: AppState, context: &BuddyBackgroundContext) {
    // Observer ticking — each observer respects its own cadence
    {
        let now = Utc::now();
        let due_observers = {
            let buddy = context.buddy_arc.lock().await;
            match buddy.as_ref() {
                Some(svc) => {
                    let s = svc.settings.clone();
                    let lt = svc.last_observer_tick.clone();
                    let due: Vec<Arc<dyn BuddyObserver>> = svc
                        .observers
                        .iter()
                        .filter(|obs| {
                            if !obs.requires_setting(&s) {
                                return false;
                            }
                            match lt.get(obs.id()) {
                                Some(t) => (now - *t).num_seconds() as u64 >= obs.cadence_seconds(),
                                None => true,
                            }
                        })
                        .cloned()
                        .collect();
                    due
                }
                None => vec![],
            }
        };
        if !due_observers.is_empty() {
            // Run observers without holding buddy lock (prevents deadlock with
            // DiagnosticClusterObserver which also locks buddy).
            let all_facts = observe_buddy_facts_parallel(
                due_observers.clone(),
                gcx.clone(),
                context.project_root.clone(),
                now,
            )
            .await;
            // Phase 1: ingest facts, detect candidates, add non-humor opps — all under buddy lock.
            let source_owner = context
                .buddy_arc
                .lock()
                .await
                .as_ref()
                .map(BuddyService::admission_identity);
            let mut native_ready = Vec::new();
            let (humor_tasks, pulse_for_humor, humor_arc) = {
                let mut buddy = context.buddy_arc.lock().await;
                if let Some(svc) = buddy.as_mut() {
                    for obs in &due_observers {
                        svc.last_observer_tick.insert(obs.id(), now);
                    }
                    for (refresh_ttl_secs, facts) in all_facts {
                        svc.fact_store.ingest_many_with_refresh_ttl(
                            facts,
                            chrono::Duration::seconds(refresh_ttl_secs as i64),
                        );
                    }
                    let candidates = OpportunityDetector::new().detect(
                        &svc.fact_store,
                        &svc.pulse,
                        &svc.opportunity_queue,
                    );
                    let mut humor_needed: Vec<(BuddyOpportunity, BuddyFactKind, u64)> = vec![];
                    for (opp, cooldown_secs) in candidates {
                        if svc.operator_features.storage {
                            native_ready.push((opp, cooldown_secs));
                            continue;
                        }
                        match evaluate_with_mutes(
                            &opp,
                            &svc.settings,
                            &svc.opportunity_queue,
                            &svc.state.muted_rules,
                        ) {
                            PolicyDecision::Drop {
                                reason,
                            } => {
                                tracing::debug!("buddy: opp dropped by policy: {}", reason);
                            }
                            PolicyDecision::Surface {
                                humor_allowed,
                            } => {
                                if humor_allowed && !svc.operator_features.editorial {
                                    let kind =
                                        primary_fact_kind_for_opportunity(&opp, &svc.fact_store);
                                    humor_needed.push((opp, kind, cooldown_secs));
                                } else {
                                    svc.surface_opportunity_with_cooldown(opp, cooldown_secs);
                                }
                            }
                        }
                    }
                    let pulse = svc.pulse.clone();
                    let humor_arc = svc.humor_service.clone();
                    (humor_needed, pulse, humor_arc)
                } else {
                    (
                        vec![],
                        BuddyPulse::default(),
                        Arc::new(tokio::sync::Mutex::new(HumorService::new())),
                    )
                }
            }; // buddy lock released — LLM humor calls happen outside the lock

            // Phase 2: attach humor outside the buddy lock.
            let mut ready: Vec<(BuddyOpportunity, u64)> = native_ready;
            for (mut opp, kind, cooldown_secs) in humor_tasks {
                let legacy_humor_allowed = {
                    let buddy = context.buddy_arc.lock().await;
                    buddy
                        .as_ref()
                        .is_some_and(|svc| !svc.operator_features.editorial)
                };
                if !legacy_humor_allowed {
                    opp.humor = None;
                    ready.push((opp, cooldown_secs));
                    continue;
                }
                let plan = {
                    let mut humor = humor_arc.lock().await;
                    humor.plan_humor(kind, &pulse_for_humor)
                };
                match plan {
                    HumorPlan::Ready(line) => {
                        opp.humor = Some(line);
                    }
                    HumorPlan::Generate(reservation) => {
                        let legacy_humor_allowed = {
                            let buddy = context.buddy_arc.lock().await;
                            buddy
                                .as_ref()
                                .is_some_and(|svc| !svc.operator_features.editorial)
                        };
                        if !legacy_humor_allowed {
                            opp.humor = None;
                            ready.push((opp, cooldown_secs));
                            continue;
                        }
                        let lines = reservation.generate(gcx.clone()).await;
                        let line = {
                            let mut humor = humor_arc.lock().await;
                            humor.complete_humor(reservation, lines)
                        };
                        if let Some(line) = line {
                            opp.humor = Some(line);
                        }
                    }
                    HumorPlan::Skip => {}
                }
                ready.push((opp, cooldown_secs));
            }

            // Persistence never owns the actor mutex.
            for (opp, cooldown_secs) in ready {
                let admission = {
                    let mut guard = context.buddy_arc.lock().await;
                    guard
                        .as_mut()
                        .filter(|svc| {
                            source_owner
                                .as_ref()
                                .is_some_and(|owner| svc.has_admission_identity(owner))
                        })
                        .ok_or("Buddy observer owner changed".to_string())
                        .and_then(|svc| svc.prepare_opportunity(opp, cooldown_secs))
                };
                if let Err(error) = async { admission?.commit(&context.buddy_arc).await }.await {
                    tracing::warn!(%error, "operator admission failed");
                }
            }
        }
    }
}

async fn handle_tick_scheduler(gcx: AppState, context: &BuddyBackgroundContext, expiry_tick: u64) {
    if expiry_tick % 30 == 0 {
        context
            .scheduler
            .tick(
                gcx.clone(),
                context.buddy_arc.clone(),
                &context.project_root,
            )
            .await;
    }
}

async fn handle_publish_and_persist(context: &BuddyBackgroundContext) {
    if let Err(error) = publish_operator_inputs(&context.buddy_arc).await {
        tracing::debug!("buddy: operator publication deferred: {}", error);
    }
    let state_to_save = {
        let mut buddy = context.buddy_arc.lock().await;
        buddy.as_mut().and_then(|svc| {
            if svc.dirty {
                svc.dirty = false;
                Some(svc.state.clone())
            } else {
                None
            }
        })
    };
    if let Some(s) = state_to_save {
        if let Err(e) = super::state::save_state(&context.project_root, &s).await {
            warn!("buddy: failed to save state: {}", e);
            if let Some(svc) = context.buddy_arc.lock().await.as_mut() {
                svc.dirty = true;
            }
        }
    }
    let drafts_to_save = {
        let mut buddy = context.buddy_arc.lock().await;
        buddy.as_mut().and_then(|svc| {
            if svc.drafts_dirty && !svc.operator_features.storage {
                svc.drafts_dirty = false;
                Some(svc.draft_store.snapshot())
            } else {
                None
            }
        })
    };
    if let Some(drafts) = drafts_to_save {
        if let Err(e) = super::storage::save_drafts(&context.project_root, &drafts).await {
            warn!("buddy: failed to save drafts: {}", e);
            if let Some(svc) = context.buddy_arc.lock().await.as_mut() {
                svc.drafts_dirty = true;
            }
        }
    }
}

async fn handle_compact_runtime_queue(context: &BuddyBackgroundContext, expiry_tick: u64) {
    // Periodic compaction: ask the writer task to rewrite the JSONL from
    // the in-memory queue every 5 minutes. The append-on-mutation log can
    // grow unbounded under churn (progress updates, repeated coalesces);
    // this collapses it back to one line per surviving event. Routing
    // through the same channel keeps writes strictly ordered.
    if expiry_tick % 300 == 0 {
        let buddy = context.buddy_arc.lock().await;
        if let Some(svc) = buddy.as_ref() {
            if let Some(tx) = &svc.queue_writer {
                let _ = tx.send(RuntimeQueueWriteOp::Compact(svc.runtime_queue.clone()));
            }
        }
    }
}

async fn handle_compact_memory_ops(context: &BuddyBackgroundContext, expiry_tick: u64) {
    if expiry_tick % MEMORY_OPS_COMPACT_INTERVAL_SECS == 0 {
        match super::storage::compact_memory_ops(&context.project_root).await {
            Ok(memory_ops) => {
                if let Err(error) =
                    BuddyService::set_memory_ops_durable(&context.buddy_arc, memory_ops).await
                {
                    tracing::warn!(%error, "operator memory admission failed");
                }
            }
            Err(err) => warn!("buddy: failed to compact memory ops queue: {}", err),
        }
    }
}

async fn handle_drain_memory_ops(
    gcx: AppState,
    context: &BuddyBackgroundContext,
    expiry_tick: u64,
) {
    if expiry_tick % MEMORY_OPS_DRAIN_INTERVAL_SECS == 0 {
        let drain_settings = {
            let buddy = context.buddy_arc.lock().await;
            buddy
                .as_ref()
                .map(|svc| (svc.settings.autonomy_level, svc.settings.enabled))
        };
        if let Some((autonomy, enabled)) = drain_settings {
            if enabled {
                match super::storage::drain_memory_ops(
                    &context.project_root,
                    gcx.clone(),
                    autonomy,
                    MEMORY_OPS_DRAIN_MAX_APPLIES,
                )
                .await
                {
                    Ok((memory_ops, changed)) => {
                        if let Err(error) =
                            BuddyService::set_memory_ops_durable(&context.buddy_arc, memory_ops)
                                .await
                        {
                            tracing::warn!(%error, "operator memory admission failed");
                        }
                        if changed > 0 {
                            info!("buddy: drained {} memory ops", changed);
                        }
                    }
                    Err(err) => {
                        warn!("buddy: failed to drain memory ops queue: {}", err)
                    }
                }
            }
        }
    }
}

async fn run_buddy_maintenance(gcx: AppState, context: &BuddyBackgroundContext) {
    handle_startup_suggestion(&context.project_root, &context.buddy_arc).await;
    let mut expiry_tick = 0;
    loop {
        if context.shutdown_flag.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        expiry_tick += 1;
        handle_operator_maintenance(gcx.clone(), context, expiry_tick).await;
        handle_expire_suggestions(context, expiry_tick).await;
        handle_refresh_pulse(gcx.clone(), context, expiry_tick).await;
        handle_refresh_quiet_window(gcx.clone(), context).await;
        handle_run_observers(gcx.clone(), context).await;
        handle_tick_scheduler(gcx.clone(), context, expiry_tick).await;
        handle_publish_and_persist(context).await;
        handle_compact_runtime_queue(context, expiry_tick).await;
        handle_compact_memory_ops(context, expiry_tick).await;
        handle_drain_memory_ops(gcx.clone(), context, expiry_tick).await;
    }
}

pub async fn buddy_background_task(gcx: AppState) {
    let Some(context) = initialize_buddy_background(gcx.clone()).await else {
        return;
    };
    let maintenance = run_buddy_maintenance(gcx, &context);
    drive_maintenance(
        &context.buddy_arc,
        context.shutdown_flag.clone(),
        maintenance,
    )
    .await;
    shutdown_buddy_background(context).await;
}

async fn shutdown_buddy_background(context: BuddyBackgroundContext) {
    let state_opt = {
        let buddy = context.buddy_arc.lock().await;
        buddy.as_ref().map(|s| s.state.clone())
    };
    if let Some(s) = state_opt {
        let _ = super::state::save_state(&context.project_root, &s).await;
    }
    let drafts_opt = {
        let buddy = context.buddy_arc.lock().await;
        buddy
            .as_ref()
            .filter(|s| s.drafts_dirty && !s.operator_features.storage)
            .map(|s| s.draft_store.snapshot())
    };
    if let Some(drafts) = drafts_opt {
        let _ = super::storage::save_drafts(&context.project_root, &drafts).await;
    }

    // Final compaction on shutdown so the JSONL on disk is canonical, then
    // drop the writer sender (replacing the service slot does that for us)
    // and wait for the writer task to drain.
    // Take the whole service out of the slot atomically FIRST so any concurrent add_diagnostic
    // observes None and cannot spawn a new tracked task or enqueue new runtime records. Send the
    // final compaction from the taken service so nothing can mutate Buddy state after it.
    // Dropping the taken service also drops the writer sender so the writer task can finish.
    let mut service = context.buddy_arc.lock().await.take();
    if let Some(svc) = service.as_ref() {
        if let Some(tx) = &svc.queue_writer {
            let _ = tx.send(RuntimeQueueWriteOp::Compact(svc.runtime_queue.clone()));
        }
    }
    let mut background_tasks = service
        .as_mut()
        .map(|svc| svc.take_background_tasks())
        .unwrap_or_default();
    let drain = async {
        for handle in background_tasks.iter_mut() {
            let _ = handle.await;
        }
    };
    if tokio::time::timeout(std::time::Duration::from_secs(10), drain)
        .await
        .is_err()
    {
        warn!("buddy: background tasks did not drain within 10s, aborting remaining");
        for handle in &background_tasks {
            handle.abort();
        }
    }
    // Release the operator store's exclusive writer lock so the next engine or
    // daemon owner can open it; the database itself is left untouched.
    if let Some(runtime) = service.as_ref().and_then(|svc| svc.outcome_runtime.clone()) {
        runtime.shutdown().await;
    }
    if let Some(svc) = service.as_mut() {
        svc.shutdown_operator_reopen().await;
    }
    drop(service);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), context.writer_handle).await;
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        context.commit_poller_handle,
    )
    .await;

    info!("buddy: background task stopped");
}

#[cfg(test)]
#[path = "actor_operator_tests.rs"]
mod operator_tests;

#[cfg(test)]
mod migration_restart_regressions {
    use super::*;

    #[tokio::test]
    async fn stale_owner_failed_history_and_restart_preserve_committed_cursor() {
        let root =
            tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "migration-restart".into(),
            canonical_root: root.path().to_string_lossy().into_owned(),
        };
        let worker = refact_buddy_core::operator_types::WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        };
        let runtime = super::super::outcome_runtime::OutcomeRuntime::open(
            root.path(),
            project.clone(),
            worker.clone(),
        )
        .unwrap();
        let mut svc = BuddyService::new(
            root.path().to_path_buf(),
            super::super::state::default_buddy_state(),
            Default::default(),
            vec![],
            RuntimeQueue::new(),
            broadcast::channel(64).0,
            None,
        );
        // Direct retained-state fixture avoids the intentionally bounded live queue.
        svc.state.suggestion_state = (0..260)
            .map(|i| BuddySuggestion {
                id: format!("seed-{i:04}"),
                suggestion_type: "setup".into(),
                title: format!("Proposal {i}"),
                description: "Review only".into(),
                created_at: "2020-01-01T00:00:00Z".into(),
                dismissed: false,
                controls: vec![],
                quest: None,
            })
            .collect();
        svc.operator_features.storage = true;
        let buddy = Arc::new(tokio::sync::Mutex::new(Some(svc)));
        assert!(publish_operator_inputs(&buddy).await.is_err());
        assert_eq!(
            buddy
                .lock()
                .await
                .as_ref()
                .unwrap()
                .operator_legacy_seed_cursor,
            0
        );
        buddy.lock().await.as_mut().unwrap().outcome_runtime = Some(runtime.clone());
        let path = super::super::receipts::receipts_path(root.path());
        tokio::fs::write(&path, b"invalid-json").await.unwrap();
        assert!(publish_operator_inputs(&buddy).await.is_err());
        {
            let guard = buddy.lock().await;
            let svc = guard.as_ref().unwrap();
            assert_eq!(svc.operator_legacy_seed_cursor, 128);
            assert!(!svc.operator_legacy_seeded);
            assert!(!svc.operator_legacy_history_seeded);
        }
        tokio::fs::write(&path, b"[]").await.unwrap();
        publish_operator_inputs(&buddy).await.unwrap();
        assert_eq!(
            buddy
                .lock()
                .await
                .as_ref()
                .unwrap()
                .operator_legacy_seed_cursor,
            256
        );
        publish_operator_inputs(&buddy).await.unwrap();
        assert!(buddy.lock().await.as_ref().unwrap().operator_legacy_seeded);
        assert!(
            !buddy
                .lock()
                .await
                .as_ref()
                .unwrap()
                .operator_legacy_history_seeded
        );
        runtime.shutdown().await;
        let reopened =
            super::super::outcome_runtime::OutcomeRuntime::open(root.path(), project, worker)
                .unwrap();
        {
            let mut guard = buddy.lock().await;
            let svc = guard.as_mut().unwrap();
            svc.outcome_runtime = Some(reopened.clone());
            svc.operator_legacy_seed_cursor = 0;
            svc.operator_legacy_seeded = false;
        }
        for _ in 0..3 {
            publish_operator_inputs(&buddy).await.unwrap();
        }
        let first = reopened.list_work_items(None, 200).await.unwrap();
        let second = reopened
            .list_work_items(first.next_after, 200)
            .await
            .unwrap();
        assert_eq!(first.entries.len() + second.entries.len(), 260);
        let mut stale_project = root.path().to_path_buf();
        stale_project.push("different-owner");
        buddy.lock().await.as_mut().unwrap().project_root = stale_project;
        assert!(publish_operator_inputs(&buddy)
            .await
            .unwrap_err()
            .contains("identity changed"));
        assert_eq!(
            buddy
                .lock()
                .await
                .as_ref()
                .unwrap()
                .operator_legacy_seed_cursor,
            260
        );
        reopened.shutdown().await;
    }
}
