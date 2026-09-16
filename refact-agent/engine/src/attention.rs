//! The human attention budget.
//!
//! One person sits behind every question, approval and escalation the agent
//! tree produces, so their attention is a scarce resource that has to be
//! governed like any other. This layer sits on top of the one tracked question
//! service (`refact_chat_api::QuestionRegistry`) and never becomes a second
//! question system: it decides *whether and when* a question reaches the human,
//! while the service still owns identity, state and settlement.
//!
//! Four rules: materially identical questions produce one prompt whose single
//! answer fans out to every waiter, arrivals inside a window are presented
//! together, prompts are ranked by how much work is stalled behind them, and a
//! saturated human refuses new low-priority questions instead of queueing them.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use refact_chat_api::{
    AskQuestion, QuestionContract, QuestionError, QuestionKind, QuestionRegistry,
    QuestionSettlement, QuestionState,
};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;

/// The boundary dedup may never cross. Two questions only merge inside one
/// project and one privacy scope, however identical their text.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AttentionScope {
    pub project: String,
    pub privacy: String,
}

impl AttentionScope {
    pub fn new(project: impl Into<String>, privacy: impl Into<String>) -> Self {
        Self {
            project: project.into(),
            privacy: privacy.into(),
        }
    }

    /// A scope that can never equal any other. Used when the project or its
    /// privacy classification cannot be resolved: when in doubt, do not merge.
    pub fn unresolved() -> Self {
        Self::new(
            format!("unresolved:{}", uuid::Uuid::new_v4().simple()),
            "unresolved",
        )
    }
}

/// How much of the human's attention a question is entitled to claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionPriority {
    /// Refused rather than queued once the human is saturated, and the only
    /// priority allowed to carry a timeout default.
    Low,
    Normal,
    /// Skips the batching window because work is blocked on it right now.
    Blocking,
}

