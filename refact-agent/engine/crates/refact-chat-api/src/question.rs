use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::{sleep_until, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    UserInput,
    AgentParent,
    BoardPlanner,
    PlanApproval,
    PermissionEscalation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionState {
    Pending,
    Answered,
    /// A low-stakes question whose documented default was applied because the
    /// human never got to it. Never reported as `Answered`: nobody consented.
    Defaulted,
    Rejected,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionContract {
    pub id: String,
    pub kind: QuestionKind,
    pub asker: String,
    pub answerer: String,
    pub correlation_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    pub state: QuestionState,
    pub asked_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Set by an asker that has an irreversible effect behind the answer. The
    /// kind already forces this for approvals and permission escalations.
    #[serde(default)]
    pub irreversible: bool,
    /// The explicit safe default a low-stakes asker supplied. Its presence is
    /// the only thing that lets expiry settle as `Defaulted` instead of
    /// `Expired`, and `validate` refuses it on anything irreversible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<DateTime<Utc>>,
}

impl QuestionContract {
    pub fn pending(
        id: impl Into<String>,
        kind: QuestionKind,
        asker: impl Into<String>,
        answerer: impl Into<String>,
        correlation_id: impl Into<String>,
        text: impl Into<String>,
        options: Option<Vec<String>>,
        asked_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Self, String> {
        let question = Self {
            id: id.into(),
            kind,
            asker: asker.into(),
            answerer: answerer.into(),
            correlation_id: correlation_id.into(),
            text: text.into(),
            options,
            state: QuestionState::Pending,
            asked_at,
            expires_at,
            irreversible: false,
            default_answer: None,
            answer: None,
            answered_at: None,
        };
        question.validate()?;
        Ok(question)
    }

    /// Declare what is at stake behind the answer. Rejected when a default is
    /// offered for something irreversible, so the combination cannot exist.
    pub fn with_stakes(
        mut self,
        irreversible: bool,
        default_answer: Option<String>,
    ) -> Result<Self, String> {
        self.irreversible = irreversible;
        self.default_answer = default_answer;
        self.validate()?;
        Ok(self)
    }

    /// Approvals and permission escalations are irreversible by kind, whatever
    /// the asker claims.
    pub fn is_irreversible(&self) -> bool {
        self.irreversible
            || matches!(
                self.kind,
                QuestionKind::PlanApproval | QuestionKind::PermissionEscalation
            )
    }

    /// The default expiry would apply, or `None` when this question must be
    /// answered by a human.
    pub fn defaultable(&self) -> Option<&str> {
        if self.is_irreversible() {
            return None;
        }
        self.default_answer.as_deref()
    }

    /// The text a human actually chose. Never returns the default: a defaulted
    /// question carries no consent.
    pub fn answered(&self) -> Option<&str> {
        match self.state {
            QuestionState::Answered => self.answer.as_deref(),
            _ => None,
        }
    }

    /// The default that was applied, when nobody answered in time.
    pub fn defaulted(&self) -> Option<&str> {
        match self.state {
            QuestionState::Defaulted => self.answer.as_deref(),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if [
            self.id.as_str(),
            self.asker.as_str(),
            self.answerer.as_str(),
            self.correlation_id.as_str(),
            self.text.as_str(),
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        {
            return Err(
                "question identity, principals, correlation and text must be nonempty".into(),
            );
        }
        if self
            .expires_at
            .is_some_and(|expires_at| expires_at <= self.asked_at)
        {
            return Err("question expiry must be after creation".into());
        }
        if self.state == QuestionState::Answered
            && (self.answer.as_deref().is_none_or(str::is_empty) || self.answered_at.is_none())
        {
            return Err("answered questions require answer text and timestamp".into());
        }
        if let Some(default) = &self.default_answer {
            if default.trim().is_empty() {
                return Err("a defaultable question needs nonempty default text".into());
            }
            if self.is_irreversible() {
                return Err(
                    "approvals, permission escalations and irreversible questions cannot carry a default"
                        .into(),
                );
            }
        }
        if self.state == QuestionState::Defaulted
            && (self.default_answer.is_none()
                || self.answer.as_deref() != self.default_answer.as_deref()
                || self.answered_at.is_none())
        {
            return Err("defaulted questions require the stored default and its timestamp".into());
        }
        Ok(())
    }
}

/// Maximum expiry any question family may request, so a configured bound can
/// never overflow the chrono arithmetic used to compute a deadline.
pub const QUESTION_MAX_EXPIRY_SECS: u64 = 31_536_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestionLimits {
    pub max_pending: usize,
    pub default_expiry_secs: u64,
    pub max_expiry_secs: u64,
}

impl Default for QuestionLimits {
    fn default() -> Self {
        Self {
            max_pending: 256,
            default_expiry_secs: 1800,
            max_expiry_secs: 604_800,
        }
    }
}

impl QuestionLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_pending == 0 {
            return Err("question max_pending must be at least 1".into());
        }
        if self.default_expiry_secs == 0 || self.max_expiry_secs == 0 {
            return Err("question expiry bounds must be at least 1 second".into());
        }
        if self.max_expiry_secs > QUESTION_MAX_EXPIRY_SECS {
            return Err(format!(
                "question max expiry must not exceed {QUESTION_MAX_EXPIRY_SECS} seconds"
            ));
        }
        if self.default_expiry_secs > self.max_expiry_secs {
            return Err("question default expiry must not exceed the maximum".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionError {
    Invalid(String),
    Capacity,
    Duplicate,
    NotFound,
    NotAsker,
    NotAnswerer,
    AlreadyAnswered,
    AlreadySettled(QuestionState),
}

impl QuestionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_question",
            Self::Capacity => "question_capacity",
            Self::Duplicate => "duplicate_question",
            Self::NotFound => "question_not_found",
            Self::NotAsker => "not_question_asker",
            Self::NotAnswerer => "not_question_answerer",
            Self::AlreadyAnswered => "already_answered",
            Self::AlreadySettled(_) => "already_settled",
        }
    }
}

impl fmt::Display for QuestionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid_question: {reason}"),
            Self::Capacity => write!(f, "question_capacity: too many pending questions"),
            Self::Duplicate => write!(f, "duplicate_question: correlation is already pending"),
            Self::NotFound => write!(f, "question_not_found"),
            Self::NotAsker => write!(f, "not_question_asker"),
            Self::NotAnswerer => write!(f, "not_question_answerer"),
            Self::AlreadyAnswered => write!(f, "already_answered"),
            Self::AlreadySettled(state) => {
                write!(f, "already_settled: question is {state:?}")
            }
        }
    }
}

impl std::error::Error for QuestionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionSettlement {
    Answered(String),
    Defaulted(String),
    Rejected,
    Cancelled,
    Expired,
}

impl QuestionSettlement {
    pub fn state(&self) -> QuestionState {
        match self {
            Self::Answered(_) => QuestionState::Answered,
            Self::Defaulted(_) => QuestionState::Defaulted,
            Self::Rejected => QuestionState::Rejected,
            Self::Cancelled => QuestionState::Cancelled,
            Self::Expired => QuestionState::Expired,
        }
    }

    /// Only a real answer. A defaulted question returns `None` here so no
    /// caller can mistake a timeout for consent.
    pub fn answer(&self) -> Option<&str> {
        match self {
            Self::Answered(answer) => Some(answer.as_str()),
            _ => None,
        }
    }

    pub fn default_used(&self) -> Option<&str> {
        match self {
            Self::Defaulted(default) => Some(default.as_str()),
            _ => None,
        }
    }

    pub fn is_answered(&self) -> bool {
        matches!(self, Self::Answered(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskQuestion {
    pub id: Option<String>,
    pub kind: QuestionKind,
    pub asker: String,
    pub answerer: String,
    pub correlation_id: String,
    pub text: String,
    pub options: Option<Vec<String>>,
    pub expires_in_secs: Option<u64>,
    pub irreversible: bool,
    pub default_answer: Option<String>,
}

impl AskQuestion {
    pub fn new(
        kind: QuestionKind,
        asker: impl Into<String>,
        answerer: impl Into<String>,
        correlation_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            id: None,
            kind,
            asker: asker.into(),
            answerer: answerer.into(),
            correlation_id: correlation_id.into(),
            text: text.into(),
            options: None,
            expires_in_secs: None,
            irreversible: false,
            default_answer: None,
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_options(mut self, options: Option<Vec<String>>) -> Self {
        self.options = options;
        self
    }

    pub fn with_expiry_secs(mut self, seconds: Option<u64>) -> Self {
        self.expires_in_secs = seconds;
        self
    }

    pub fn irreversible(mut self, irreversible: bool) -> Self {
        self.irreversible = irreversible;
        self
    }

    /// Offer an explicit safe default. Registration fails if the question is
    /// irreversible, rather than quietly dropping the default.
    pub fn with_default_answer(mut self, default_answer: Option<String>) -> Self {
        self.default_answer = default_answer;
        self
    }
}

/// The single tracked lifecycle behind every question family. One id space, one
/// correlation model, one state machine: `Pending` is the only non-terminal
/// state and `Answered`/`Rejected`/`Cancelled`/`Expired` are mutually exclusive
/// and final.
#[derive(Clone)]
pub struct QuestionRegistry {
    entries: Arc<Mutex<HashMap<String, QuestionContract>>>,
    notify: Arc<Notify>,
    limits: QuestionLimits,
}

impl QuestionRegistry {
    pub fn new(limits: QuestionLimits) -> Result<Self, String> {
        limits.validate()?;
        Ok(Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            notify: Arc::new(Notify::new()),
            limits,
        })
    }

    pub fn limits(&self) -> QuestionLimits {
        self.limits
    }

    fn guard(&self) -> MutexGuard<'_, HashMap<String, QuestionContract>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn ask(
        &self,
        request: AskQuestion,
        now: DateTime<Utc>,
    ) -> Result<QuestionContract, QuestionError> {
        let seconds = request
            .expires_in_secs
            .unwrap_or(self.limits.default_expiry_secs);
        if seconds == 0 || seconds > self.limits.max_expiry_secs {
            return Err(QuestionError::Invalid(format!(
                "question expiry must be between 1 and {} seconds, got {seconds}",
                self.limits.max_expiry_secs
            )));
        }
        let contract = QuestionContract::pending(
            request
                .id
                .unwrap_or_else(|| Uuid::new_v4().simple().to_string()),
            request.kind,
            request.asker,
            request.answerer,
            request.correlation_id,
            request.text,
            request.options,
            now,
            Some(now + ChronoDuration::seconds(seconds as i64)),
        )
        .and_then(|contract| contract.with_stakes(request.irreversible, request.default_answer))
        .map_err(QuestionError::Invalid)?;
        self.register(contract.clone(), now)?;
        Ok(contract)
    }

    pub fn register(
        &self,
        contract: QuestionContract,
        now: DateTime<Utc>,
    ) -> Result<(), QuestionError> {
        contract.validate().map_err(QuestionError::Invalid)?;
        let mut entries = self.guard();
        Self::expire_locked(&mut entries, now);
        if entries.contains_key(&contract.id) {
            return Err(QuestionError::Duplicate);
        }
        if contract.state == QuestionState::Pending {
            if entries.values().any(|entry| {
                entry.state == QuestionState::Pending
                    && entry.correlation_id == contract.correlation_id
            }) {
                return Err(QuestionError::Duplicate);
            }
            if Self::pending_count(&entries) >= self.limits.max_pending {
                return Err(QuestionError::Capacity);
            }
        }
        entries.insert(contract.id.clone(), contract);
        self.prune_locked(&mut entries);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<QuestionContract> {
        self.guard().get(id).cloned()
    }

    pub fn list(&self, kind: Option<QuestionKind>, now: DateTime<Utc>) -> Vec<QuestionContract> {
        let mut entries = self.guard();
        Self::expire_locked(&mut entries, now);
        Self::sorted(
            entries
                .values()
                .filter(|entry| kind.is_none_or(|kind| entry.kind == kind))
                .cloned(),
        )
    }

    pub fn pending_for_asker(&self, asker: &str, now: DateTime<Utc>) -> Vec<QuestionContract> {
        let mut entries = self.guard();
        Self::expire_locked(&mut entries, now);
        Self::sorted(
            entries
                .values()
                .filter(|entry| entry.state == QuestionState::Pending && entry.asker == asker)
                .cloned(),
        )
    }

    pub fn answer(
        &self,
        id: &str,
        answerer: &str,
        answer: String,
        now: DateTime<Utc>,
    ) -> Result<QuestionContract, QuestionError> {
        if answer.trim().is_empty() {
            return Err(QuestionError::Invalid(
                "answer text must be nonempty".into(),
            ));
        }
        self.settle(
            id,
            answerer,
            false,
            QuestionState::Answered,
            Some(answer),
            now,
        )
    }

    pub fn reject(
        &self,
        id: &str,
        answerer: &str,
        now: DateTime<Utc>,
    ) -> Result<QuestionContract, QuestionError> {
        self.settle(id, answerer, false, QuestionState::Rejected, None, now)
    }

    pub fn cancel(
        &self,
        id: &str,
        asker: &str,
        now: DateTime<Utc>,
    ) -> Result<QuestionContract, QuestionError> {
        self.settle(id, asker, true, QuestionState::Cancelled, None, now)
    }

    fn settle(
        &self,
        id: &str,
        principal: &str,
        require_asker: bool,
        target: QuestionState,
        answer: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<QuestionContract, QuestionError> {
        let settled = {
            let mut entries = self.guard();
            Self::expire_locked(&mut entries, now);
            let entry = entries.get_mut(id).ok_or(QuestionError::NotFound)?;
            let owner = if require_asker {
                &entry.asker
            } else {
                &entry.answerer
            };
            if owner != principal {
                return Err(if require_asker {
                    QuestionError::NotAsker
                } else {
                    QuestionError::NotAnswerer
                });
            }
            match entry.state {
                QuestionState::Pending => {}
                QuestionState::Answered => return Err(QuestionError::AlreadyAnswered),
                other => return Err(QuestionError::AlreadySettled(other)),
            }
            entry.state = target;
            if target == QuestionState::Answered {
                entry.answer = answer;
                entry.answered_at = Some(now);
            }
            entry.clone()
        };
        self.notify.notify_waiters();
        Ok(settled)
    }

    pub fn expire(&self, now: DateTime<Utc>) {
        {
            let mut entries = self.guard();
            Self::expire_locked(&mut entries, now);
            self.prune_locked(&mut entries);
        }
        self.notify.notify_waiters();
    }

    pub fn settlement(&self, id: &str, now: DateTime<Utc>) -> Option<QuestionSettlement> {
        let mut entries = self.guard();
        Self::expire_locked(&mut entries, now);
        entries.get(id).and_then(Self::settlement_of)
    }

    /// Block until the question reaches a terminal state or its own expiry
    /// elapses. An answer racing the timeout settles exactly once because both
    /// paths read the terminal state back under the same mutex.
    pub async fn wait(
        &self,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<QuestionSettlement, QuestionError> {
        let started = Instant::now();
        let snapshot = self
            .guard()
            .get(id)
            .cloned()
            .ok_or(QuestionError::NotFound)?;
        let mut timeout = snapshot.expires_at.map(|expires_at| {
            Box::pin(sleep_until(
                started
                    + Duration::from_millis((expires_at - now).num_milliseconds().max(0) as u64),
            ))
        });
        loop {
            let notification = self.notify.notified();
            tokio::pin!(notification);
            notification.as_mut().enable();
            let current = now
                + ChronoDuration::milliseconds(
                    started.elapsed().as_millis().min(i64::MAX as u128) as i64
                );
            if let Some(settlement) = self.observe(id, &snapshot, current) {
                return Ok(settlement);
            }
            match timeout.as_mut() {
                Some(timeout) => {
                    tokio::select! {
                        _ = timeout.as_mut() => {
                            let expiry = snapshot.expires_at.unwrap_or(current);
                            return Ok(self
                                .observe(id, &snapshot, expiry)
                                .unwrap_or(QuestionSettlement::Expired));
                        }
                        _ = &mut notification => {}
                    }
                }
                None => notification.await,
            }
        }
    }

    fn observe(
        &self,
        id: &str,
        snapshot: &QuestionContract,
        now: DateTime<Utc>,
    ) -> Option<QuestionSettlement> {
        let mut entries = self.guard();
        Self::expire_locked(&mut entries, now);
        match entries.get(id) {
            Some(entry)
                if entry.correlation_id == snapshot.correlation_id
                    && entry.asked_at == snapshot.asked_at =>
            {
                Self::settlement_of(entry)
            }
            _ => Some(QuestionSettlement::Expired),
        }
    }

    pub fn snapshot(&self) -> Vec<QuestionContract> {
        Self::sorted(self.guard().values().cloned())
    }

    /// Re-seed the service from a durable projection so a waiter resumes
    /// against the same logical question after a restart.
    pub fn restore(
        &self,
        contracts: impl IntoIterator<Item = QuestionContract>,
        now: DateTime<Utc>,
    ) -> usize {
        contracts
            .into_iter()
            .filter(|contract| self.register(contract.clone(), now).is_ok())
            .count()
    }

    pub fn len(&self) -> usize {
        self.guard().len()
    }

    pub fn is_empty(&self) -> bool {
        self.guard().is_empty()
    }

    fn sorted(contracts: impl IntoIterator<Item = QuestionContract>) -> Vec<QuestionContract> {
        let mut all: Vec<_> = contracts.into_iter().collect();
        all.sort_by(|left, right| (left.asked_at, &left.id).cmp(&(right.asked_at, &right.id)));
        all
    }

    fn settlement_of(entry: &QuestionContract) -> Option<QuestionSettlement> {
        match entry.state {
            QuestionState::Pending => None,
            QuestionState::Answered => Some(QuestionSettlement::Answered(
                entry.answer.clone().unwrap_or_default(),
            )),
            QuestionState::Defaulted => Some(QuestionSettlement::Defaulted(
                entry.answer.clone().unwrap_or_default(),
            )),
            QuestionState::Rejected => Some(QuestionSettlement::Rejected),
            QuestionState::Cancelled => Some(QuestionSettlement::Cancelled),
            QuestionState::Expired => Some(QuestionSettlement::Expired),
        }
    }

    fn pending_count(entries: &HashMap<String, QuestionContract>) -> usize {
        entries
            .values()
            .filter(|entry| entry.state == QuestionState::Pending)
            .count()
    }

    /// The single deadline path. A low-stakes question with an explicit safe
    /// default settles as `Defaulted` and records the default it used;
    /// everything else — every approval, every escalation — simply expires.
    fn expire_locked(entries: &mut HashMap<String, QuestionContract>, now: DateTime<Utc>) {
        for entry in entries.values_mut() {
            if entry.state != QuestionState::Pending
                || !entry.expires_at.is_some_and(|expires_at| expires_at <= now)
            {
                continue;
            }
            match entry.defaultable().map(str::to_string) {
                Some(default) => {
                    entry.state = QuestionState::Defaulted;
                    entry.answer = Some(default);
                    entry.answered_at = Some(now);
                }
                None => entry.state = QuestionState::Expired,
            }
        }
    }

    fn prune_locked(&self, entries: &mut HashMap<String, QuestionContract>) {
        let mut settled: Vec<_> = entries
            .values()
            .filter(|entry| entry.state != QuestionState::Pending)
            .map(|entry| (entry.asked_at, entry.id.clone()))
            .collect();
        let excess = settled.len().saturating_sub(self.limits.max_pending);
        if excess == 0 {
            return;
        }
        settled.sort_unstable();
        for (_, id) in settled.into_iter().take(excess) {
            entries.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_question_family_roundtrips_through_one_contract() {
        for kind in [
            QuestionKind::UserInput,
            QuestionKind::AgentParent,
            QuestionKind::BoardPlanner,
            QuestionKind::PlanApproval,
            QuestionKind::PermissionEscalation,
        ] {
            let question = QuestionContract::pending(
                "q",
                kind,
                "asker",
                "answerer",
                "correlation",
                "Continue?",
                None,
                Utc::now(),
                None,
            )
            .unwrap();
            let decoded: QuestionContract =
                serde_json::from_value(serde_json::to_value(&question).unwrap()).unwrap();
            assert_eq!(decoded, question);
        }
    }

    const KINDS: [QuestionKind; 5] = [
        QuestionKind::UserInput,
        QuestionKind::AgentParent,
        QuestionKind::BoardPlanner,
        QuestionKind::PlanApproval,
        QuestionKind::PermissionEscalation,
    ];

    fn registry() -> QuestionRegistry {
        QuestionRegistry::new(QuestionLimits::default()).unwrap()
    }

    fn ask(kind: QuestionKind, correlation: &str) -> AskQuestion {
        AskQuestion::new(kind, "asker", "answerer", correlation, "Continue?")
    }

    #[test]
    fn one_service_backs_every_kind_from_create_to_answer() {
        let registry = registry();
        let now = Utc::now();
        for (index, kind) in KINDS.into_iter().enumerate() {
            let question = registry.ask(ask(kind, &format!("c-{index}")), now).unwrap();
            assert_eq!(question.state, QuestionState::Pending);
            assert!(question.expires_at.is_some());
            let answered = registry
                .answer(&question.id, "answerer", "yes".into(), now)
                .unwrap();
            assert_eq!(answered.kind, kind);
            assert_eq!(answered.state, QuestionState::Answered);
            assert_eq!(answered.answer.as_deref(), Some("yes"));
            assert_eq!(
                registry.settlement(&question.id, now),
                Some(QuestionSettlement::Answered("yes".into()))
            );
        }
        assert_eq!(registry.list(None, now).len(), KINDS.len());
        assert_eq!(
            registry.list(Some(QuestionKind::PlanApproval), now).len(),
            1
        );
    }

    #[test]
    fn a_second_answer_is_already_answered_and_never_overwrites() {
        let registry = registry();
        let now = Utc::now();
        let question = registry
            .ask(ask(QuestionKind::UserInput, "c"), now)
            .unwrap();
        registry
            .answer(&question.id, "answerer", "first".into(), now)
            .unwrap();
        assert_eq!(
            registry.answer(&question.id, "answerer", "second".into(), now),
            Err(QuestionError::AlreadyAnswered)
        );
        assert_eq!(QuestionError::AlreadyAnswered.code(), "already_answered");
        assert_eq!(
            registry.get(&question.id).unwrap().answer.as_deref(),
            Some("first")
        );
    }

    #[test]
    fn terminal_states_are_mutually_exclusive() {
        let now = Utc::now();
        for settle in [
            QuestionState::Rejected,
            QuestionState::Cancelled,
            QuestionState::Answered,
        ] {
            let registry = registry();
            let question = registry
                .ask(ask(QuestionKind::AgentParent, "c"), now)
                .unwrap();
            match settle {
                QuestionState::Rejected => {
                    registry.reject(&question.id, "answerer", now).unwrap();
                }
                QuestionState::Cancelled => {
                    registry.cancel(&question.id, "asker", now).unwrap();
                }
                _ => {
                    registry
                        .answer(&question.id, "answerer", "ok".into(), now)
                        .unwrap();
                }
            }
            assert_eq!(registry.get(&question.id).unwrap().state, settle);
            let expected = Err(if settle == QuestionState::Answered {
                QuestionError::AlreadyAnswered
            } else {
                QuestionError::AlreadySettled(settle)
            });
            assert_eq!(
                registry.answer(&question.id, "answerer", "late".into(), now),
                expected
            );
            assert_eq!(registry.reject(&question.id, "answerer", now), expected);
            assert_eq!(registry.cancel(&question.id, "asker", now), expected);
        }
    }

    #[test]
    fn only_the_bound_principals_may_settle_a_question() {
        let registry = registry();
        let now = Utc::now();
        let question = registry
            .ask(ask(QuestionKind::BoardPlanner, "c"), now)
            .unwrap();
        assert_eq!(
            registry.answer(&question.id, "someone-else", "yes".into(), now),
            Err(QuestionError::NotAnswerer)
        );
        assert_eq!(
            registry.cancel(&question.id, "someone-else", now),
            Err(QuestionError::NotAsker)
        );
        assert_eq!(
            registry.answer("missing", "answerer", "yes".into(), now),
            Err(QuestionError::NotFound)
        );
        assert_eq!(
            registry.get(&question.id).unwrap().state,
            QuestionState::Pending
        );
    }

    #[test]
    fn expiry_moves_pending_to_expired_and_leaves_settled_questions_alone() {
        let registry = registry();
        let now = Utc::now();
        let pending = registry
            .ask(ask(QuestionKind::UserInput, "a"), now)
            .unwrap();
        let answered = registry
            .ask(ask(QuestionKind::UserInput, "b"), now)
            .unwrap();
        registry
            .answer(&answered.id, "answerer", "yes".into(), now)
            .unwrap();
        registry.expire(now + ChronoDuration::seconds(1_000_000));
        assert_eq!(
            registry.get(&pending.id).unwrap().state,
            QuestionState::Expired
        );
        assert_eq!(
            registry.get(&answered.id).unwrap().state,
            QuestionState::Answered
        );
        assert_eq!(
            registry.answer(&pending.id, "answerer", "late".into(), now),
            Err(QuestionError::AlreadySettled(QuestionState::Expired))
        );
    }

    #[test]
    fn bounds_are_rejected_not_coerced() {
        for limits in [
            QuestionLimits {
                max_pending: 0,
                ..QuestionLimits::default()
            },
            QuestionLimits {
                default_expiry_secs: 0,
                ..QuestionLimits::default()
            },
            QuestionLimits {
                max_expiry_secs: 0,
                ..QuestionLimits::default()
            },
            QuestionLimits {
                default_expiry_secs: 100,
                max_expiry_secs: 10,
                ..QuestionLimits::default()
            },
            QuestionLimits {
                max_expiry_secs: QUESTION_MAX_EXPIRY_SECS + 1,
                ..QuestionLimits::default()
            },
        ] {
            assert!(limits.validate().is_err());
            assert!(QuestionRegistry::new(limits).is_err());
        }
        let registry = QuestionRegistry::new(QuestionLimits {
            max_pending: 1,
            default_expiry_secs: 10,
            max_expiry_secs: 60,
        })
        .unwrap();
        let now = Utc::now();
        assert!(matches!(
            registry.ask(
                ask(QuestionKind::UserInput, "a").with_expiry_secs(Some(61)),
                now
            ),
            Err(QuestionError::Invalid(_))
        ));
        registry
            .ask(ask(QuestionKind::UserInput, "a"), now)
            .unwrap();
        assert_eq!(
            registry.ask(ask(QuestionKind::UserInput, "b"), now),
            Err(QuestionError::Capacity)
        );
    }

    #[test]
    fn one_pending_question_per_correlation_and_per_id() {
        let registry = registry();
        let now = Utc::now();
        let question = registry
            .ask(ask(QuestionKind::UserInput, "c"), now)
            .unwrap();
        assert_eq!(
            registry.ask(ask(QuestionKind::UserInput, "c"), now),
            Err(QuestionError::Duplicate)
        );
        assert_eq!(
            registry.ask(
                ask(QuestionKind::UserInput, "other").with_id(&question.id),
                now
            ),
            Err(QuestionError::Duplicate)
        );
        registry
            .answer(&question.id, "answerer", "yes".into(), now)
            .unwrap();
        registry
            .ask(ask(QuestionKind::UserInput, "c"), now)
            .unwrap();
    }

    #[test]
    fn restore_preserves_correlation_for_a_pending_question() {
        let first = registry();
        let now = Utc::now();
        let pending = first
            .ask(ask(QuestionKind::AgentParent, "corr"), now)
            .unwrap();
        let answered = first
            .ask(ask(QuestionKind::AgentParent, "done"), now)
            .unwrap();
        first
            .answer(&answered.id, "answerer", "yes".into(), now)
            .unwrap();
        let durable = first.snapshot();

        let restarted = registry();
        assert_eq!(restarted.restore(durable, now), 2);
        let recovered = restarted.get(&pending.id).unwrap();
        assert_eq!(recovered.correlation_id, pending.correlation_id);
        assert_eq!(recovered.state, QuestionState::Pending);
        assert_eq!(
            restarted.settlement(&answered.id, now),
            Some(QuestionSettlement::Answered("yes".into()))
        );
        restarted
            .answer(&pending.id, "answerer", "after restart".into(), now)
            .unwrap();
        assert_eq!(
            restarted.pending_for_asker("asker", now),
            Vec::<QuestionContract>::new()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_answer_racing_the_timeout_settles_exactly_once() {
        let registry = registry();
        let now = Utc::now();
        let question = registry
            .ask(
                ask(QuestionKind::PermissionEscalation, "c").with_expiry_secs(Some(1)),
                now,
            )
            .unwrap();
        let answering = registry.clone();
        let id = question.id.clone();
        let waiter = tokio::spawn(async move { registry.wait(&id, now).await });
        tokio::time::sleep(Duration::from_millis(999)).await;
        let answered = answering
            .answer(&question.id, "answerer", "approve".into(), Utc::now())
            .is_ok();
        let settled = waiter.await.unwrap().unwrap();
        assert_eq!(
            settled,
            if answered {
                QuestionSettlement::Answered("approve".into())
            } else {
                QuestionSettlement::Expired
            }
        );
        assert_eq!(
            answering.settlement(&question.id, Utc::now()),
            Some(settled)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiter_is_told_the_truth_about_expiry_cancel_and_answer() {
        for (settle, expected) in [
            (None, QuestionSettlement::Expired),
            (Some("cancel"), QuestionSettlement::Cancelled),
            (Some("reject"), QuestionSettlement::Rejected),
            (Some("answer"), QuestionSettlement::Answered("yes".into())),
        ] {
            let registry = registry();
            let now = Utc::now();
            let question = registry
                .ask(
                    ask(QuestionKind::PlanApproval, "c").with_expiry_secs(Some(2)),
                    now,
                )
                .unwrap();
            let settling = registry.clone();
            let id = question.id.clone();
            let waiter = tokio::spawn(async move { registry.wait(&id, now).await });
            if let Some(settle) = settle {
                tokio::time::sleep(Duration::from_millis(10)).await;
                match settle {
                    "cancel" => settling.cancel(&question.id, "asker", Utc::now()).unwrap(),
                    "reject" => settling
                        .reject(&question.id, "answerer", Utc::now())
                        .unwrap(),
                    _ => settling
                        .answer(&question.id, "answerer", "yes".into(), Utc::now())
                        .unwrap(),
                };
            }
            assert_eq!(waiter.await.unwrap().unwrap(), expected);
        }
    }

    #[test]
    fn a_low_stakes_timeout_defaults_and_is_never_reported_as_answered() {
        let registry = registry();
        let now = Utc::now();
        let question = registry
            .ask(
                ask(QuestionKind::UserInput, "c")
                    .with_expiry_secs(Some(60))
                    .with_default_answer(Some("keep going".into())),
                now,
            )
            .unwrap();
        assert_eq!(question.defaultable(), Some("keep going"));
        registry.expire(now + ChronoDuration::seconds(61));
        let settled = registry.get(&question.id).unwrap();
        assert_eq!(settled.state, QuestionState::Defaulted);
        assert_eq!(settled.defaulted(), Some("keep going"));
        assert_eq!(settled.answered(), None);
        let settlement = registry.settlement(&question.id, now).unwrap();
        assert_eq!(
            settlement,
            QuestionSettlement::Defaulted("keep going".into())
        );
        assert_eq!(settlement.default_used(), Some("keep going"));
        assert_eq!(settlement.answer(), None);
        assert!(!settlement.is_answered());
        assert_eq!(settlement.state(), QuestionState::Defaulted);
        assert_eq!(
            registry.answer(&question.id, "answerer", "late".into(), now),
            Err(QuestionError::AlreadySettled(QuestionState::Defaulted))
        );
        settled.validate().unwrap();
    }

    #[test]
    fn nothing_irreversible_can_carry_a_default_so_it_expires_instead() {
        let registry = registry();
        let now = Utc::now();
        for (kind, irreversible) in [
            (QuestionKind::PermissionEscalation, false),
            (QuestionKind::PlanApproval, false),
            (QuestionKind::UserInput, true),
        ] {
            let request = ask(kind, "c")
                .with_expiry_secs(Some(60))
                .irreversible(irreversible)
                .with_default_answer(Some("approve".into()));
            assert!(matches!(
                registry.ask(request, now),
                Err(QuestionError::Invalid(_))
            ));
            let question = registry
                .ask(
                    ask(kind, "c")
                        .with_expiry_secs(Some(60))
                        .irreversible(irreversible),
                    now,
                )
                .unwrap();
            assert!(question.is_irreversible());
            assert_eq!(question.defaultable(), None);
            registry.expire(now + ChronoDuration::seconds(1_000));
            assert_eq!(
                registry.get(&question.id).unwrap().state,
                QuestionState::Expired
            );
            assert_eq!(
                registry.settlement(&question.id, now),
                Some(QuestionSettlement::Expired)
            );
        }
    }

    #[test]
    fn a_defaulted_record_must_carry_the_default_it_used() {
        let base = QuestionContract::pending(
            "q",
            QuestionKind::UserInput,
            "asker",
            "answerer",
            "correlation",
            "Continue?",
            None,
            Utc::now(),
            None,
        )
        .unwrap();
        assert!(base.clone().with_stakes(false, Some("  ".into())).is_err());
        let mut defaulted = base.with_stakes(false, Some("yes".into())).unwrap();
        defaulted.state = QuestionState::Defaulted;
        assert!(defaulted.validate().is_err());
        defaulted.answer = Some("yes".into());
        defaulted.answered_at = Some(Utc::now());
        defaulted.validate().unwrap();
        let mut forged = defaulted.clone();
        forged.answer = Some("something else".into());
        assert!(forged.validate().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiter_on_a_low_stakes_question_is_told_it_defaulted() {
        let registry = registry();
        let now = Utc::now();
        let question = registry
            .ask(
                ask(QuestionKind::UserInput, "c")
                    .with_expiry_secs(Some(1))
                    .with_default_answer(Some("skip".into())),
                now,
            )
            .unwrap();
        assert_eq!(
            registry.wait(&question.id, now).await,
            Ok(QuestionSettlement::Defaulted("skip".into()))
        );
    }

    #[tokio::test]
    async fn waiting_on_an_unknown_question_is_not_found() {
        let registry = registry();
        assert_eq!(
            registry.wait("missing", Utc::now()).await,
            Err(QuestionError::NotFound)
        );
    }
}
