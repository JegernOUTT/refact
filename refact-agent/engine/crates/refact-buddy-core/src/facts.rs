use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

use crate::types::{BuddyFact, BuddyFactKind};

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
                if self
                    .ring
                    .get(idx)
                    .map(|f| f.key == fact.key)
                    .unwrap_or(false)
                {
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
            entry.kind = fact.kind;
            entry.source = fact.source;
            entry.payload = fact.payload;
            entry.seen_at = fact.seen_at;
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

    pub fn ingest_with_refresh_ttl(&mut self, fact: BuddyFact, refresh_ttl: Duration) -> bool {
        if refresh_ttl > Duration::zero() {
            if let Some(existing) = self.ring.iter_mut().find(|f| f.key == fact.key) {
                if fact.seen_at.signed_duration_since(existing.seen_at) < refresh_ttl {
                    existing.payload = fact.payload;
                    existing.confidence = fact.confidence;
                    return false;
                }
            }
        }
        self.ingest(fact);
        true
    }

    pub fn ingest_many_with_refresh_ttl(
        &mut self,
        facts: Vec<BuddyFact>,
        refresh_ttl: Duration,
    ) -> usize {
        let mut accepted = 0usize;
        for fact in facts {
            if self.ingest_with_refresh_ttl(fact, refresh_ttl) {
                accepted += 1;
            }
        }
        accepted
    }

    /// Return references to all facts of `kind` seen within `within` of now.
    pub fn recent(&self, kind: BuddyFactKind, within: Duration) -> Vec<&BuddyFact> {
        self.recent_at(kind, within, Utc::now())
    }

    pub fn recent_at(
        &self,
        kind: BuddyFactKind,
        within: Duration,
        now: DateTime<Utc>,
    ) -> Vec<&BuddyFact> {
        let cutoff = now - within;
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
}

impl Default for FactStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(key: &str, seen_at: DateTime<Utc>, count: u64) -> BuddyFact {
        BuddyFact {
            kind: BuddyFactKind::DiagnosticCluster,
            key: key.to_string(),
            source: "test",
            payload: serde_json::json!({ "count": count }),
            seen_at,
            confidence: 0.9,
        }
    }

    #[test]
    fn refresh_ttl_suppresses_reemission_but_updates_payload() {
        let mut store = FactStore::new();
        let t0 = Utc::now();
        let ttl = Duration::minutes(30);

        assert!(store.ingest_with_refresh_ttl(fact("diag:x", t0, 3), ttl));
        assert!(!store.ingest_with_refresh_ttl(fact("diag:x", t0 + Duration::minutes(1), 5), ttl));
        assert!(!store.ingest_with_refresh_ttl(fact("diag:x", t0 + Duration::minutes(29), 9), ttl));

        let stored = store.iter().find(|f| f.key == "diag:x").unwrap();
        assert_eq!(stored.seen_at, t0);
        assert_eq!(stored.payload["count"], 9);

        assert!(store.ingest_with_refresh_ttl(fact("diag:x", t0 + Duration::minutes(31), 11), ttl));
        let stored = store.iter().find(|f| f.key == "diag:x").unwrap();
        assert_eq!(stored.seen_at, t0 + Duration::minutes(31));
        assert_eq!(stored.payload["count"], 11);
    }

    #[test]
    fn zero_ttl_keeps_legacy_refresh_behavior() {
        let mut store = FactStore::new();
        let t0 = Utc::now();

        assert!(store.ingest_with_refresh_ttl(fact("diag:y", t0, 1), Duration::zero()));
        assert!(store.ingest_with_refresh_ttl(
            fact("diag:y", t0 + Duration::seconds(1), 2),
            Duration::zero()
        ));
        let stored = store.iter().find(|f| f.key == "diag:y").unwrap();
        assert_eq!(stored.payload["count"], 2);
    }

    #[test]
    fn ttl_gate_tracks_distinct_keys_independently() {
        let mut store = FactStore::new();
        let t0 = Utc::now();
        let ttl = Duration::minutes(30);

        assert!(store.ingest_with_refresh_ttl(fact("diag:a", t0, 1), ttl));
        assert!(store.ingest_with_refresh_ttl(fact("diag:b", t0 + Duration::minutes(1), 1), ttl));
        assert_eq!(
            store.ingest_many_with_refresh_ttl(
                vec![
                    fact("diag:a", t0 + Duration::minutes(2), 2),
                    fact("diag:c", t0 + Duration::minutes(2), 1),
                ],
                ttl,
            ),
            1
        );
    }
}
