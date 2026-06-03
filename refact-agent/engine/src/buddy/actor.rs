use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;
use chrono::{DateTime, Utc};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};
use uuid::Uuid;

use crate::app_state::AppState;
use refact_buddy_core::user_action::UserAction;
use super::drafts::{
    validate_draft_payload, DraftCreateError, DraftStore, DraftTarget, DraftValidationError,
};
use super::events::BuddyEvent;
use super::facts::FactStore;
use super::humor::{HumorPlan, HumorService};
use super::memory_lifecycle::{
    detect_memory_lifecycle_ops_from_knowledge_dirs, memory_lifecycle_op_counts, MemoryOpsState,
};
use super::observers::{build_observer_registry, BuddyObserver, ObserverContext};
use super::opportunities::{primary_fact_kind_for_opportunity, OpportunityDetector, OpportunityQueue};
use super::policy::{evaluate, PolicyDecision};
use super::runtime_queue::RuntimeQueue;
use super::settings::BuddySettings;
use super::snapshot::BuddySnapshot;
use super::storage::RuntimeQueueRecord;
use super::types::{
    BuddyActivity, BuddyCareAction, BuddyChatPhraseBank, BuddyDraft, BuddyFact, BuddyFactKind,
    BuddyOpportunity, BuddyPersonalityProfile, BuddyPulse, BuddyQuest, BuddyRuntimeEvent,
    BuddySpeechItem, BuddyState, BuddySuggestion, OpportunityStatus,
};
use super::voice_service::{SpeechIntent, VoiceCtx, VoiceIntent, voice_service};

const SUGGESTION_RATE_LIMIT_SECS: u64 = 300;
const SUGGESTION_EXPIRY_SECS: i64 = 300;
const PET_DECAY_INTERVAL_SECS: u64 = 15;
const OBSERVER_CONCURRENCY: usize = 4;
const MEMORY_OPS_ARCHIVE_THRESHOLD_BYTES: u64 = 32 * 1024 * 1024;
const MEMORY_OPS_COMPACT_INTERVAL_SECS: u64 = 6 * 60 * 60;
const BUDDY_SPEECH_MAX_CHARS: usize = 280;
const BUDDY_TITLE_MAX_CHARS: usize = 120;
const BUDDY_DESCRIPTION_MAX_CHARS: usize = 500;
const BUDDY_STATIC_SPEECH_FALLBACK: &str = "Tiny gremlin update: something needs attention.";
const BUDDY_STATIC_TITLE_FALLBACK: &str = "Buddy update";
const BUDDY_STATIC_DESCRIPTION_FALLBACK: &str = "Buddy has an update ready.";

pub(crate) fn chat_phrase_bank_is_fresh(bank: &BuddyChatPhraseBank, now: DateTime<Utc>) -> bool {
    bank.day == now.date_naive().to_string()
}

