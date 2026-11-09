use serde::{Deserialize, Serialize};

/// Host-applied policy, deliberately separate from legacy manifest constructors.
/// A policy change is execution-bearing and requires renewed approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourcePolicy {
    /// Host-compiled canonical owner authority, never supplied by policy authors.
    pub owner_definition_digest: Option<String>,
    pub timeout_ms: u64,
    pub idle_timeout_ms: u64,
    pub stale_after_ms: u64,
    pub retry_base_ms: u64,
    pub retry_cap_ms: u64,
    /// Only enable when the upstream explicitly guarantees Last-Event-ID replay.
    pub replay_capable: bool,
    /// Explicit upstream contract: nonempty IDs on individual dispatches are unique.
    /// SSE Last-Event-ID alone does not imply uniqueness. Defaults false.
    pub unique_event_ids: bool,
    /// Explicit upstream producer incarnation; never a local worker ID.
    pub upstream_epoch: Option<String>,
    /// References only; secrets must be resolved by a host adapter, never journaled.
    pub credential_refs: Vec<String>,
    pub credential_bindings: Vec<super::CredentialBinding>,
    pub websocket_protocols: Vec<String>,
    pub websocket_subscribe: Option<serde_json::Value>,
    pub transform: Option<super::TransformSpec>,
}
impl Default for SourcePolicy {
    fn default() -> Self {
        Self {
            owner_definition_digest: None,
            timeout_ms: 15_000,
            idle_timeout_ms: 30_000,
            stale_after_ms: 60_000,
            retry_base_ms: 5_000,
            retry_cap_ms: 300_000,
            replay_capable: false,
            unique_event_ids: false,
            upstream_epoch: None,
            credential_refs: Vec::new(),
            credential_bindings: Vec::new(),
            websocket_protocols: Vec::new(),
            websocket_subscribe: None,
            transform: None,
        }
    }
}
impl SourcePolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !(100..=120_000).contains(&self.timeout_ms)
            || !(100..=600_000).contains(&self.idle_timeout_ms)
            || !(100..=86_400_000).contains(&self.stale_after_ms)
            || self.retry_base_ms < 100
            || self.retry_cap_ms < self.retry_base_ms
            || self.retry_cap_ms > 3_600_000
            || self.credential_refs.len() > 16
            || self.credential_refs.iter().any(|r| {
                r.is_empty()
                    || r.len() > 256
                    || !r
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-:./".contains(&b))
            })
        {
            return Err("invalid source policy bounds or credential reference".into());
        }
        if (self.replay_capable || self.unique_event_ids)
            && self
                .upstream_epoch
                .as_ref()
                .is_none_or(|s| s.is_empty() || s.len() > 256)
        {
            return Err("replay requires an explicit stable upstream producer incarnation".into());
        }
        if self.credential_bindings.len() > 16 || self.websocket_protocols.len() > 8 {
            return Err("source protocol policy limit".into());
        }
        let mut headers = std::collections::HashSet::new();
        for binding in &self.credential_bindings {
            super::credentials::validate_binding(binding)?;
            if !headers.insert(binding.header.to_ascii_lowercase()) {
                return Err("duplicate credential header".into());
            }
        }
        if self
            .credential_refs
            .iter()
            .any(|r| !self.credential_bindings.iter().any(|b| &b.reference == r))
        {
            return Err("credential reference requires explicit header binding".into());
        }
        for protocol in &self.websocket_protocols {
            if protocol.is_empty()
                || protocol.len() > 128
                || !protocol
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            {
                return Err("invalid websocket subprotocol".into());
            }
        }
        if self.websocket_subscribe.as_ref().is_some_and(|v| {
            !v.is_object() || serde_json::to_vec(v).map_or(true, |b| b.len() > 16384)
        }) {
            return Err("websocket subscribe must be bounded public JSON object".into());
        }
        if let Some(spec) = &self.transform {
            spec.validate()?;
        }
        Ok(())
    }
    pub(crate) fn retry_delay(&self, failures: u32, retry_after_ms: Option<u64>) -> u64 {
        let base = self
            .retry_base_ms
            .saturating_mul(1u64 << failures.saturating_sub(1).min(16));
        // Positive bounded jitter avoids synchronizing project collectors.
        let jitter = (uuid::Uuid::new_v4().as_u128() % (base / 4 + 1) as u128) as u64;
        base.saturating_add(jitter)
            .max(retry_after_ms.unwrap_or(0))
            .min(self.retry_cap_ms)
    }
}
