use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

use crate::buddy::types::{BuddyDraft, DraftKind};

pub const DRAFT_TTL: Duration = Duration::hours(2);

/// In-memory store for short-lived `BuddyDraft` values.
pub struct DraftStore {
    drafts: HashMap<String, BuddyDraft>,
}

impl DraftStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            drafts: HashMap::new(),
        }
    }

    /// Create a new draft, mint a UUID, set TTL, and return a clone.
    pub fn create(
        &mut self,
        kind: DraftKind,
        title: String,
        yaml_or_json: String,
        explanation: String,
    ) -> BuddyDraft {
        let now = Utc::now();
        let draft = BuddyDraft {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            title,
            yaml_or_json,
            explanation,
            created_at: now,
            expires_at: now + DRAFT_TTL,
        };
        self.drafts.insert(draft.id.clone(), draft.clone());
        draft
    }

    /// Look up a draft by id.
    pub fn get(&self, id: &str) -> Option<&BuddyDraft> {
        self.drafts.get(id)
    }

    /// Remove and return a draft by id.
    pub fn delete(&mut self, id: &str) -> Option<BuddyDraft> {
        self.drafts.remove(id)
    }

    /// Consume (delete and return) a draft by id.
    pub fn consume(&mut self, id: &str) -> Option<BuddyDraft> {
        self.drafts.remove(id)
    }

    /// Remove all drafts whose TTL has passed relative to `now`.
    pub fn expire_old(&mut self, now: DateTime<Utc>) {
        self.drafts.retain(|_, d| d.expires_at > now);
    }

    /// Clone all drafts for inclusion in a snapshot.
    pub fn snapshot(&self) -> Vec<BuddyDraft> {
        self.drafts.values().cloned().collect()
    }
}

impl Default for DraftStore {
    fn default() -> Self {
        Self::new()
    }
}