pub(crate) async fn observe_buddy_facts_parallel(
    due_observers: Vec<Arc<dyn BuddyObserver>>,
    gcx: AppState,
    project_root: std::path::PathBuf,
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut pending = FuturesUnordered::new();
    let mut all_facts = Vec::new();
    for obs in due_observers {
        let gcx = gcx.clone();
        let project_root = project_root.clone();
        pending.push(async move {
            let ctx = ObserverContext { project_root, now };
            tokio::time::timeout(tokio::time::Duration::from_secs(5), obs.observe(gcx, &ctx))
                .await
                .unwrap_or_default()
        });
        if pending.len() >= OBSERVER_CONCURRENCY {
            if let Some(facts) = pending.next().await {
                all_facts.extend(facts);
            }
        }
    }
    while let Some(facts) = pending.next().await {
        all_facts.extend(facts);
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
                UserAction::CommitMade { sha, .. } => Some(sha.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        for action in actions {
            if let UserAction::CommitMade { sha, .. } = &action {
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

pub(crate) fn redact_diagnostic_metadata(value: &str) -> Option<String> {
    let redacted = redact_sensitive(value).trim().to_string();
    if redacted.is_empty() {
        None
    } else {
        Some(redacted)
    }
}

/// Single producer/consumer for the runtime_queue.jsonl log. Funneling all
/// mutations through one task gives us a strict total order on disk that
/// matches the in-memory mutation order, which is what makes restart-replay
/// correct in the face of concurrent backend events.
#[derive(Debug)]
pub enum RuntimeQueueWriteOp {
    Append(RuntimeQueueRecord),
    Compact(RuntimeQueue),
}

pub async fn run_runtime_queue_writer(
    project_root: std::path::PathBuf,
    mut rx: mpsc::UnboundedReceiver<RuntimeQueueWriteOp>,
) {
    while let Some(op) = rx.recv().await {
        match op {
            RuntimeQueueWriteOp::Append(record) => {
                if let Err(err) =
                    super::storage::append_runtime_record(&project_root, &record).await
                {
                    warn!("buddy: failed to persist runtime queue record: {}", err);
                }
            }
            RuntimeQueueWriteOp::Compact(queue) => {
                if let Err(err) = super::storage::compact_runtime_queue(&project_root, &queue).await
                {
                    warn!("buddy: failed to compact runtime queue: {}", err);
                }
            }
        }
    }
}

pub struct BuddyService {
    pub state: BuddyState,
    pub settings: BuddySettings,
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
    pub last_observer_tick: HashMap<&'static str, DateTime<Utc>>,
    pub observers: Vec<Arc<dyn BuddyObserver>>,
    pub chat_reaction_limiter: crate::buddy::chat_reactions::ChatReactionLimiter,
    pub chat_reaction_debug: crate::buddy::chat_reactions::ChatReactionDebugState,
    #[cfg(test)]
    pub force_next_runtime_enqueue_drop: bool,
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
        "{} pending ops, {} stuck tasks",
        pulse.memory.pending_ops, pulse.tasks.stuck
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
        "{} pending ops, {} stuck tasks",
        pulse.memory.pending_ops, pulse.tasks.stuck
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
        "{} pending ops, {} stuck tasks",
        pulse.memory.pending_ops, pulse.tasks.stuck
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
            project_root,
            state,
            settings,
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
            last_observer_tick: HashMap::new(),
            observers: build_observer_registry(),
            chat_reaction_limiter: crate::buddy::chat_reactions::ChatReactionLimiter::new(),
            chat_reaction_debug: crate::buddy::chat_reactions::ChatReactionDebugState::new(),
            #[cfg(test)]
            force_next_runtime_enqueue_drop: false,
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
        self.persist_record(RuntimeQueueRecord::Event { event });
    }

    fn persist_removal(&self, id: String) {
        self.persist_record(RuntimeQueueRecord::Removed { id });
    }

    fn persist_now_playing(&self, slot: Option<BuddyRuntimeEvent>) {
        self.persist_record(RuntimeQueueRecord::NowPlaying { event: slot });
    }

    pub fn snapshot(&self) -> BuddySnapshot {
        let opportunities = self.opportunity_queue.snapshot();
        let mut state = self.state.clone();
        state.opportunities = opportunities.clone();
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

    pub fn add_opportunity_with_cooldown(&mut self, opp: BuddyOpportunity, cooldown_secs: u64) {
        self.opportunity_queue
            .push_with_cooldown(opp.clone(), cooldown_secs);
        self.state.opportunities = self.opportunity_queue.snapshot();
        self.state.dismissed_history = self.opportunity_queue.dismissed_history_snapshot();
        self.dirty = true;
        let _ = self
            .events_tx
            .send(BuddyEvent::OpportunityProduced { opportunity: opp });
    }

    pub fn surface_opportunity_with_cooldown(
        &mut self,
        mut opp: BuddyOpportunity,
        cooldown_secs: u64,
    ) -> bool {
        match evaluate(&opp, &self.settings, &self.opportunity_queue) {
            PolicyDecision::Drop { reason } => {
                tracing::debug!("buddy: opportunity dropped by policy: {}", reason);
                false
            }
            PolicyDecision::Surface { humor_allowed } => {
                opp.humor_allowed = humor_allowed;
                self.add_opportunity_with_cooldown(opp, cooldown_secs);
                true
            }
        }
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
        let _ = self.events_tx.send(BuddyEvent::PulseUpdated { pulse });
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

    pub fn create_draft(
        &mut self,
        kind: super::types::DraftKind,
        title: String,
        yaml_or_json: String,
        explanation: String,
    ) -> Result<BuddyDraft, DraftCreateError> {
        validate_draft_payload(&title, &yaml_or_json, &explanation)?;
        let draft = self
            .draft_store
            .create(kind, title, yaml_or_json, explanation);
        let _ = self.events_tx.send(BuddyEvent::DraftCreated {
            draft: draft.clone(),
        });
        Ok(draft)
    }

    pub fn delete_draft(&mut self, id: &str) -> Option<BuddyDraft> {
        let draft = self.draft_store.delete(id)?;
        let _ = self.events_tx.send(BuddyEvent::DraftRemoved {
            draft_id: id.to_string(),
        });
        Some(draft)
    }

    pub fn consume_draft(&mut self, id: &str) -> Option<BuddyDraft> {
        let draft = self.draft_store.consume(id)?;
        let _ = self.events_tx.send(BuddyEvent::DraftConsumed {
            draft_id: id.to_string(),
        });
        Some(draft)
    }

    pub fn expire_drafts(&mut self, now: DateTime<Utc>) -> Vec<String> {
        let expired = self.draft_store.expire_old(now);
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

    pub fn update_speech(&mut self, speech: BuddySpeechItem) {
        if let Some(key) = &speech.dedupe_key {
            if let Some(existing) = &self.active_speech {
                if existing.dedupe_key.as_deref() == Some(key.as_str()) {
                    self.active_speech = Some(speech.clone());
                    let _ = self.events_tx.send(BuddyEvent::SpeechUpdated { speech });
                    return;
                }
            }
        }
        self.active_speech = Some(speech.clone());
        let _ = self.events_tx.send(BuddyEvent::SpeechUpdated { speech });
    }

    pub fn send_navigation(&self, page: super::types::BuddyPage) {
        let _ = self.events_tx.send(BuddyEvent::NavigationRequest { page });
    }

    pub fn enqueue_runtime_event(&mut self, event: BuddyRuntimeEvent) {
        let _ = self.enqueue_runtime_event_with_stored(event);
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
            let _ = self
                .events_tx
                .send(BuddyEvent::RuntimeEvent { event: ev.clone() });
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

    #[allow(dead_code)]
    pub fn update_runtime_progress(&mut self, dedupe_key: &str, progress: u8, title: Option<&str>) {
        self.runtime_queue
            .update_progress(dedupe_key, progress, title);
        if let Some(e) = self
            .runtime_queue
            .items
            .iter()
            .find(|e| e.dedupe_key.as_deref() == Some(dedupe_key))
            .cloned()
        {
            let _ = self
                .events_tx
                .send(BuddyEvent::RuntimeEvent { event: e.clone() });
            self.persist_event(e);
        }
        // If progress also touched now_playing, persist the new slot value.
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

    pub fn complete_runtime_event(&mut self, dedupe_key: &str, status: &str) {
        self.runtime_queue.complete(dedupe_key, status);
        if let Some(e) = self
            .runtime_queue
            .items
            .iter()
            .find(|e| e.dedupe_key.as_deref() == Some(dedupe_key))
            .cloned()
        {
            let _ = self
                .events_tx
                .send(BuddyEvent::RuntimeEvent { event: e.clone() });
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
        let _ = self.events_tx.send(BuddyEvent::ActivityAdded { activity });
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
                    state: self.state.clone(),
                });
            }
            return;
        }

        let Some(quest) = super::state::complete_active_quest(&mut self.state) else {
            return;
        };

        let reward = quest.reward_xp;
        let title = quest.title.clone();
        let icon = quest.icon.clone();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.state.clone(),
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
        self.update_speech(BuddySpeechItem {
            id: format!("quest-complete-{}", quest.id),
            text: format!("Quest complete: {title}! Tiny victory dance?"),
            mood: "happy".to_string(),
            scope: "global".to_string(),
            persistent: false,
            ttl_seconds: 12,
            dedupe_key: Some(format!("quest_complete_{}", quest.quest_type)),
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
        if reward > 0 {
            self.grant_xp(reward);
        }
    }

    pub fn dismiss_quest(&mut self) {
        super::state::clear_active_quest(&mut self.state);
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.state.clone(),
        });
    }

    #[allow(dead_code)]
    pub fn accept_quest(&mut self, quest: BuddyQuest) {
        let title = quest.title.clone();
        super::state::activate_quest(&mut self.state, quest.clone());
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.state.clone(),
        });
        self.update_speech(BuddySpeechItem {
            id: format!("quest-accept-{}", quest.id),
            text: format!("Quest accepted: {title}. I’ll keep score from here."),
            mood: "happy".to_string(),
            scope: "global".to_string(),
            persistent: false,
            ttl_seconds: 12,
            dedupe_key: Some(format!("quest_accept_{}", quest.quest_type)),
            created_at: Utc::now().to_rfc3339(),
            controls: quest.controls,
            chat_id: None,
        });
    }

    pub fn grant_xp(&mut self, amount: u64) {
        super::state::grant_xp(&mut self.state, amount);
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.state.clone(),
        });
    }

    #[allow(dead_code)]
    pub fn apply_care_action(&mut self, action: BuddyCareAction, toy: Option<&str>) -> String {
        let (_, message) = super::state::apply_care_action(&mut self.state, action.clone(), toy);
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.state.clone(),
        });
        message
    }

    #[allow(dead_code)]
    pub fn reroll_personality(&mut self) {
        super::state::reroll_personality(&mut self.state);
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.state.clone(),
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
            state: self.state.clone(),
        });
    }

    pub fn add_suggestion(&mut self, suggestion: BuddySuggestion) {
        if self.state.suggestion_state.len() >= 50 {
            if let Some(pos) = self.state.suggestion_state.iter().position(|s| s.dismissed) {
                self.state.suggestion_state.remove(pos);
            }
        }
        self.state.suggestion_state.push(suggestion.clone());
        self.last_suggestion_at = Some(Instant::now());
        self.dirty = true;
        let _ = self
            .events_tx
            .send(BuddyEvent::SuggestionAdded { suggestion });
    }

    pub fn maybe_add_suggestion(&mut self, suggestion: BuddySuggestion) -> bool {
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
        if let Some(s) = self.state.suggestion_state.iter_mut().find(|s| s.id == id) {
            s.dismissed = true;
        }
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::SuggestionDismissed {
            suggestion_id: id.to_string(),
        });
    }

    #[allow(dead_code)]
    pub fn workflow_completed(
        &mut self,
        workflow_id: &str,
        xp: u64,
        activity: super::types::BuddyActivity,
    ) {
        super::state::add_activity(&mut self.state, activity.clone());
        let _ = self.events_tx.send(BuddyEvent::ActivityAdded { activity });
        super::state::grant_xp(&mut self.state, xp);
        let now = Utc::now().to_rfc3339();
        if let Some(ws) = self
            .state
            .workflow_summaries
            .iter_mut()
            .find(|w| w.workflow_id == workflow_id)
        {
            ws.last_run = Some(now);
            ws.run_count += 1;
            ws.last_outcome = Some("success".to_string());
            ws.failure_category = None;
            ws.failure_summary = None;
        } else {
            self.state
                .workflow_summaries
                .push(super::types::BuddyWorkflowSummary {
                    workflow_id: workflow_id.to_string(),
                    last_run: Some(now),
                    run_count: 1,
                    failure_category: None,
                    failure_summary: None,
                    last_outcome: Some("success".to_string()),
                });
        }
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.state.clone(),
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
                });
        }
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.state.clone(),
        });
    }

    pub fn record_workflow_failure_report(
        &mut self,
        report: super::workflows::WorkflowFailureReport,
    ) -> Option<(std::path::PathBuf, super::workflows::WorkflowFailureReport)> {
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
                });
        }
        self.refresh_active_quest();
        self.dirty = true;
        let _ = self.events_tx.send(BuddyEvent::StateUpdated {
            state: self.state.clone(),
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
            .take(20)
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
        tokio::spawn(async move {
            if let Err(err) = super::storage::append_diagnostic(&project_root, &ctx_for_disk).await
            {
                warn!("buddy: failed to persist diagnostic history: {}", err);
            }
        });
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
        self.recent_issue_errors
            .push((error_message, chrono::Utc::now()));
        if self.recent_issue_errors.len() > 200 {
            self.recent_issue_errors.remove(0);
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
        let lower = error_msg.to_lowercase();
        let severity = if lower.contains("critical") || lower.contains("panic") {
            super::diagnostics::DiagnosticSeverity::Critical
        } else if lower.contains("error") {
            super::diagnostics::DiagnosticSeverity::High
        } else if lower.contains("warn") {
            super::diagnostics::DiagnosticSeverity::Medium
        } else {
            super::diagnostics::DiagnosticSeverity::High
        };
        let ctx = super::diagnostics::DiagnosticContext {
            error_type: error_type.to_string(),
            error_message: error_msg.to_string(),
            source_file: source.map(|s| s.to_string()),
            tool_name: None,
            chat_id: chat_id.map(|s| s.to_string()),
            collected_at: Utc::now().to_rfc3339(),
            severity,
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
        let now = chrono::Utc::now();
        let mut changed = false;
        for s in self.state.suggestion_state.iter_mut() {
            if s.dismissed {
                continue;
            }
            if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&s.created_at) {
                let age = now.signed_duration_since(created).num_seconds();
                if age > SUGGESTION_EXPIRY_SECS {
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
            if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&s.created_at) {
                now.signed_duration_since(created).num_seconds() < 3600
            } else {
                false
            }
        });
        if changed || self.state.suggestion_state.len() != before {
            self.dirty = true;
            let _ = self.events_tx.send(BuddyEvent::StateUpdated {
                state: self.state.clone(),
            });
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

pub async fn buddy_snapshot(gcx: AppState) -> Option<BuddySnapshot> {
    let buddy_arc = gcx.buddy.buddy.clone();
    let lock = buddy_arc.lock().await;
    lock.as_ref().map(|svc| svc.snapshot())
}

pub async fn buddy_update_speech(gcx: AppState, speech: BuddySpeechItem) {
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.update_speech(speech);
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
    let buddy_arc = gcx.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.report_error(error_type, error_msg, source, chat_id);
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
        svc.grant_xp(m.xp);
    }
    if let Some(activity) = m.activity {
        svc.add_activity(activity);
    }
    if let Some(mood) = m.mood {
        svc.state.semantic.mood = mood;
        svc.dirty = true;
        let _ = svc.events_tx.send(BuddyEvent::StateUpdated {
            state: svc.state.clone(),
        });
    }
}

pub async fn buddy_background_task(gcx: AppState) {
    let project_root = loop {
        if gcx.runtime.shutdown_flag.load(Ordering::SeqCst) {
            return;
        }
        let dirs = crate::files_correction::get_project_dirs(gcx.gcx.clone()).await;
        if let Some(root) = dirs.into_iter().next() {
            break root;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    };

    if let Err(e) = super::storage::bootstrap_buddy_storage(&project_root).await {
        warn!("buddy: failed to bootstrap storage: {}", e);
        return;
    }
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
    let memory_ops = super::storage::load_memory_ops(&project_root).await;
    let runtime_queue = super::storage::load_runtime_queue(&project_root).await;

    let events_tx = gcx.buddy.buddy_events_tx.clone();

    // Spawn the single-writer task that owns runtime_queue.jsonl. All queue
    // mutations forward to this channel, which preserves on-disk write order.
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
    service.memory_ops = memory_ops;

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
                svc.add_suggestion(suggestion);
            }
        }
    }

    info!("buddy: service started for {:?}", project_root);

    let scheduler = super::scheduler::BuddyScheduler::new();
    let shutdown_flag = gcx.runtime.shutdown_flag.clone();
    let commit_poller_handle = tokio::spawn(commit_activity_poller(
        gcx.clone(),
        project_root.clone(),
        shutdown_flag.clone(),
    ));
    let mut expiry_tick: u64 = 0;

    loop {
        if shutdown_flag.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        expiry_tick += 1;
        if expiry_tick % PET_DECAY_INTERVAL_SECS == 0 {
            let mut buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                svc.apply_pet_tick(PET_DECAY_INTERVAL_SECS);
            }
        }
        if expiry_tick % 60 == 0 {
            let mut buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_mut() {
                svc.expire_suggestions();
            }
        }
        // Pulse refresh + opportunity expiry every 60s
        if expiry_tick % 60 == 0 {
            let now = Utc::now();
            let fact_snap = {
                let buddy = buddy_arc.lock().await;
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
                    let mut memory_ops = super::storage::load_memory_ops(&project_root).await;
                    for op in lifecycle_ops {
                        match super::storage::enqueue_memory_op(&project_root, op).await {
                            Ok(updated) => memory_ops = updated,
                            Err(err) => {
                                warn!("buddy: failed to enqueue memory lifecycle op: {}", err)
                            }
                        }
                    }
                    let mut buddy = buddy_arc.lock().await;
                    if let Some(svc) = buddy.as_mut() {
                        svc.memory_ops = memory_ops;
                    }
                }
                let new_pulse =
                    super::pulse::build_pulse(gcx.clone(), &project_root, &tmp_store).await;
                let mut buddy = buddy_arc.lock().await;
                if let Some(svc) = buddy.as_mut() {
                    svc.set_pulse(new_pulse);
                    svc.expire_opportunities();
                    svc.opportunity_queue.refresh_cooldowns(now);
                    svc.expire_drafts(now);
                    svc.expire_runtime_events_at(now);
                }
            }
        }
        // Observer ticking — each observer respects its own cadence
        {
            let now = Utc::now();
            let due_observers = {
                let buddy = buddy_arc.lock().await;
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
                                    Some(t) => {
                                        (now - *t).num_seconds() as u64 >= obs.cadence_seconds()
                                    }
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
                    project_root.clone(),
                    now,
                )
                .await;
                // Phase 1: ingest facts, detect candidates, add non-humor opps — all under buddy lock.
                let (humor_tasks, pulse_for_humor, humor_arc) = {
                    let mut buddy = buddy_arc.lock().await;
                    if let Some(svc) = buddy.as_mut() {
                        for obs in &due_observers {
                            svc.last_observer_tick.insert(obs.id(), now);
                        }
                        svc.fact_store.ingest_many(all_facts);
                        let candidates = OpportunityDetector::new().detect(
                            &svc.fact_store,
                            &svc.pulse,
                            &svc.opportunity_queue,
                        );
                        let mut humor_needed: Vec<(BuddyOpportunity, BuddyFactKind, u64)> = vec![];
                        for (opp, cooldown_secs) in candidates {
                            match evaluate(&opp, &svc.settings, &svc.opportunity_queue) {
                                PolicyDecision::Drop { reason } => {
                                    tracing::debug!("buddy: opp dropped by policy: {}", reason);
                                }
                                PolicyDecision::Surface { humor_allowed } => {
                                    if humor_allowed {
                                        let kind = primary_fact_kind_for_opportunity(
                                            &opp,
                                            &svc.fact_store,
                                        );
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
                let mut ready: Vec<(BuddyOpportunity, u64)> = Vec::with_capacity(humor_tasks.len());
                for (mut opp, kind, cooldown_secs) in humor_tasks {
                    let plan = {
                        let mut humor = humor_arc.lock().await;
                        humor.plan_humor(kind, &pulse_for_humor)
                    };
                    match plan {
                        HumorPlan::Ready(line) => {
                            opp.humor = Some(line);
                        }
                        HumorPlan::Generate(reservation) => {
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

                // Phase 3: re-acquire buddy lock to add humor-processed opps.
                if !ready.is_empty() {
                    let mut buddy = buddy_arc.lock().await;
                    if let Some(svc) = buddy.as_mut() {
                        for (opp, cooldown_secs) in ready {
                            svc.surface_opportunity_with_cooldown(opp, cooldown_secs);
                        }
                    }
                }
            }
        }
        if expiry_tick % 30 == 0 {
            scheduler
                .tick(gcx.clone(), buddy_arc.clone(), &project_root)
                .await;
        }
        let state_to_save = {
            let mut buddy = buddy_arc.lock().await;
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
            if let Err(e) = super::state::save_state(&project_root, &s).await {
                warn!("buddy: failed to save state: {}", e);
                if let Some(svc) = buddy_arc.lock().await.as_mut() {
                    svc.dirty = true;
                }
            }
        }

        // Periodic compaction: ask the writer task to rewrite the JSONL from
        // the in-memory queue every 5 minutes. The append-on-mutation log can
        // grow unbounded under churn (progress updates, repeated coalesces);
        // this collapses it back to one line per surviving event. Routing
        // through the same channel keeps writes strictly ordered.
        if expiry_tick % 300 == 0 {
            let buddy = buddy_arc.lock().await;
            if let Some(svc) = buddy.as_ref() {
                if let Some(tx) = &svc.queue_writer {
                    let _ = tx.send(RuntimeQueueWriteOp::Compact(svc.runtime_queue.clone()));
                }
            }
        }
        if expiry_tick % MEMORY_OPS_COMPACT_INTERVAL_SECS == 0 {
            match super::storage::compact_memory_ops(&project_root).await {
                Ok(memory_ops) => {
                    let mut buddy = buddy_arc.lock().await;
                    if let Some(svc) = buddy.as_mut() {
                        svc.memory_ops = memory_ops;
                    }
                }
                Err(err) => warn!("buddy: failed to compact memory ops queue: {}", err),
            }
        }
    }

    let state_opt = {
        let buddy = buddy_arc.lock().await;
        buddy.as_ref().map(|s| s.state.clone())
    };
    if let Some(s) = state_opt {
        let _ = super::state::save_state(&project_root, &s).await;
    }

    // Final compaction on shutdown so the JSONL on disk is canonical, then
    // drop the writer sender (replacing the service slot does that for us)
    // and wait for the writer task to drain.
    {
        let buddy = buddy_arc.lock().await;
        if let Some(svc) = buddy.as_ref() {
            if let Some(tx) = &svc.queue_writer {
                let _ = tx.send(RuntimeQueueWriteOp::Compact(svc.runtime_queue.clone()));
            }
        }
    }
    *buddy_arc.lock().await = None;
    let _ = writer_handle.await;
    let _ = commit_poller_handle.await;

    info!("buddy: background task stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_made_poller_pushes_new_shas() {
        let mut ring =
            crate::buddy::user_activity::UserActivityRing::new(PathBuf::from("/tmp/project"), 200);
        ring.push(UserAction::CommitMade {
            sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            message_first_line: "old".to_string(),
            files: 0,
            ts: Utc::now(),
        });
        let existing = ring
            .snapshot()
            .iter()
            .filter_map(|action| match action {
                UserAction::CommitMade { sha, .. } => Some(sha.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();

        for action in parse_commit_activity_from_log(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|old|old\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb|new|new commit",
        ) {
            if let UserAction::CommitMade { sha, .. } = &action {
                if existing.contains(sha) {
                    continue;
                }
            }
            ring.push(action);
        }

        let actions = ring.snapshot();
        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(action, UserAction::CommitMade { .. }))
                .count(),
            2
        );
        assert!(actions.iter().any(|action| matches!(
            action,
            UserAction::CommitMade { sha, message_first_line, .. }
                if sha == "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" && message_first_line == "new commit"
        )));
    }
}