pub fn parse_priority(value: &str) -> Result<AttentionPriority, String> {
    match value {
        "low" => Ok(AttentionPriority::Low),
        "normal" => Ok(AttentionPriority::Normal),
        "blocking" => Ok(AttentionPriority::Blocking),
        other => Err(format!(
            "unknown question priority '{other}'. Valid priorities: low, normal, blocking"
        )),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttentionLimits {
    pub batch_window_ms: u64,
    pub max_outstanding: usize,
    pub default_timeout_secs: u64,
}

impl AttentionLimits {
    pub fn from_settings() -> Self {
        let settings = refact_work_core::coordination_settings();
        Self {
            batch_window_ms: settings.attention_batch_window_ms,
            max_outstanding: settings.attention_max_outstanding_questions,
            default_timeout_secs: settings.attention_default_timeout_secs,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.max_outstanding == 0 {
            return Err("attention cap must allow at least one outstanding question".into());
        }
        if self.default_timeout_secs == 0
            || self.default_timeout_secs > refact_chat_api::QUESTION_MAX_EXPIRY_SECS
        {
            return Err(format!(
                "attention default timeout must be between 1 and {} seconds",
                refact_chat_api::QUESTION_MAX_EXPIRY_SECS
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttentionError {
    Invalid(String),
    /// The human already has more questions than the budget allows.
    Backpressure {
        outstanding: usize,
        cap: usize,
    },
    NotTracked,
    Question(QuestionError),
}

impl AttentionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_attention_request",
            Self::Backpressure { .. } => "attention_backpressure",
            Self::NotTracked => "question_not_tracked",
            Self::Question(error) => error.code(),
        }
    }
}

impl fmt::Display for AttentionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid_attention_request: {reason}"),
            Self::Backpressure { outstanding, cap } => write!(
                f,
                "attention_backpressure: {outstanding} questions are already waiting on the user, \
                 which is the configured cap of {cap}. This question was not asked and nothing was \
                 approved on your behalf: proceed without an answer, or retry once the queue drains."
            ),
            Self::NotTracked => write!(f, "question_not_tracked: no such attention question"),
            Self::Question(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AttentionError {}

impl From<QuestionError> for AttentionError {
    fn from(error: QuestionError) -> Self {
        Self::Question(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttentionRequest {
    pub scope: AttentionScope,
    pub kind: QuestionKind,
    pub asker: String,
    pub answerer: String,
    pub text: String,
    pub options: Option<Vec<String>>,
    pub priority: AttentionPriority,
    pub irreversible: bool,
    pub default_answer: Option<String>,
}

impl AttentionRequest {
    pub fn new(
        scope: AttentionScope,
        kind: QuestionKind,
        asker: impl Into<String>,
        answerer: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            scope,
            kind,
            asker: asker.into(),
            answerer: answerer.into(),
            text: text.into(),
            options: None,
            priority: AttentionPriority::Normal,
            irreversible: false,
            default_answer: None,
        }
    }

    pub fn with_options(mut self, options: Option<Vec<String>>) -> Self {
        self.options = options;
        self
    }

    pub fn with_priority(mut self, priority: AttentionPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn irreversible(mut self, irreversible: bool) -> Self {
        self.irreversible = irreversible;
        self
    }

    pub fn with_default_answer(mut self, default_answer: Option<String>) -> Self {
        self.default_answer = default_answer;
        self
    }

    fn validate(&self) -> Result<(), AttentionError> {
        if [
            self.scope.project.as_str(),
            self.scope.privacy.as_str(),
            self.asker.as_str(),
            self.answerer.as_str(),
            self.text.as_str(),
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        {
            return Err(AttentionError::Invalid(
                "a question needs a scope, both principals and nonempty text".into(),
            ));
        }
        if self.default_answer.is_some() && self.priority != AttentionPriority::Low {
            return Err(AttentionError::Invalid(
                "a timeout default only applies to a question the asker marked low stakes".into(),
            ));
        }
        if self.default_answer.is_some()
            && (self.irreversible
                || matches!(
                    self.kind,
                    QuestionKind::PlanApproval | QuestionKind::PermissionEscalation
                ))
        {
            return Err(AttentionError::Invalid(
                "approvals, permission escalations and irreversible questions can never carry a default"
                    .into(),
            ));
        }
        Ok(())
    }

    fn dedup_key(&self) -> DedupKey {
        DedupKey {
            scope: self.scope.clone(),
            kind: self.kind,
            answerer: self.answerer.clone(),
            text: normalize(&self.text),
            options: self.options.clone(),
            irreversible: self.irreversible,
            default_answer: self.default_answer.clone(),
        }
    }
}

/// "Materially identical" spelled out, and deliberately conservative: same
/// scope, same kind, same human, same whitespace-normalized text, same options
/// and the same stakes. Anything else asks separately.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DedupKey {
    scope: AttentionScope,
    kind: QuestionKind,
    answerer: String,
    text: String,
    options: Option<Vec<String>>,
    irreversible: bool,
    default_answer: Option<String>,
}

impl DedupKey {
    /// A correlation the question service can hold one pending entry per, so
    /// two scopes with identical text stay two separate questions.
    fn correlation_id(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        format!("attention:{:016x}", hasher.finish())
    }
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttentionAdmission {
    pub question_id: String,
    /// True when this waiter joined a question the human was already asked, so
    /// no second prompt is owed.
    pub shared: bool,
    pub blocked_runs: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttentionPrompt {
    pub question: QuestionContract,
    pub scope: AttentionScope,
    pub priority: AttentionPriority,
    /// How many runs are stalled behind this question. Direct waiters only:
    /// the wait-for graph of card T-12 has not landed, so nothing transitive is
    /// counted here.
    pub blocked_runs: usize,
}

struct Tracked {
    key: DedupKey,
    scope: AttentionScope,
    priority: AttentionPriority,
    /// The waiter whose question the human was actually shown. Everything else
    /// in `waiters` was deduped into it.
    origin: String,
    waiters: BTreeSet<String>,
    presented: bool,
}

/// How many settled questions stay visible after the fact. A question that
/// timed out into its default is the whole reason this history exists: the
/// human has to be able to see, afterwards, what was decided without them.
const SETTLED_HISTORY: usize = 64;

struct Retired {
    question: QuestionContract,
    scope: AttentionScope,
    priority: AttentionPriority,
    origin: String,
    waiters: Vec<String>,
    presented: bool,
    retired_at_ms: i64,
}

#[derive(Default)]
struct BudgetState {
    by_key: HashMap<DedupKey, String>,
    tracked: HashMap<String, Tracked>,
    batch_opened_at: Option<DateTime<Utc>>,
    retired: VecDeque<Retired>,
}

/// What became of a question the human was asked. `Defaulted` is the state this
/// surface exists for: nobody consented, a documented default was applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionEntryState {
    Waiting,
    Answered,
    Defaulted,
    Rejected,
    Cancelled,
    Expired,
}

impl AttentionEntryState {
    fn from_question(state: QuestionState) -> Self {
        match state {
            QuestionState::Pending => Self::Waiting,
            QuestionState::Answered => Self::Answered,
            QuestionState::Defaulted => Self::Defaulted,
            QuestionState::Rejected => Self::Rejected,
            QuestionState::Cancelled => Self::Cancelled,
            QuestionState::Expired => Self::Expired,
        }
    }
}

/// One prompt as the human should see it: where it ranks, why it ranks there,
/// who was merged into it, and what settled it when they never got to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionQueueEntry {
    pub question_id: String,
    pub text: String,
    pub kind: QuestionKind,
    pub priority: AttentionPriority,
    pub state: AttentionEntryState,
    pub rank: usize,
    pub rank_reason: String,
    pub blocked_runs: usize,
    pub waiters: Vec<String>,
    /// The waiter whose ask produced the prompt the human sees.
    pub asked_by: String,
    /// Waiters that joined a question the human was already asked, so no second
    /// prompt was owed. Empty when nothing was deduped into this one.
    pub merged_waiters: Vec<String>,
    pub presented: bool,
    pub irreversible: bool,
    pub default_answer: Option<String>,
    /// The default that was applied because nobody answered in time.
    pub default_used: Option<String>,
    pub project: String,
    pub asked_at_ms: i64,
    pub settled_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionQueue {
    pub waiting: Vec<AttentionQueueEntry>,
    /// Newest first. Retained so a silent default is impossible to miss later.
    pub settled: Vec<AttentionQueueEntry>,
    pub outstanding: usize,
    pub cap: usize,
    pub batch_window_ms: u64,
    pub default_timeout_secs: u64,
    pub silent_defaults: usize,
    pub generated_at_ms: i64,
}

fn rank_reason(blocked_runs: usize, priority: AttentionPriority) -> String {
    let runs = if blocked_runs == 1 {
        "1 run blocked".to_string()
    } else {
        format!("{blocked_runs} runs blocked")
    };
    let note = match priority {
        AttentionPriority::Blocking => "blocking priority skips the batching window",
        AttentionPriority::Normal => "normal priority",
        AttentionPriority::Low => "low priority, refused when saturated",
    };
    format!("{runs} · {note}")
}

fn queue_entry(
    question: QuestionContract,
    scope: &AttentionScope,
    priority: AttentionPriority,
    origin: &str,
    waiters: Vec<String>,
    presented: bool,
    settled_at_ms: Option<i64>,
) -> AttentionQueueEntry {
    let blocked_runs = waiters.len();
    AttentionQueueEntry {
        question_id: question.id.clone(),
        text: question.text.clone(),
        kind: question.kind,
        priority,
        state: AttentionEntryState::from_question(question.state),
        rank: 0,
        rank_reason: rank_reason(blocked_runs, priority),
        blocked_runs,
        asked_by: origin.to_string(),
        merged_waiters: waiters
            .iter()
            .filter(|waiter| waiter.as_str() != origin)
            .cloned()
            .collect(),
        waiters,
        presented,
        irreversible: question.is_irreversible(),
        default_answer: question.default_answer.clone(),
        default_used: question.defaulted().map(str::to_string),
        project: scope.project.clone(),
        asked_at_ms: question.asked_at.timestamp_millis(),
        settled_at_ms,
    }
}

/// Governs what reaches the human. Wraps the one question service rather than
/// duplicating it: identity, exactly-once settlement and expiry all still
/// belong to `QuestionRegistry`.
pub struct AttentionBudget {
    questions: QuestionRegistry,
    limits: AttentionLimits,
    state: Mutex<BudgetState>,
}

impl AttentionBudget {
    pub fn new(questions: QuestionRegistry, limits: AttentionLimits) -> Result<Self, String> {
        limits.validate()?;
        Ok(Self {
            questions,
            limits,
            state: Mutex::new(BudgetState::default()),
        })
    }

    pub fn from_settings(questions: QuestionRegistry) -> Result<Self, String> {
        Self::new(questions, AttentionLimits::from_settings())
    }

    pub fn limits(&self) -> AttentionLimits {
        self.limits
    }

    pub fn questions(&self) -> &QuestionRegistry {
        &self.questions
    }

    fn guard(&self) -> MutexGuard<'_, BudgetState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Ask for a slice of the human's attention, or join the question they were
    /// already asked. A joined waiter costs the human nothing, so it is never
    /// refused for backpressure.
    pub fn join(
        &self,
        request: AttentionRequest,
        waiter: &str,
        now: DateTime<Utc>,
    ) -> Result<AttentionAdmission, AttentionError> {
        request.validate()?;
        if waiter.trim().is_empty() {
            return Err(AttentionError::Invalid(
                "a waiter must identify itself to join a question".into(),
            ));
        }
        let key = request.dedup_key();
        let mut state = self.guard();
        self.sweep(&mut state, now);
        if let Some(id) = state.by_key.get(&key).cloned() {
            if let Some(tracked) = state.tracked.get_mut(&id) {
                tracked.waiters.insert(waiter.to_string());
                tracked.priority = tracked.priority.max(request.priority);
                return Ok(AttentionAdmission {
                    question_id: id,
                    shared: true,
                    blocked_runs: tracked.waiters.len(),
                });
            }
        }
        let outstanding = state.tracked.len();
        if request.priority == AttentionPriority::Low && outstanding >= self.limits.max_outstanding
        {
            return Err(AttentionError::Backpressure {
                outstanding,
                cap: self.limits.max_outstanding,
            });
        }
        let question = self.questions.ask(
            AskQuestion::new(
                request.kind,
                &request.asker,
                &request.answerer,
                key.correlation_id(),
                &request.text,
            )
            .with_options(request.options.clone())
            .irreversible(request.irreversible)
            .with_default_answer(request.default_answer.clone())
            .with_expiry_secs(
                request
                    .default_answer
                    .as_ref()
                    .map(|_| self.limits.default_timeout_secs),
            ),
            now,
        )?;
        if request.priority != AttentionPriority::Blocking && state.batch_opened_at.is_none() {
            state.batch_opened_at = Some(now);
        }
        state.by_key.insert(key.clone(), question.id.clone());
        state.tracked.insert(
            question.id.clone(),
            Tracked {
                key,
                scope: request.scope,
                priority: request.priority,
                origin: waiter.to_string(),
                waiters: BTreeSet::from([waiter.to_string()]),
                presented: false,
            },
        );
        Ok(AttentionAdmission {
            question_id: question.id,
            shared: false,
            blocked_runs: 1,
        })
    }

    /// Stop waiting. A shared question outlives any one waiter: only the last
    /// one to leave cancels it. Returns the waiters that remain.
    pub fn leave(
        &self,
        question_id: &str,
        waiter: &str,
        now: DateTime<Utc>,
    ) -> Result<usize, AttentionError> {
        let mut state = self.guard();
        let tracked = state
            .tracked
            .get_mut(question_id)
            .ok_or(AttentionError::NotTracked)?;
        tracked.waiters.remove(waiter);
        let remaining = tracked.waiters.len();
        if remaining > 0 {
            return Ok(remaining);
        }
        if let Some(question) = self.questions.get(question_id) {
            let _ = self.questions.cancel(question_id, &question.asker, now);
        }
        self.retire(&mut state, question_id, now);
        Ok(0)
    }

    /// Move a question out of the live queue and into the retained history, so
    /// what settled it stays visible after the fact.
    fn retire(&self, state: &mut BudgetState, question_id: &str, now: DateTime<Utc>) {
        let Some(tracked) = state.tracked.remove(question_id) else {
            return;
        };
        state.by_key.remove(&tracked.key);
        let Some(question) = self.questions.get(question_id) else {
            return;
        };
        state.retired.push_back(Retired {
            question,
            scope: tracked.scope,
            priority: tracked.priority,
            origin: tracked.origin,
            waiters: tracked.waiters.into_iter().collect(),
            presented: tracked.presented,
            retired_at_ms: now.timestamp_millis(),
        });
        while state.retired.len() > SETTLED_HISTORY {
            state.retired.pop_front();
        }
    }

    /// Everything waiting on the human right now, ranked with the reason for
    /// the rank, plus what was settled without them.
    pub fn queue(&self, now: DateTime<Utc>) -> AttentionQueue {
        let mut state = self.guard();
        self.sweep(&mut state, now);
        let mut waiting: Vec<AttentionQueueEntry> = state
            .tracked
            .iter()
            .filter_map(|(id, tracked)| {
                let question = self.questions.get(id)?;
                Some(queue_entry(
                    question,
                    &tracked.scope,
                    tracked.priority,
                    &tracked.origin,
                    tracked.waiters.iter().cloned().collect(),
                    tracked.presented,
                    None,
                ))
            })
            .collect();
        waiting.sort_by(|left, right| {
            right
                .blocked_runs
                .cmp(&left.blocked_runs)
                .then_with(|| left.asked_at_ms.cmp(&right.asked_at_ms))
                .then_with(|| left.question_id.cmp(&right.question_id))
        });
        for (index, entry) in waiting.iter_mut().enumerate() {
            entry.rank = index + 1;
        }
        let settled: Vec<AttentionQueueEntry> = state
            .retired
            .iter()
            .rev()
            .map(|retired| {
                queue_entry(
                    retired.question.clone(),
                    &retired.scope,
                    retired.priority,
                    &retired.origin,
                    retired.waiters.clone(),
                    retired.presented,
                    Some(retired.retired_at_ms),
                )
            })
            .collect();
        AttentionQueue {
            outstanding: waiting.len(),
            silent_defaults: settled
                .iter()
                .filter(|entry| entry.state == AttentionEntryState::Defaulted)
                .count(),
            waiting,
            settled,
            cap: self.limits.max_outstanding,
            batch_window_ms: self.limits.batch_window_ms,
            default_timeout_secs: self.limits.default_timeout_secs,
            generated_at_ms: now.timestamp_millis(),
        }
    }

    /// The prompts the human should see now, ranked by stalled work. Blocking
    /// questions come through immediately; everything else waits for its batch.
    /// Each question is handed over exactly once.
    pub fn take_due(&self, now: DateTime<Utc>) -> Vec<AttentionPrompt> {
        let mut state = self.guard();
        self.sweep(&mut state, now);
        let window = ChronoDuration::milliseconds(self.limits.batch_window_ms as i64);
        let batch_due = state
            .batch_opened_at
            .is_some_and(|opened_at| opened_at + window <= now);
        let mut due = Vec::new();
        for (id, tracked) in state.tracked.iter_mut() {
            if tracked.presented || (tracked.priority != AttentionPriority::Blocking && !batch_due)
            {
                continue;
            }
            let Some(question) = self.questions.get(id) else {
                continue;
            };
            tracked.presented = true;
            due.push(AttentionPrompt {
                question,
                scope: tracked.scope.clone(),
                priority: tracked.priority,
                blocked_runs: tracked.waiters.len(),
            });
        }
        if batch_due {
            state.batch_opened_at = None;
        }
        due.sort_by(|left, right| {
            right
                .blocked_runs
                .cmp(&left.blocked_runs)
                .then_with(|| left.question.asked_at.cmp(&right.question.asked_at))
                .then_with(|| left.question.id.cmp(&right.question.id))
        });
        due
    }

    /// Human-facing questions still awaiting an answer.
    pub fn outstanding(&self, now: DateTime<Utc>) -> usize {
        let mut state = self.guard();
        self.sweep(&mut state, now);
        state.tracked.len()
    }

    pub fn blocked_runs(&self, question_id: &str) -> usize {
        self.guard()
            .tracked
            .get(question_id)
            .map(|tracked| tracked.waiters.len())
            .unwrap_or_default()
    }

    /// Block until the shared question settles. Every waiter on one question
    /// observes the same single outcome exactly once.
    pub async fn wait(
        &self,
        question_id: &str,
        now: DateTime<Utc>,
    ) -> Result<QuestionSettlement, AttentionError> {
        Ok(self.questions.wait(question_id, now).await?)
    }

    /// Forget questions the service has already settled so a later identical
    /// question opens a fresh prompt instead of joining a dead one.
    fn sweep(&self, state: &mut BudgetState, now: DateTime<Utc>) {
        self.questions.expire(now);
        let settled: Vec<String> = state
            .tracked
            .keys()
            .filter(|id| {
                self.questions
                    .get(id)
                    .is_none_or(|question| question.state != QuestionState::Pending)
            })
            .cloned()
            .collect();
        for id in settled {
            self.retire(state, &id, now);
        }
        if !state
            .tracked
            .values()
            .any(|tracked| !tracked.presented && tracked.priority != AttentionPriority::Blocking)
        {
            state.batch_opened_at = None;
        }
    }
}

/// The dedup boundary a chat's questions belong to: its project, and the
/// privacy classification that project carries. Anything unresolvable yields a
/// scope that never merges.
pub async fn scope_for_chat(app: &AppState, chat_id: &str) -> AttentionScope {
    let Some(project) = project_root(app, chat_id).await else {
        return AttentionScope::unresolved();
    };
    let privacy = crate::privacy::get_file_privacy_level(
        app.workspace.privacy_settings.clone(),
        project.as_path(),
    );
    AttentionScope::new(project.to_string_lossy(), format!("{privacy:?}"))
}

async fn project_root(app: &AppState, chat_id: &str) -> Option<PathBuf> {
    let session = app.chat.sessions.read().await.get(chat_id).cloned();
    if let Some(session) = session {
        if let Some(root) = session.lock().await.work_project_root.clone() {
            return Some(root);
        }
    }
    app.workspace
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .first()
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn new_budget(limits: AttentionLimits) -> AttentionBudget {
        AttentionBudget::new(QuestionRegistry::new(Default::default()).unwrap(), limits).unwrap()
    }

    fn limits() -> AttentionLimits {
        AttentionLimits {
            batch_window_ms: 1_000,
            max_outstanding: 8,
            default_timeout_secs: 60,
        }
    }

    fn scope() -> AttentionScope {
        AttentionScope::new("/project", "AllowToSendAnywhere")
    }

    fn request(text: &str) -> AttentionRequest {
        AttentionRequest::new(scope(), QuestionKind::UserInput, "agent", "user", text)
    }

    #[test]
    fn two_identical_questions_become_one_prompt_for_the_human() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let first = budget.join(request("Ship it?"), "agent-one", now).unwrap();
        let second = budget
            .join(request("  Ship   it?  "), "agent-two", now)
            .unwrap();
        assert!(!first.shared);
        assert!(second.shared);
        assert_eq!(second.question_id, first.question_id);
        assert_eq!(second.blocked_runs, 2);
        assert_eq!(budget.outstanding(now), 1);
        assert_eq!(budget.questions().list(None, now).len(), 1);
        assert_eq!(
            budget
                .take_due(now + ChronoDuration::seconds(2))
                .iter()
                .map(|prompt| prompt.question.id.clone())
                .collect::<Vec<_>>(),
            vec![first.question_id]
        );
    }

    #[tokio::test]
    async fn one_answer_settles_every_waiter_of_a_shared_question_exactly_once() {
        let budget = Arc::new(new_budget(limits()));
        let now = Utc::now();
        let admission = budget.join(request("Ship it?"), "agent-one", now).unwrap();
        for waiter in ["agent-two", "agent-three"] {
            assert!(
                budget
                    .join(request("Ship it?"), waiter, now)
                    .unwrap()
                    .shared
            );
        }
        let waiters: Vec<_> = (0..3)
            .map(|_| {
                let budget = budget.clone();
                let id = admission.question_id.clone();
                tokio::spawn(async move { budget.wait(&id, now).await })
            })
            .collect();
        tokio::task::yield_now().await;
        budget
            .questions()
            .answer(&admission.question_id, "user", "yes".into(), Utc::now())
            .unwrap();
        for waiter in waiters {
            assert_eq!(
                waiter.await.unwrap(),
                Ok(QuestionSettlement::Answered("yes".into()))
            );
        }
        assert_eq!(
            budget
                .questions()
                .answer(&admission.question_id, "user", "again".into(), Utc::now()),
            Err(QuestionError::AlreadyAnswered)
        );
        assert_eq!(
            budget
                .questions()
                .get(&admission.question_id)
                .unwrap()
                .state,
            QuestionState::Answered
        );
    }

    #[test]
    fn identical_text_in_another_project_or_privacy_scope_is_never_merged() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let mine = budget.join(request("Ship it?"), "agent-one", now).unwrap();
        for other in [
            AttentionScope::new("/other-project", "AllowToSendAnywhere"),
            AttentionScope::new("/project", "OnlySendToServersIControl"),
            AttentionScope::unresolved(),
        ] {
            let mut foreign = request("Ship it?");
            foreign.scope = other;
            let admission = budget.join(foreign, "agent-two", now).unwrap();
            assert!(!admission.shared);
            assert_ne!(admission.question_id, mine.question_id);
            assert_eq!(admission.blocked_runs, 1);
        }
        assert_eq!(budget.outstanding(now), 4);
    }

    #[test]
    fn a_different_answerer_kind_option_set_or_stake_is_asked_separately() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let base = budget.join(request("Ship it?"), "agent-one", now).unwrap();
        let mut other_answerer = request("Ship it?");
        other_answerer.answerer = "operator".into();
        let mut other_kind = request("Ship it?");
        other_kind.kind = QuestionKind::AgentParent;
        let variants = [
            other_answerer,
            other_kind,
            request("Ship it?").with_options(Some(vec!["yes".into(), "no".into()])),
            request("Ship it?").irreversible(true),
            request("Ship it?")
                .with_priority(AttentionPriority::Low)
                .with_default_answer(Some("no".into())),
        ];
        for variant in variants {
            let admission = budget.join(variant, "agent-two", now).unwrap();
            assert!(!admission.shared);
            assert_ne!(admission.question_id, base.question_id);
        }
    }

    #[test]
    fn arrivals_inside_the_window_are_presented_together_and_blocking_bypasses_it() {
        let budget = new_budget(limits());
        let now = Utc::now();
        budget.join(request("First?"), "agent-one", now).unwrap();
        budget
            .join(
                request("Second?"),
                "agent-two",
                now + ChronoDuration::milliseconds(200),
            )
            .unwrap();
        assert!(budget
            .take_due(now + ChronoDuration::milliseconds(300))
            .is_empty());
        let urgent = budget
            .join(
                request("Now?").with_priority(AttentionPriority::Blocking),
                "agent-three",
                now + ChronoDuration::milliseconds(400),
            )
            .unwrap();
        let bypassed = budget.take_due(now + ChronoDuration::milliseconds(500));
        assert_eq!(bypassed.len(), 1);
        assert_eq!(bypassed[0].question.id, urgent.question_id);
        let batch = budget.take_due(now + ChronoDuration::milliseconds(1_100));
        assert_eq!(batch.len(), 2);
        assert!(budget.take_due(now + ChronoDuration::seconds(2)).is_empty());
    }

    #[test]
    fn the_question_blocking_the_most_runs_is_ranked_first() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let lonely = budget.join(request("Lonely?"), "agent-one", now).unwrap();
        let popular = budget.join(request("Crowded?"), "agent-two", now).unwrap();
        for waiter in ["agent-three", "agent-four", "agent-five"] {
            budget.join(request("Crowded?"), waiter, now).unwrap();
        }
        let middle = budget.join(request("Shared?"), "agent-six", now).unwrap();
        budget.join(request("Shared?"), "agent-seven", now).unwrap();
        let due = budget.take_due(now + ChronoDuration::seconds(2));
        assert_eq!(
            due.iter()
                .map(|prompt| (prompt.question.id.clone(), prompt.blocked_runs))
                .collect::<Vec<_>>(),
            vec![
                (popular.question_id, 4),
                (middle.question_id, 2),
                (lonely.question_id, 1)
            ]
        );
    }

    #[test]
    fn a_low_stakes_question_times_out_into_its_documented_default() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let admission = budget
            .join(
                request("Rename the branch?")
                    .with_priority(AttentionPriority::Low)
                    .with_default_answer(Some("keep the name".into())),
                "agent-one",
                now,
            )
            .unwrap();
        let pending = budget.questions().get(&admission.question_id).unwrap();
        assert_eq!(
            pending.expires_at,
            Some(now + ChronoDuration::seconds(limits().default_timeout_secs as i64))
        );
        let later = now + ChronoDuration::seconds(limits().default_timeout_secs as i64 + 1);
        assert_eq!(budget.outstanding(later), 0);
        let settled = budget.questions().get(&admission.question_id).unwrap();
        assert_eq!(settled.state, QuestionState::Defaulted);
        assert_eq!(settled.defaulted(), Some("keep the name"));
        assert_eq!(settled.answered(), None);
        let settlement = budget
            .questions()
            .settlement(&admission.question_id, later)
            .unwrap();
        assert!(!settlement.is_answered());
        assert_eq!(settlement.default_used(), Some("keep the name"));
        assert_eq!(settlement.answer(), None);
    }

    #[test]
    fn an_approval_or_irreversible_question_never_defaults_however_long_it_waits() {
        let budget = new_budget(limits());
        let now = Utc::now();
        for irreversible in [
            request("Delete production?")
                .with_priority(AttentionPriority::Low)
                .irreversible(true),
            AttentionRequest::new(
                scope(),
                QuestionKind::PermissionEscalation,
                "agent",
                "user",
                "Run rm -rf?",
            )
            .with_priority(AttentionPriority::Low),
            AttentionRequest::new(
                scope(),
                QuestionKind::PlanApproval,
                "agent",
                "user",
                "Approve the plan?",
            )
            .with_priority(AttentionPriority::Low),
        ] {
            assert!(matches!(
                budget.join(
                    irreversible
                        .clone()
                        .with_default_answer(Some("approve".into())),
                    "agent-one",
                    now
                ),
                Err(AttentionError::Invalid(_))
            ));
            let admission = budget.join(irreversible, "agent-one", now).unwrap();
            let question = budget.questions().get(&admission.question_id).unwrap();
            assert!(question.is_irreversible());
            assert_eq!(question.defaultable(), None);
            let forever = now + ChronoDuration::days(3_650);
            budget.questions().expire(forever);
            let settled = budget.questions().get(&admission.question_id).unwrap();
            assert_eq!(settled.state, QuestionState::Expired);
            assert_eq!(settled.defaulted(), None);
            assert_eq!(settled.answered(), None);
        }
    }

    #[test]
    fn a_default_is_refused_unless_the_asker_marked_the_question_low_stakes() {
        let budget = new_budget(limits());
        let now = Utc::now();
        for priority in [AttentionPriority::Normal, AttentionPriority::Blocking] {
            assert!(matches!(
                budget.join(
                    request("Rename?")
                        .with_priority(priority)
                        .with_default_answer(Some("no".into())),
                    "agent-one",
                    now
                ),
                Err(AttentionError::Invalid(_))
            ));
        }
        assert_eq!(budget.outstanding(now), 0);
    }

    #[test]
    fn a_saturated_human_refuses_low_priority_questions_instead_of_queueing_them() {
        let budget = new_budget(AttentionLimits {
            max_outstanding: 2,
            ..limits()
        });
        let now = Utc::now();
        let first = budget.join(request("One?"), "agent-one", now).unwrap();
        budget.join(request("Two?"), "agent-two", now).unwrap();
        let refused = budget
            .join(
                request("Three?").with_priority(AttentionPriority::Low),
                "agent-three",
                now,
            )
            .unwrap_err();
        assert_eq!(
            refused,
            AttentionError::Backpressure {
                outstanding: 2,
                cap: 2
            }
        );
        assert_eq!(refused.code(), "attention_backpressure");
        let refusal = refused.to_string();
        assert!(refusal.contains("proceed without an answer"), "{refusal}");
        assert!(refusal.contains("nothing was approved"), "{refusal}");
        assert_eq!(budget.outstanding(now), 2);
        assert!(
            budget
                .join(request("One?"), "agent-three", now)
                .unwrap()
                .shared
        );
        budget
            .join(
                request("Urgent?").with_priority(AttentionPriority::Blocking),
                "agent-four",
                now,
            )
            .unwrap();
        budget
            .questions()
            .answer(&first.question_id, "user", "yes".into(), now)
            .unwrap();
        budget
            .join(
                request("Three?").with_priority(AttentionPriority::Low),
                "agent-five",
                now,
            )
            .unwrap_err();
    }

    #[test]
    fn cancelling_one_waiter_leaves_a_shared_question_alive_for_the_others() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let admission = budget.join(request("Ship it?"), "agent-one", now).unwrap();
        budget.join(request("Ship it?"), "agent-two", now).unwrap();
        assert_eq!(
            budget
                .leave(&admission.question_id, "agent-one", now)
                .unwrap(),
            1
        );
        assert_eq!(
            budget
                .questions()
                .get(&admission.question_id)
                .unwrap()
                .state,
            QuestionState::Pending
        );
        assert_eq!(budget.blocked_runs(&admission.question_id), 1);
        assert_eq!(
            budget
                .leave(&admission.question_id, "agent-two", now)
                .unwrap(),
            0
        );
        assert_eq!(
            budget
                .questions()
                .get(&admission.question_id)
                .unwrap()
                .state,
            QuestionState::Cancelled
        );
        assert_eq!(
            budget.leave(&admission.question_id, "agent-two", now),
            Err(AttentionError::NotTracked)
        );
        let reasked = budget
            .join(request("Ship it?"), "agent-three", now)
            .unwrap();
        assert!(!reasked.shared);
        assert_ne!(reasked.question_id, admission.question_id);
    }

    #[test]
    fn limits_and_priorities_are_rejected_rather_than_coerced() {
        for invalid in [
            AttentionLimits {
                max_outstanding: 0,
                ..limits()
            },
            AttentionLimits {
                default_timeout_secs: 0,
                ..limits()
            },
            AttentionLimits {
                default_timeout_secs: refact_chat_api::QUESTION_MAX_EXPIRY_SECS + 1,
                ..limits()
            },
        ] {
            assert!(invalid.validate().is_err());
            assert!(AttentionBudget::new(
                QuestionRegistry::new(Default::default()).unwrap(),
                invalid
            )
            .is_err());
        }
        assert!(parse_priority("sometimes").is_err());
        assert_eq!(parse_priority("low"), Ok(AttentionPriority::Low));
        assert_eq!(parse_priority("normal"), Ok(AttentionPriority::Normal));
        assert_eq!(parse_priority("blocking"), Ok(AttentionPriority::Blocking));
        let budget = new_budget(limits());
        let now = Utc::now();
        assert!(matches!(
            budget.join(request("Ship it?"), "  ", now),
            Err(AttentionError::Invalid(_))
        ));
        assert!(matches!(
            budget.join(request("   "), "agent-one", now),
            Err(AttentionError::Invalid(_))
        ));
    }

    #[test]
    fn the_queue_surface_ranks_by_blocked_work_and_names_the_reason() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let lonely = budget.join(request("Lonely?"), "agent-one", now).unwrap();
        let popular = budget.join(request("Crowded?"), "agent-two", now).unwrap();
        for waiter in ["agent-three", "agent-four"] {
            budget.join(request("Crowded?"), waiter, now).unwrap();
        }
        let queue = budget.queue(now);
        assert_eq!(queue.outstanding, 2);
        assert_eq!(queue.cap, limits().max_outstanding);
        assert_eq!(
            queue
                .waiting
                .iter()
                .map(|entry| (entry.question_id.clone(), entry.rank, entry.blocked_runs))
                .collect::<Vec<_>>(),
            vec![
                (popular.question_id.clone(), 1, 3),
                (lonely.question_id, 2, 1)
            ]
        );
        assert_eq!(
            queue.waiting[0].rank_reason,
            "3 runs blocked · normal priority"
        );
        assert_eq!(
            queue.waiting[1].rank_reason,
            "1 run blocked · normal priority"
        );
        assert_eq!(queue.waiting[0].asked_by, "agent-two");
        assert_eq!(
            queue.waiting[0].merged_waiters,
            vec!["agent-four".to_string(), "agent-three".to_string()],
            "the asker that produced the prompt is never listed as merged into itself"
        );
        assert_eq!(queue.waiting[1].asked_by, "agent-one");
        assert!(queue.waiting[1].merged_waiters.is_empty());
        assert_eq!(
            budget.queue(now),
            queue,
            "identical input ranks identically"
        );
    }

    #[test]
    fn a_question_that_timed_out_into_its_default_stays_visible_afterwards() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let admission = budget
            .join(
                request("Rename the branch?")
                    .with_priority(AttentionPriority::Low)
                    .with_default_answer(Some("keep the name".into())),
                "agent-one",
                now,
            )
            .unwrap();
        assert!(budget.queue(now).settled.is_empty());
        let later = now + ChronoDuration::seconds(limits().default_timeout_secs as i64 + 1);
        let queue = budget.queue(later);
        assert!(queue.waiting.is_empty());
        assert_eq!(queue.silent_defaults, 1);
        let entry = &queue.settled[0];
        assert_eq!(entry.question_id, admission.question_id);
        assert_eq!(entry.state, AttentionEntryState::Defaulted);
        assert_eq!(entry.default_used.as_deref(), Some("keep the name"));
        assert_eq!(entry.settled_at_ms, Some(later.timestamp_millis()));
    }

    #[test]
    fn an_answered_question_is_never_reported_as_a_silent_default() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let admission = budget.join(request("Ship it?"), "agent-one", now).unwrap();
        budget
            .questions()
            .answer(&admission.question_id, "user", "yes".into(), now)
            .unwrap();
        let queue = budget.queue(now);
        assert_eq!(queue.silent_defaults, 0);
        assert_eq!(queue.settled[0].state, AttentionEntryState::Answered);
        assert_eq!(queue.settled[0].default_used, None);
    }

    #[test]
    fn an_empty_queue_is_an_empty_queue_and_a_single_prompt_ranks_first() {
        let budget = new_budget(limits());
        let now = Utc::now();
        let empty = budget.queue(now);
        assert!(empty.waiting.is_empty());
        assert!(empty.settled.is_empty());
        assert_eq!(empty.outstanding, 0);
        budget.join(request("Only?"), "agent-one", now).unwrap();
        let one = budget.queue(now);
        assert_eq!(one.waiting.len(), 1);
        assert_eq!(one.waiting[0].rank, 1);
    }

    #[test]
    fn configured_limits_come_from_coordination_settings() {
        let limits = AttentionLimits::from_settings();
        limits.validate().unwrap();
        let settings = refact_work_core::coordination_settings();
        assert_eq!(limits.batch_window_ms, settings.attention_batch_window_ms);
        assert_eq!(
            limits.max_outstanding,
            settings.attention_max_outstanding_questions
        );
        assert_eq!(
            limits.default_timeout_secs,
            settings.attention_default_timeout_secs
        );
    }
}
