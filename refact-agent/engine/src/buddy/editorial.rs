use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use refact_buddy_core::operator_types::ProjectIdentity;
use super::operator_store::{OperatorStore, OperatorStoreError};

/// Suppress legacy eager commentary when the shared editorial rollout is enabled.
/// Native approvals and durable warnings do not depend on this cosmetic gate.
pub async fn enabled(app: &crate::app_state::AppState) -> bool {
    app.buddy
        .buddy
        .lock()
        .await
        .as_ref()
        .is_some_and(|svc| svc.operator_features.editorial)
}

pub const MAX_RECEIPTS: usize = 256;
pub const RETENTION_SECONDS: i64 = 7 * 86400;
const NAMESPACE: &str = "editorial";
const STATE_ID: &str = "attention-v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    pub intent: String,
    pub topic: String,
    pub evidence: String,
    pub created_at: i64,
    pub expires_at: i64,
}

pub fn fingerprint(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

impl Candidate {
    pub fn new(intent: &str, topic: &str, evidence: &str, now: i64) -> Self {
        let topic = fingerprint(&[intent, topic]);
        let evidence = fingerprint(&[evidence]);
        Self {
            id: fingerprint(&[intent, &topic, &evidence]),
            intent: intent.into(),
            topic,
            evidence,
            created_at: now,
            expires_at: now.saturating_add(300),
        }
    }
    fn valid(&self) -> bool {
        [
            self.id.as_str(),
            self.topic.as_str(),
            self.evidence.as_str(),
        ]
        .iter()
        .all(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
            && matches!(
                self.intent.as_str(),
                "humor"
                    | "suggestion"
                    | "insight"
                    | "win"
                    | "error_alert"
                    | "greeting"
                    | "tour"
                    | "milestone"
                    | "memory_pulse_commentary"
                    | "quest_accept"
                    | "quest_complete"
                    | "chat_reaction"
            )
            && self.expires_at > self.created_at
            && self.expires_at.saturating_sub(self.created_at) <= 300
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Feedback {
    Useful,
    Noisy,
    LessLikeThis,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Panel,
    Avatar,
    Notification,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Eligible,
    Invalid,
    Expired,
    Duplicate,
    TopicBudget,
    AttentionBudget,
    FeedbackSuppressed,
    Capacity,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Counters {
    pub generated: u64,
    pub eligible: u64,
    pub selected: u64,
    pub delivered: u64,
    pub skipped: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub candidate: Candidate,
    pub selected_at: i64,
    pub speech_id: Option<String>,
    pub claimed_surface: Option<Surface>,
    pub feedback: Option<Feedback>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorialState {
    pub schema_version: u32,
    pub project: ProjectIdentity,
    pub counters: Counters,
    pub receipts: Vec<Receipt>,
}
impl EditorialState {
    pub fn new(project: ProjectIdentity) -> Self {
        Self {
            schema_version: 1,
            project,
            counters: Counters::default(),
            receipts: vec![],
        }
    }
    pub fn prune(&mut self, now: i64) {
        self.receipts
            .retain(|r| now.saturating_sub(r.selected_at) < RETENTION_SECONDS);
    }
    pub fn evaluate(&self, candidate: &Candidate, now: i64) -> Decision {
        if !candidate.valid() {
            return Decision::Invalid;
        }
        if now < candidate.created_at || now >= candidate.expires_at {
            return Decision::Expired;
        }
        let recent: Vec<_> = self
            .receipts
            .iter()
            .filter(|r| now.saturating_sub(r.selected_at) < RETENTION_SECONDS)
            .collect();
        if recent
            .iter()
            .any(|r| r.candidate.id == candidate.id || r.candidate.evidence == candidate.evidence)
        {
            return Decision::Duplicate;
        }
        if recent.iter().any(|r| {
            r.candidate.topic == candidate.topic
                && match r.feedback {
                    Some(Feedback::LessLikeThis) => true,
                    Some(Feedback::Noisy) => now.saturating_sub(r.selected_at) < 86400,
                    _ => false,
                }
        }) {
            return Decision::FeedbackSuppressed;
        }
        if recent
            .iter()
            .filter(|r| {
                r.candidate.topic == candidate.topic && now.saturating_sub(r.selected_at) < 3600
            })
            .count()
            >= 2
        {
            return Decision::TopicBudget;
        }
        if recent
            .iter()
            .filter(|r| now.saturating_sub(r.selected_at) < 3600)
            .count()
            >= 6
        {
            return Decision::AttentionBudget;
        }
        if recent.len() >= MAX_RECEIPTS {
            return Decision::Capacity;
        }
        Decision::Eligible
    }
    pub fn reserve(&mut self, candidate: Candidate, now: i64) -> Decision {
        self.prune(now);
        let decision = self.evaluate(&candidate, now);
        if decision == Decision::Eligible {
            self.counters.selected = self.counters.selected.saturating_add(1);
            self.receipts.push(Receipt {
                candidate,
                selected_at: now,
                speech_id: None,
                claimed_surface: None,
                feedback: None,
            });
        }
        decision
    }
    pub fn publish(
        &mut self,
        id: &str,
        speech_id: &str,
        now: i64,
    ) -> Result<(), OperatorStoreError> {
        if speech_id.is_empty() || speech_id.len() > 128 {
            return Err(OperatorStoreError::InvalidInput("invalid speech identity"));
        }
        if self
            .receipts
            .iter()
            .any(|r| r.candidate.id != id && r.speech_id.as_deref() == Some(speech_id))
        {
            return Err(OperatorStoreError::Conflict(
                "speech identity already published",
            ));
        }
        let receipt = self
            .receipts
            .iter_mut()
            .find(|r| r.candidate.id == id)
            .ok_or(OperatorStoreError::Conflict(
                "editorial selection not found",
            ))?;
        if receipt.candidate.expires_at <= now {
            return Err(OperatorStoreError::Conflict("editorial selection expired"));
        }
        if receipt
            .speech_id
            .as_deref()
            .is_some_and(|old| old != speech_id)
        {
            return Err(OperatorStoreError::Conflict("speech already published"));
        }
        receipt.speech_id = Some(speech_id.into());
        Ok(())
    }
    pub fn claim(
        &mut self,
        speech_id: &str,
        surface: Surface,
        now: i64,
    ) -> Result<bool, OperatorStoreError> {
        let receipt = self
            .receipts
            .iter_mut()
            .find(|r| r.speech_id.as_deref() == Some(speech_id))
            .ok_or(OperatorStoreError::Conflict("published speech not found"))?;
        if now >= receipt.candidate.expires_at || receipt.claimed_surface.is_some() {
            return Ok(false);
        }
        receipt.claimed_surface = Some(surface);
        self.counters.delivered = self.counters.delivered.saturating_add(1);
        Ok(true)
    }
    pub fn feedback(
        &mut self,
        speech_id: &str,
        feedback: Feedback,
        now: i64,
    ) -> Result<(), OperatorStoreError> {
        self.prune(now);
        let receipt = self
            .receipts
            .iter_mut()
            .find(|r| r.speech_id.as_deref() == Some(speech_id) && r.claimed_surface.is_some())
            .ok_or(OperatorStoreError::Conflict("delivered speech not found"))?;
        receipt.feedback = Some(feedback);
        Ok(())
    }
}

pub fn with_state<T>(
    store: &mut OperatorStore,
    f: impl FnOnce(&mut EditorialState) -> Result<T, OperatorStoreError>,
) -> Result<T, OperatorStoreError> {
    let saved = store.get_document::<EditorialState>(NAMESPACE, STATE_ID)?;
    let revision = saved.as_ref().map(|v| v.revision);
    let mut state = saved
        .map(|v| v.value)
        .unwrap_or_else(|| EditorialState::new(store.identity().clone()));
    if state.schema_version != 1
        || state.project != *store.identity()
        || state.receipts.len() > MAX_RECEIPTS
        || state.receipts.iter().any(|r| !r.candidate.valid())
    {
        return Err(OperatorStoreError::InvalidInput(
            "unsupported editorial state",
        ));
    }
    let previous_ids: std::collections::BTreeSet<_> = state
        .receipts
        .iter()
        .map(|r| r.candidate.id.clone())
        .collect();
    let result = f(&mut state)?;
    if state
        .receipts
        .iter()
        .any(|r| !previous_ids.contains(&r.candidate.id))
    {
        store.admit_editorial_selection()?;
    }
    store.put_editorial_document(&state, revision)?;
    Ok(result)
}

pub fn snapshot(store: &OperatorStore) -> Result<Counters, OperatorStoreError> {
    Ok(store
        .get_document::<EditorialState>(NAMESPACE, STATE_ID)?
        .map(|s| s.value.counters)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> EditorialState {
        EditorialState::new(ProjectIdentity {
            project_id: "p".into(),
            canonical_root: "/fixture".into(),
        })
    }
    #[test]
    fn claims_are_single_across_clients_and_surfaces_and_not_permissions() {
        let mut state = state();
        let c = Candidate::new("insight", "topic", "evidence", 100);
        assert_eq!(state.reserve(c.clone(), 100), Decision::Eligible);
        state.publish(&c.id, "speech", 101).unwrap();
        assert!(state.claim("speech", Surface::Panel, 102).unwrap());
        assert!(!state.claim("speech", Surface::Avatar, 102).unwrap());
        assert!(!state.claim("speech", Surface::Panel, 102).unwrap());
        assert_eq!(state.counters.delivered, 1);
        state
            .feedback("speech", Feedback::LessLikeThis, 103)
            .unwrap();
        assert_eq!(
            state.evaluate(
                &Candidate::new("insight", "topic", "new evidence", 103),
                103
            ),
            Decision::FeedbackSuppressed
        );
    }
    #[test]
    fn publication_cannot_reuse_another_candidates_delivery_identity() {
        let mut state = state();
        let a = Candidate::new("win", "a", "a", 100);
        let b = Candidate::new("win", "b", "b", 100);
        assert_eq!(state.reserve(a.clone(), 100), Decision::Eligible);
        assert_eq!(state.reserve(b.clone(), 100), Decision::Eligible);
        state.publish(&a.id, "shared", 100).unwrap();
        assert!(state.publish(&b.id, "shared", 100).is_err());
        assert!(state.claim("shared", Surface::Panel, 100).unwrap());
        assert!(!state.claim("shared", Surface::Notification, 100).unwrap());
    }

    #[test]
    fn privacy_and_expiry_and_useful_never_raise_budget() {
        let mut state = state();
        for n in 0..2 {
            let c = Candidate::new(
                "insight",
                "private topic",
                &format!("private evidence {n}"),
                100,
            );
            assert_eq!(state.reserve(c.clone(), 100), Decision::Eligible);
            let id = format!("s{n}");
            state.publish(&c.id, &id, 101).unwrap();
            assert!(state.claim(&id, Surface::Panel, 101).unwrap());
            state.feedback(&id, Feedback::Useful, 101).unwrap();
        }
        assert_eq!(
            state.evaluate(&Candidate::new("insight", "private topic", "new", 102), 102),
            Decision::TopicBudget
        );
        let encoded = serde_json::to_string(&state).unwrap();
        assert!(!encoded.contains("private"));
        let c = Candidate::new("win", "new", "new", 100);
        assert_eq!(state.evaluate(&c, 400), Decision::Expired);
        assert_eq!(state.evaluate(&c, 99), Decision::Expired);
    }
    #[test]
    fn evidence_novelty_crosses_topics_and_retention_is_bounded() {
        let mut state = state();
        let candidate = Candidate::new("insight", "one", "same evidence", 100);
        state.reserve(candidate, 100);
        assert_eq!(
            state.evaluate(&Candidate::new("win", "two", "same evidence", 101), 101),
            Decision::Duplicate
        );
        state.prune(100 + RETENTION_SECONDS);
        assert!(state.receipts.is_empty());
        assert_eq!(
            state.evaluate(
                &Candidate::new("win", "two", "same evidence", 100 + RETENTION_SECONDS),
                100 + RETENTION_SECONDS
            ),
            Decision::Eligible
        );
    }
    #[test]
    fn counters_saturate_and_unpublished_or_expired_claims_do_not_deliver() {
        let mut state = state();
        state.counters.selected = u64::MAX;
        let c = Candidate::new("win", "topic", "evidence", 100);
        state.reserve(c.clone(), 100);
        assert_eq!(state.counters.selected, u64::MAX);
        assert!(state.claim("missing", Surface::Panel, 100).is_err());
        assert!(state.feedback("missing", Feedback::Useful, 100).is_err());
        state.publish(&c.id, "s", 100).unwrap();
        assert!(!state.claim("s", Surface::Panel, 400).unwrap());
        assert_eq!(state.counters.delivered, 0);
    }

    #[test]
    fn durable_claims_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "p".into(),
            canonical_root: dunce::canonicalize(dir.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        let c = Candidate::new("win", "topic", "evidence", 100);
        with_state(&mut store, |s| {
            s.reserve(c.clone(), 100);
            s.publish(&c.id, "s", 100)?;
            assert!(s.claim("s", Surface::Panel, 100)?);
            Ok(())
        })
        .unwrap();
        drop(store);
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        assert!(!with_state(&mut store, |s| s.claim("s", Surface::Avatar, 101)).unwrap());
    }
}
