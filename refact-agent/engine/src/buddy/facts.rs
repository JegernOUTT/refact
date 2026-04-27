use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

use crate::buddy::types::{BuddyFact, BuddyFactKind};

pub const FACT_RING_CAPACITY: usize = 1000;

/// Ring-buffered store of `BuddyFact` values with key-based deduplication.
///
/// The `by_key` map is a best-effort index hint. It may hold stale entries after
/// evictions; always validate the hint before trusting it.
pub struct FactStore {
    ring: std::collections::VecDeque<BuddyFact>,
    by_key: HashMap<String, usize>,
}

impl FactStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            ring: std::collections::VecDeque::new(),
            by_key: HashMap::new(),
        }
    }

    /// Ingest one fact, replacing any existing fact with the same key in-place.
    ///
    /// When the ring is full the oldest entry is evicted first.
    pub fn ingest(&mut self, fact: BuddyFact) {
        let existing_pos: Option<usize> = {
            let hint = self.by_key.get(&fact.key).copied();
            if let Some(idx) = hint {
                if self.ring.get(idx).map(|f| f.key == fact.key).unwrap_or(false) {
                    Some(idx)
                } else {
                    self.ring.iter().position(|f| f.key == fact.key)
                }
            } else {
                self.ring.iter().position(|f| f.key == fact.key)
            }
        };

        if let Some(pos) = existing_pos {
            let key = fact.key.clone();
            let entry = &mut self.ring[pos];
            entry.seen_at = fact.seen_at;
            entry.payload = fact.payload;
            entry.confidence = fact.confidence;
            self.by_key.insert(key, pos);
            return;
        }

        if self.ring.len() >= FACT_RING_CAPACITY {
            if let Some(evicted) = self.ring.pop_front() {
                self.by_key.remove(&evicted.key);
            }
            self.by_key.clear();
            for (i, f) in self.ring.iter().enumerate() {
                self.by_key.insert(f.key.clone(), i);
            }
        }

        let idx = self.ring.len();
        self.by_key.insert(fact.key.clone(), idx);
        self.ring.push_back(fact);
    }

    /// Ingest multiple facts.
    pub fn ingest_many(&mut self, facts: Vec<BuddyFact>) {
        for fact in facts {
            self.ingest(fact);
        }
    }

    /// Return references to all facts of `kind` seen within `within` of now.
    pub fn recent(&self, kind: BuddyFactKind, within: Duration) -> Vec<&BuddyFact> {
        let cutoff = Utc::now() - within;
        self.ring
            .iter()
            .filter(|f| f.kind == kind && f.seen_at >= cutoff)
            .collect()
    }

    /// Count facts of `kind` seen within `within` of now.
    pub fn count_within(&self, kind: BuddyFactKind, within: Duration) -> usize {
        self.recent(kind, within).len()
    }

    /// Iterate over all facts in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &BuddyFact> {
        self.ring.iter()
    }

    /// Number of facts currently stored.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Remove all facts whose `seen_at` is older than `max_age` from now.
    pub fn clear_older_than(&mut self, max_age: Duration) {
        let cutoff = Utc::now() - max_age;
        self.ring.retain(|f| f.seen_at >= cutoff);
        self.by_key.clear();
        for (idx, fact) in self.ring.iter().enumerate() {
            self.by_key.insert(fact.key.clone(), idx);
        }
    }
}

impl Default for FactStore {
    fn default() -> Self {
        Self::new()
    }
}
