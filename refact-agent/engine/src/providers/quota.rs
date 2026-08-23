use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Number;

pub const PROVIDER_QUOTA_CACHE_TTL_SECONDS: u64 = 60;
pub const PROVIDER_QUOTA_CACHE_CAPACITY: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderQuotaSource {
    ClaudeCode,
    OpenaiCodex,
    Opencode,
    GoogleAntigravity,
    XaiOauth,
    Litellm,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderQuotaWindow {
    pub id: String,
    pub label: String,
    pub used_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_after_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProviderQuotaFactValue {
    String(String),
    Number(Number),
    Boolean(bool),
    Null,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderQuotaFact {
    pub id: String,
    pub label: String,
    pub value: ProviderQuotaFactValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderQuotaSnapshot {
    pub provider_name: String,
    pub base_provider: String,
    pub source: ProviderQuotaSource,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default)]
    pub windows: Vec<ProviderQuotaWindow>,
    #[serde(default)]
    pub facts: Vec<ProviderQuotaFact>,
    pub fetched_at: String,
    pub stale: bool,
}

impl ProviderQuotaSnapshot {
    pub fn unavailable(provider: impl Into<String>, base_provider: impl Into<String>) -> Self {
        Self {
            provider_name: provider.into(),
            base_provider: base_provider.into(),
            source: ProviderQuotaSource::Unavailable,
            available: false,
            error: None,
            plan: None,
            windows: Vec::new(),
            facts: Vec::new(),
            fetched_at: ProviderQuotaCache::now_iso_timestamp(),
            stale: false,
        }
    }

    pub fn error(
        provider: impl Into<String>,
        base_provider: impl Into<String>,
        source: ProviderQuotaSource,
        error: impl Into<String>,
    ) -> Self {
        Self {
            provider_name: provider.into(),
            base_provider: base_provider.into(),
            source,
            available: false,
            error: Some(error.into()),
            plan: None,
            windows: Vec::new(),
            facts: Vec::new(),
            fetched_at: ProviderQuotaCache::now_iso_timestamp(),
            stale: false,
        }
    }
}

#[derive(Debug, Clone)]
struct ProviderQuotaCacheEntry {
    inserted_at: u64,
    snapshot: ProviderQuotaSnapshot,
}

#[derive(Debug, Clone)]
pub struct ProviderQuotaCache {
    entries: HashMap<String, ProviderQuotaCacheEntry>,
    capacity: usize,
    ttl_seconds: u64,
}

impl Default for ProviderQuotaCache {
    fn default() -> Self {
        Self::new(
            PROVIDER_QUOTA_CACHE_CAPACITY,
            PROVIDER_QUOTA_CACHE_TTL_SECONDS,
        )
    }
}

impl ProviderQuotaCache {
    pub fn new(capacity: usize, ttl_seconds: u64) -> Self {
        Self {
            entries: HashMap::new(),
            capacity: capacity.max(1),
            ttl_seconds,
        }
    }

    pub fn now_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    }

    pub fn now_iso_timestamp() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    pub fn get_fresh(&self, provider: &str, now: u64) -> Option<ProviderQuotaSnapshot> {
        self.entries.get(provider).and_then(|entry| {
            now.saturating_sub(entry.inserted_at)
                .lt(&self.ttl_seconds)
                .then(|| entry.snapshot.clone())
        })
    }

    pub fn get_stale(&self, provider: &str) -> Option<ProviderQuotaSnapshot> {
        self.entries
            .get(provider)
            .map(|entry| entry.snapshot.clone())
    }

    pub fn invalidate(&mut self, provider: &str) -> Option<ProviderQuotaSnapshot> {
        self.entries.remove(provider).map(|entry| entry.snapshot)
    }

    pub fn insert_at(&mut self, provider: String, snapshot: ProviderQuotaSnapshot, now: u64) {
        if !self.entries.contains_key(&provider) && self.entries.len() >= self.capacity {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.inserted_at)
                .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(
            provider,
            ProviderQuotaCacheEntry {
                inserted_at: now,
                snapshot,
            },
        );
    }

    pub fn insert(&mut self, provider: String, snapshot: ProviderQuotaSnapshot) {
        self.insert_at(provider, snapshot, Self::now_timestamp());
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_respects_refresh_age_and_capacity() {
        let mut cache = ProviderQuotaCache::new(2, 60);
        cache.insert_at(
            "one".to_string(),
            ProviderQuotaSnapshot::unavailable("one", "x"),
            10,
        );
        cache.insert_at(
            "two".to_string(),
            ProviderQuotaSnapshot::unavailable("two", "x"),
            20,
        );
        assert!(cache.get_fresh("one", 69).is_some());
        assert!(cache.get_fresh("one", 70).is_none());

        cache.insert_at(
            "three".to_string(),
            ProviderQuotaSnapshot::unavailable("three", "x"),
            30,
        );
        assert_eq!(cache.len(), 2);
        assert!(cache.get_fresh("one", 30).is_none());
        assert!(cache.get_fresh("two", 30).is_some());
    }

    #[test]
    fn unavailable_snapshot_has_explicit_state() {
        let snapshot = ProviderQuotaSnapshot::unavailable("anthropic-work", "anthropic");
        assert!(!snapshot.available);
        assert_eq!(snapshot.source, ProviderQuotaSource::Unavailable);
        assert!(snapshot.error.is_none());
    }

    #[test]
    fn snapshot_serializes_gui_required_fields() {
        let snapshot = ProviderQuotaSnapshot::unavailable("xai-work", "xai_oauth");
        let value = serde_json::to_value(snapshot).unwrap();

        assert_eq!(value["provider_name"], "xai-work");
        assert_eq!(value["base_provider"], "xai_oauth");
        assert_eq!(value["source"], "unavailable");
        assert_eq!(value["available"], false);
        assert_eq!(value["stale"], false);
        assert!(value["fetched_at"]
            .as_str()
            .is_some_and(|date| date.contains('T')));
        assert!(value["windows"].is_array());
        assert!(value["facts"].is_array());
        assert!(value.get("error").is_none());
        assert!(value.get("details").is_none());
    }

    #[test]
    fn cache_supports_stale_reads_and_invalidation() {
        let mut cache = ProviderQuotaCache::new(2, 1);
        cache.insert_at(
            "one".to_string(),
            ProviderQuotaSnapshot::unavailable("one", "x"),
            10,
        );

        assert!(cache.get_fresh("one", 11).is_none());
        assert!(cache.get_stale("one").is_some());
        assert!(cache.invalidate("one").is_some());
        assert!(cache.get_stale("one").is_none());
    }
}
