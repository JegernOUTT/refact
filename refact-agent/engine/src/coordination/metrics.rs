//! Structured metrics and the auditable explanation log (M10).
//!
//! Two things live here. **Series** measure the pressure points this program
//! already has: queue latency, permits, provider pressure and its AIMD state,
//! unknown file attribution, stale-write rejections, delivery lag, duplicates
//! and expiry, abandoned claims, recovery decisions, evidence cache validity,
//! causal-trace completeness and context payload size. **Explanations** record
//! why an automatic decision was taken: the decision, the inputs that produced
//! it and the rule that fired.
//!
//! **Never fails, never delays.** Recording is an in-memory map update behind a
//! mutex whose poisoning is recovered, exactly as O-3a's causal recorder
//! behaves. There is no I/O and no await on any path, so a recorder that is
//! broken, poisoned or full cannot fail or slow the operation it measures.
//!
//! **States stay distinct.** M5 built six evidence verdicts precisely so that
//! `failed`, `blocked`, `unavailable` and `not_run` would stop being the same
//! word. One series per state; nothing here collapses them.
//!
//! **Secrets do not reach a series.** Label names come from a fixed allowlist,
//! and every value — label or explanation input — passes redaction, which
//! replaces a secret-shaped value with a non-reversible fingerprint. File
//! content never becomes a label, and paths are counted by outcome rather than
//! by name, which bounds cardinality and keeps the path out of the export.
//!
//! **Bounded.** Series cardinality and explanation retention are capped from
//! coordination settings. A new series past the cap is folded into one overflow
//! counter: an unbounded label set is a memory leak with a dashboard.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Label names safe to export. A label outside this set is dropped rather than
/// guessed at, because an arbitrary key is how an environment value escapes.
pub const ALLOWED_LABELS: &[&str] = &[
    "bucket", "class", "decision", "kind", "outcome", "priority", "reason", "result", "source",
    "state", "verdict",
];

pub const EVIDENCE_VERDICTS: &str = "evidence_verdicts";
pub const EVIDENCE_CACHE: &str = "evidence_cache";
pub const QUEUE_LATENCY_MS: &str = "queue_latency_ms";
pub const PERMITS: &str = "permits";
pub const PROVIDER_PRESSURE: &str = "provider_pressure";
pub const PROVIDER_AIMD: &str = "provider_aimd";
pub const UNKNOWN_FILE_ATTRIBUTION: &str = "unknown_file_attribution";
pub const STALE_WRITE_REJECTIONS: &str = "stale_write_rejections";
pub const DELIVERY_LAG_MS: &str = "delivery_lag_ms";
pub const DELIVERY_DUPLICATES: &str = "delivery_duplicates";
pub const DELIVERY_EXPIRY: &str = "delivery_expiry";
pub const ABANDONED_CLAIMS: &str = "abandoned_claims";
pub const RECOVERY_DECISIONS: &str = "recovery_decisions";
pub const CAUSAL_TRACE_COMPLETENESS: &str = "causal_trace_completeness";
pub const CONTEXT_PAYLOAD_BYTES: &str = "context_payload_bytes";
pub const DECISIONS: &str = "decisions";

/// Series the recorder keeps about itself, so a dropped label or an evicted
/// explanation is visible instead of silent.
pub const SERIES_DROPPED: &str = "metrics_series_dropped";
pub const LABELS_DROPPED: &str = "metrics_labels_dropped";
pub const EXPLANATIONS_DROPPED: &str = "metrics_explanations_dropped";
pub const EXPLANATIONS_INCOMPLETE: &str = "metrics_explanations_incomplete";

/// The automatic decisions that must be explainable. Each one names a place
/// where this program decides something without asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    AutomaticPause,
    Denial,
    Retry,
    MergeRefusal,
    BackpressureDeferral,
    GoalCompletion,
    GoalRefusal,
}

impl Decision {
    pub fn label(self) -> &'static str {
        match self {
            Decision::AutomaticPause => "automatic_pause",
            Decision::Denial => "denial",
            Decision::Retry => "retry",
            Decision::MergeRefusal => "merge_refusal",
            Decision::BackpressureDeferral => "backpressure_deferral",
            Decision::GoalCompletion => "goal_completion",
            Decision::GoalRefusal => "goal_refusal",
        }
    }
}

/// One automatic decision, with the inputs that produced it and the rule that
/// fired. A decision missing either is not an explanation and is refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Explanation {
    pub decision: Decision,
    pub outcome: String,
    pub rule: String,
    pub inputs: Vec<(String, String)>,
    pub seq: u64,
    pub at_ms: i64,
}

impl Explanation {
    pub fn new(
        decision: Decision,
        outcome: impl Into<String>,
        rule: impl Into<String>,
        inputs: Vec<(String, String)>,
    ) -> Self {
        Explanation {
            decision,
            outcome: outcome.into(),
            rule: rule.into(),
            inputs,
            seq: 0,
            at_ms: 0,
        }
    }

    /// An explanation that names its rule and at least one input. "Denied" with
    /// neither is a verdict, not an explanation.
    pub fn is_complete(&self) -> bool {
        !self.rule.trim().is_empty()
            && !self.outcome.trim().is_empty()
            && self
                .inputs
                .iter()
                .any(|(name, value)| !name.trim().is_empty() && !value.trim().is_empty())
    }

    fn redacted(mut self, max_chars: usize) -> Self {
        self.outcome = redact(&self.outcome, max_chars);
        self.rule = redact(&self.rule, max_chars);
        self.inputs = self
            .inputs
            .into_iter()
            .map(|(name, value)| (redact(&name, max_chars), redact(&value, max_chars)))
            .collect();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SeriesKey {
    pub name: &'static str,
    pub labels: Vec<(&'static str, String)>,
}

impl SeriesKey {
    pub fn new(name: &'static str, labels: Vec<(&'static str, String)>) -> Self {
        SeriesKey { name, labels }
    }

    pub fn label(&self, name: &str) -> Option<&str> {
        self.labels
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Sample {
    Counter {
        value: u64,
    },
    Gauge {
        value: i64,
    },
    Observation {
        count: u64,
        sum: i64,
        min: i64,
        max: i64,
    },
}

impl Sample {
    pub fn counter(self) -> u64 {
        match self {
            Sample::Counter { value } => value,
            Sample::Gauge { value } => value.max(0) as u64,
            Sample::Observation { count, .. } => count,
        }
    }
}

/// Values that look like a credential are replaced by a stable fingerprint.
/// Prefix matching catches the well-known token shapes; the entropy rule
/// catches the rest.
const SECRET_PREFIXES: &[&str] = &[
    "sk-",
    "sk_",
    "pk_live_",
    "rk_live_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "AKIA",
    "ASIA",
    "AIza",
    "ya29.",
    "eyJ",
    "hf_",
    "npm_",
    "dop_v1_",
    "shpat_",
    "Bearer ",
    "bearer ",
    "Basic ",
];

const SECRET_MIN_ENTROPY_CHARS: usize = 24;

fn fingerprint(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let hex: String = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("redacted:{hex}")
}

/// True when a value carries the shape of a credential: a known token prefix,
/// or a long unbroken mixed-case-and-digit run with no spaces.
pub fn looks_secret(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if SECRET_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        return true;
    }
    let candidate = trimmed.rsplit(['=', ':']).next().unwrap_or(trimmed).trim();
    if candidate.chars().count() < SECRET_MIN_ENTROPY_CHARS {
        return false;
    }
    if candidate.chars().any(char::is_whitespace) {
        return false;
    }
    if candidate.contains('/') || candidate.contains('\\') {
        return false;
    }
    let has_digit = candidate.chars().any(|c| c.is_ascii_digit());
    let has_upper = candidate.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = candidate.chars().any(|c| c.is_ascii_lowercase());
    let alphanumeric_ish = candidate
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '+');
    alphanumeric_ish && has_digit && has_upper && has_lower
}

/// Redact a value for export: a secret-shaped value becomes a fingerprint, and
/// anything else is truncated to the configured label budget.
pub fn redact(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if looks_secret(trimmed) {
        return fingerprint(trimmed);
    }
    let max_chars = max_chars.max(1);
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect()
}

/// Keep only allowlisted label names and redact every value.
fn clean_labels(
    labels: Vec<(&'static str, String)>,
    max_chars: usize,
) -> (Vec<(&'static str, String)>, usize) {
    let mut dropped = 0usize;
    let mut kept: Vec<(&'static str, String)> = labels
        .into_iter()
        .filter_map(|(name, value)| {
            if !ALLOWED_LABELS.contains(&name) {
                dropped += 1;
                return None;
            }
            Some((name, redact(&value, max_chars)))
        })
        .collect();
    kept.sort();
    (kept, dropped)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MetricsSnapshot {
    pub series: BTreeMap<String, Sample>,
    pub keys: Vec<SeriesKey>,
    pub explanations: Vec<Explanation>,
}

impl MetricsSnapshot {
    /// Series identity rendered for export: `name{label=value,...}`.
    pub fn render(key: &SeriesKey) -> String {
        if key.labels.is_empty() {
            return key.name.to_string();
        }
        let labels: Vec<String> = key
            .labels
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        format!("{}{{{}}}", key.name, labels.join(","))
    }

    pub fn counter(&self, name: &str, label: &str, value: &str) -> u64 {
        self.keys
            .iter()
            .filter(|key| key.name == name && key.label(label) == Some(value))
            .filter_map(|key| self.series.get(&MetricsSnapshot::render(key)))
            .map(|sample| sample.counter())
            .sum()
    }

    pub fn explanations_for(&self, decision: Decision) -> Vec<&Explanation> {
        self.explanations
            .iter()
            .filter(|entry| entry.decision == decision)
            .collect()
    }
}

/// Bounded, process-local metric state. Every method takes `&self`, recovers a
/// poisoned lock and performs no I/O, so recording cannot fail the caller.
pub struct MetricsRecorder {
    series: Mutex<BTreeMap<SeriesKey, Sample>>,
    explanations: Mutex<VecDeque<Explanation>>,
    seq: AtomicU64,
}

impl Default for MetricsRecorder {
    fn default() -> Self {
        Self::new()
    }
}

/// The recorder's own series. They are label-free and fixed in number, and are
/// seeded at construction so that counting a refused series never has to create
/// one — which is how an overflow counter would otherwise breach the very cap
/// it reports on.
const SELF_SERIES: &[&str] = &[SERIES_DROPPED, LABELS_DROPPED, EXPLANATIONS_DROPPED];

fn seeded_series() -> BTreeMap<SeriesKey, Sample> {
    SELF_SERIES
        .iter()
        .map(|name| {
            (
                SeriesKey::new(name, Vec::new()),
                Sample::Counter { value: 0 },
            )
        })
        .collect()
}

impl MetricsRecorder {
    pub fn new() -> Self {
        MetricsRecorder {
            series: Mutex::new(seeded_series()),
            explanations: Mutex::new(VecDeque::new()),
            seq: AtomicU64::new(1),
        }
    }

    fn write_series<F>(
        &self,
        caps: Caps,
        name: &'static str,
        labels: Vec<(&'static str, String)>,
        apply: F,
    ) where
        F: FnOnce(Option<Sample>) -> Sample,
    {
        let (labels, dropped) = clean_labels(labels, caps.max_label_chars);
        let key = SeriesKey::new(name, labels);
        let mut series = self
            .series
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if dropped > 0 {
            bump(
                &mut series,
                SeriesKey::new(LABELS_DROPPED, Vec::new()),
                dropped as u64,
            );
        }
        if !series.contains_key(&key) && series.len() >= caps.max_series {
            bump(&mut series, SeriesKey::new(SERIES_DROPPED, Vec::new()), 1);
            return;
        }
        let updated = apply(series.get(&key).copied());
        series.insert(key, updated);
    }

    pub fn increment(&self, name: &'static str, labels: Vec<(&'static str, String)>, by: u64) {
        self.increment_with(caps(), name, labels, by);
    }

    fn increment_with(
        &self,
        caps: Caps,
        name: &'static str,
        labels: Vec<(&'static str, String)>,
        by: u64,
    ) {
        self.write_series(caps, name, labels, |existing| match existing {
            Some(Sample::Counter { value }) => Sample::Counter {
                value: value.saturating_add(by),
            },
            _ => Sample::Counter { value: by },
        });
    }

    pub fn count(&self, name: &'static str, labels: Vec<(&'static str, String)>) {
        self.increment_with(caps(), name, labels, 1);
    }

    pub fn gauge(&self, name: &'static str, labels: Vec<(&'static str, String)>, value: i64) {
        self.write_series(caps(), name, labels, |_| Sample::Gauge { value });
    }

    pub fn observe(&self, name: &'static str, labels: Vec<(&'static str, String)>, value: i64) {
        self.write_series(caps(), name, labels, |existing| match existing {
            Some(Sample::Observation {
                count,
                sum,
                min,
                max,
            }) => Sample::Observation {
                count: count.saturating_add(1),
                sum: sum.saturating_add(value),
                min: min.min(value),
                max: max.max(value),
            },
            _ => Sample::Observation {
                count: 1,
                sum: value,
                min: value,
                max: value,
            },
        });
    }

    /// Record one automatic decision. An entry that does not name its rule and
    /// its inputs is refused and counted, so the gap is visible rather than
    /// filed as an explanation that explains nothing.
    pub fn explain(&self, explanation: Explanation) -> bool {
        let caps = caps();
        if !explanation.is_complete() {
            self.increment_with(
                caps,
                EXPLANATIONS_INCOMPLETE,
                vec![("decision", explanation.decision.label().to_string())],
                1,
            );
            return false;
        }
        let mut entry = explanation.redacted(caps.max_label_chars);
        entry.seq = self.seq.fetch_add(1, Ordering::Relaxed);
        entry.at_ms = refact_work_core::now_ms();
        self.increment_with(
            caps,
            DECISIONS,
            vec![
                ("decision", entry.decision.label().to_string()),
                ("outcome", entry.outcome.clone()),
            ],
            1,
        );
        let mut explanations = self
            .explanations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        explanations.push_back(entry);
        let mut evicted = 0u64;
        while explanations.len() > caps.max_explanations {
            explanations.pop_front();
            evicted += 1;
        }
        drop(explanations);
        if evicted > 0 {
            self.increment_with(caps, EXPLANATIONS_DROPPED, Vec::new(), evicted);
        }
        true
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let series = self
            .series
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let keys: Vec<SeriesKey> = series.keys().cloned().collect();
        let rendered = series
            .iter()
            .map(|(key, sample)| (MetricsSnapshot::render(key), *sample))
            .collect();
        drop(series);
        let explanations = self
            .explanations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect();
        MetricsSnapshot {
            series: rendered,
            keys,
            explanations,
        }
    }

    pub fn series_len(&self) -> usize {
        self.series
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn reset(&self) {
        *self
            .series
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = seeded_series();
        self.explanations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

fn bump(series: &mut BTreeMap<SeriesKey, Sample>, key: SeriesKey, by: u64) {
    let updated = match series.get(&key).copied() {
        Some(Sample::Counter { value }) => value.saturating_add(by),
        _ => by,
    };
    series.insert(key, Sample::Counter { value: updated });
}

#[derive(Clone, Copy, Debug)]
pub struct Caps {
    pub enabled: bool,
    pub max_series: usize,
    pub max_explanations: usize,
    pub max_label_chars: usize,
}

pub fn caps() -> Caps {
    let settings = crate::runtime_settings::current();
    Caps {
        enabled: settings.coordination.metrics_enabled,
        max_series: settings.coordination.metrics_max_series.max(1),
        max_explanations: settings.coordination.metrics_max_explanations.max(1),
        max_label_chars: settings.coordination.metrics_max_label_chars.max(1),
    }
}

/// The process-wide recorder every instrumented path writes through.
pub fn recorder() -> &'static MetricsRecorder {
    static RECORDER: OnceLock<MetricsRecorder> = OnceLock::new();
    RECORDER.get_or_init(MetricsRecorder::new)
}

fn label(name: &'static str, value: impl Into<String>) -> (&'static str, String) {
    (name, value.into())
}

pub fn count(name: &'static str, labels: Vec<(&'static str, String)>) {
    if caps().enabled {
        recorder().count(name, labels);
    }
}

pub fn observe(name: &'static str, labels: Vec<(&'static str, String)>, value: i64) {
    if caps().enabled {
        recorder().observe(name, labels, value);
    }
}

pub fn gauge(name: &'static str, labels: Vec<(&'static str, String)>, value: i64) {
    if caps().enabled {
        recorder().gauge(name, labels, value);
    }
}

pub fn explain(explanation: Explanation) {
    if caps().enabled {
        recorder().explain(explanation);
    }
}

/// One series per verdict. `failed`, `blocked`, `unavailable` and `not_run` are
/// four different facts and stay four different series.
pub fn record_verdict(verdict: crate::chat::evidence::EvidenceVerdict) {
    count(EVIDENCE_VERDICTS, vec![label("verdict", verdict.label())]);
}

/// Replayed evidence is a cache hit; executed evidence is a miss.
pub fn record_evidence_cache(from_cache: bool) {
    let result = if from_cache { "hit" } else { "miss" };
    count(EVIDENCE_CACHE, vec![label("result", result)]);
}

pub fn record_queue_latency(state: &'static str, latency_ms: i64) {
    observe(QUEUE_LATENCY_MS, vec![label("state", state)], latency_ms);
}

pub fn record_unknown_file_attribution(outcome: &str) {
    count(
        UNKNOWN_FILE_ATTRIBUTION,
        vec![label("outcome", outcome.to_string())],
    );
}

pub fn record_stale_write_rejection(reason: &str) {
    count(
        STALE_WRITE_REJECTIONS,
        vec![label("reason", reason.to_string())],
    );
}

pub fn record_delivery_lag(kind: &'static str, lag_ms: i64) {
    observe(DELIVERY_LAG_MS, vec![label("kind", kind)], lag_ms);
}

pub fn record_delivery_duplicate(reason: &str) {
    count(
        DELIVERY_DUPLICATES,
        vec![label("reason", reason.to_string())],
    );
}

pub fn record_delivery_expiry(reason: &str, expired: u64) {
    if expired > 0 && caps().enabled {
        recorder().increment(
            DELIVERY_EXPIRY,
            vec![label("reason", reason.to_string())],
            expired,
        );
    }
}

pub fn record_abandoned_claim(reason: &str) {
    count(ABANDONED_CLAIMS, vec![label("reason", reason.to_string())]);
}

pub fn record_recovery_decision(decision: &str) {
    count(
        RECOVERY_DECISIONS,
        vec![label("decision", decision.to_string())],
    );
}

/// A trace that lost an ancestor is a different fact from a whole one.
pub fn record_trace_completeness(complete: bool) {
    let result = if complete { "complete" } else { "incomplete" };
    count(CAUSAL_TRACE_COMPLETENESS, vec![label("result", result)]);
}

pub fn record_context_payload(kind: &'static str, bytes: i64) {
    observe(CONTEXT_PAYLOAD_BYTES, vec![label("kind", kind)], bytes);
}

/// Sample provider pressure and AIMD state from the registry's own snapshot.
/// Reading what the controller already tracks adds nothing to the request path.
pub fn sample_provider_pressure() {
    if !caps().enabled {
        return;
    }
    let Some(registry) = refact_providers::pressure::pressure_registry() else {
        return;
    };
    for bucket in registry.snapshots() {
        let name = bucket.bucket.clone();
        gauge(
            PERMITS,
            vec![label("bucket", name.clone()), label("kind", "in_flight")],
            i64::from(bucket.in_flight),
        );
        gauge(
            PERMITS,
            vec![
                label("bucket", name.clone()),
                label("kind", "interactive_in_flight"),
            ],
            i64::from(bucket.interactive_in_flight),
        );
        gauge(
            PERMITS,
            vec![label("bucket", name.clone()), label("kind", "limit")],
            i64::from(bucket.limit),
        );
        gauge(
            PROVIDER_AIMD,
            vec![
                label("bucket", name.clone()),
                label("state", "consecutive_successes"),
            ],
            i64::from(bucket.consecutive_successes),
        );
        gauge(
            PROVIDER_AIMD,
            vec![
                label("bucket", name.clone()),
                label("state", "consecutive_retryable_failures"),
            ],
            i64::from(bucket.consecutive_retryable_failures),
        );
        let breaker = match &bucket.breaker {
            Some(state) => state.class.as_str(),
            None => "closed",
        };
        gauge(
            PROVIDER_PRESSURE,
            vec![label("bucket", name.clone()), label("state", "breaker")],
            i64::from(bucket.breaker.is_some()),
        );
        count(
            PROVIDER_PRESSURE,
            vec![label("bucket", name), label("class", breaker.to_string())],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::evidence::EvidenceVerdict;

    fn inputs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn a_seeded_credential_never_reaches_a_label_or_an_explanation() {
        let secret = "sk-live-ZZQ2ab3Ccd4Eef5Ggh6Iij7Kkl8M";
        let recorder = MetricsRecorder::new();
        recorder.count(EVIDENCE_CACHE, vec![("result", secret.to_string())]);
        recorder.explain(Explanation::new(
            Decision::Denial,
            "denied",
            "credential rejected by policy",
            inputs(&[("reason", secret), ("source", "provider")]),
        ));

        let snapshot = recorder.snapshot();
        let exported = serde_json::to_string(&snapshot).unwrap();
        assert!(
            !exported.contains(secret),
            "the seeded credential reached the export: {exported}"
        );
        assert!(
            exported.contains("redacted:"),
            "a secret-shaped value must leave a fingerprint behind"
        );
    }

    #[test]
    fn a_label_outside_the_allowlist_is_dropped_and_counted() {
        let recorder = MetricsRecorder::new();
        recorder.count(
            EVIDENCE_CACHE,
            vec![
                ("result", "hit".to_string()),
                ("AWS_SECRET_ACCESS_KEY", "value".to_string()),
            ],
        );
        let snapshot = recorder.snapshot();
        let exported = serde_json::to_string(&snapshot).unwrap();
        assert!(!exported.contains("AWS_SECRET_ACCESS_KEY"));
        assert_eq!(
            snapshot
                .series
                .get(LABELS_DROPPED)
                .map(|sample| sample.counter()),
            Some(1)
        );
    }

    #[test]
    fn a_denial_without_a_rule_is_refused_rather_than_filed_as_an_explanation() {
        let recorder = MetricsRecorder::new();
        assert!(!recorder.explain(Explanation::new(
            Decision::Denial,
            "denied",
            "   ",
            inputs(&[("reason", "nope")]),
        )));
        assert!(!recorder.explain(Explanation::new(
            Decision::Denial,
            "denied",
            "policy.rule",
            Vec::new(),
        )));
        let snapshot = recorder.snapshot();
        assert!(snapshot.explanations_for(Decision::Denial).is_empty());
        assert_eq!(
            snapshot.counter(EXPLANATIONS_INCOMPLETE, "decision", "denial"),
            2,
            "the gap is counted instead of being hidden behind an empty reason"
        );
    }

    #[test]
    fn cardinality_and_retention_caps_hold() {
        let recorder = MetricsRecorder::new();
        let cap = caps();
        for index in 0..(cap.max_series + 64) {
            recorder.count(EVIDENCE_CACHE, vec![("reason", format!("reason-{index}"))]);
        }
        assert!(
            recorder.series_len() <= cap.max_series,
            "series count {} passed the cap {}",
            recorder.series_len(),
            cap.max_series
        );
        assert!(
            recorder
                .snapshot()
                .series
                .get(SERIES_DROPPED)
                .map(|sample| sample.counter())
                .unwrap_or(0)
                > 0,
            "a refused series is counted, not silently ignored"
        );

        let recorder = MetricsRecorder::new();
        for index in 0..(cap.max_explanations + 8) {
            recorder.explain(Explanation::new(
                Decision::Retry,
                "retry",
                "provider.retryable",
                inputs(&[("reason", &format!("attempt-{index}"))]),
            ));
        }
        assert_eq!(
            recorder.snapshot().explanations.len(),
            cap.max_explanations,
            "the explanation ring is bounded"
        );
    }

    #[test]
    fn a_poisoned_recorder_keeps_recording_and_never_fails_the_caller() {
        let recorder = std::sync::Arc::new(MetricsRecorder::new());
        let poisoner = std::sync::Arc::clone(&recorder);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.series.lock().unwrap();
            panic!("poison the series lock");
        })
        .join();

        recorder.count(EVIDENCE_VERDICTS, vec![("verdict", "failed".to_string())]);
        assert!(recorder.explain(Explanation::new(
            Decision::AutomaticPause,
            "paused",
            "progress.no_progress",
            inputs(&[("reason", "unchanged output")]),
        )));
        let snapshot = recorder.snapshot();
        assert_eq!(
            snapshot.counter(EVIDENCE_VERDICTS, "verdict", "failed"),
            1,
            "a poisoned lock is recovered, not propagated"
        );
        assert_eq!(snapshot.explanations_for(Decision::AutomaticPause).len(), 1);
    }

    /// Exercising the gate itself, not the recorder: `evaluate_gates` is the
    /// path that decides a verdict, so the series must appear because a gate
    /// ran.
    #[test]
    #[serial_test::serial(coordination_metrics)]
    fn the_real_gate_path_produces_one_series_per_verdict_and_the_cache_rate() {
        use crate::chat::evidence::{
            completion_decision, evaluate_gates, unexecuted_record, EvidenceDefaults,
            SpecAuthority, VerificationSpec,
        };
        use crate::tasks::types::VerificationOutcome;

        recorder().reset();
        let defaults = EvidenceDefaults {
            timeout_secs: 60,
            output_cap_bytes: 4096,
        };
        let propose = |id: &str| {
            VerificationSpec::propose(
                id,
                "cargo test --lib",
                SpecAuthority::CardInstructions,
                &defaults,
            )
        };
        let mut specs = Vec::new();
        let mut evidence = Vec::new();
        let verdicts = [
            (EvidenceVerdict::Passed, VerificationOutcome::Passed),
            (EvidenceVerdict::Failed, VerificationOutcome::CommandFailed),
            (EvidenceVerdict::Blocked, VerificationOutcome::PolicyDenied),
            (
                EvidenceVerdict::Unavailable,
                VerificationOutcome::InfrastructureFailed,
            ),
        ];
        for (index, (verdict, outcome)) in verdicts.iter().enumerate() {
            let spec = propose(&format!("spec-{index}"));
            let mut record =
                unexecuted_record(&spec, *verdict, *outcome, format!("seeded {index}"));
            record.from_cache = index % 2 == 0;
            specs.push(spec);
            evidence.push(record);
        }

        let superseded_spec = propose("spec-superseded");
        let mut superseded = unexecuted_record(
            &superseded_spec,
            EvidenceVerdict::Passed,
            VerificationOutcome::Passed,
            "seeded superseded".to_string(),
        );
        superseded.spec_revision = superseded_spec.revision.saturating_sub(1);
        specs.push(superseded_spec);
        evidence.push(superseded);

        specs.push(propose("spec-never-ran"));

        let outcome = evaluate_gates(&specs, &evidence);
        let snapshot = recorder().snapshot();
        for verdict in [
            EvidenceVerdict::NotRun,
            EvidenceVerdict::Passed,
            EvidenceVerdict::Failed,
            EvidenceVerdict::Blocked,
            EvidenceVerdict::Unavailable,
            EvidenceVerdict::Superseded,
        ] {
            assert_eq!(
                snapshot.counter(EVIDENCE_VERDICTS, "verdict", verdict.label()),
                1,
                "{} did not get its own series from the real gate path: {:?}",
                verdict.label(),
                snapshot.series
            );
        }
        assert_eq!(
            snapshot
                .keys
                .iter()
                .filter(|key| key.name == EVIDENCE_VERDICTS)
                .count(),
            6,
            "the gate collapsed distinct verdicts into fewer series"
        );
        assert!(snapshot.counter(EVIDENCE_CACHE, "result", "hit") > 0);
        assert!(snapshot.counter(EVIDENCE_CACHE, "result", "miss") > 0);

        completion_decision(&outcome, true);
        let refusals = recorder().snapshot();
        let refusal = refusals.explanations_for(Decision::GoalRefusal);
        assert_eq!(refusal.len(), 1, "a refusal must be explained exactly once");
        assert_eq!(refusal[0].rule, "evidence.gates.required_gate_blocks");
        assert!(
            refusal[0]
                .inputs
                .iter()
                .any(|(name, _)| name == "reason" || name == "verdict"),
            "a refusal names the inputs that produced it"
        );
    }

    /// The denial comes through the conversion production uses to turn a store
    /// refusal into a lease error, so a stale write and an expired lease stay
    /// two different rejections.
    #[test]
    #[serial_test::serial(coordination_metrics)]
    fn the_real_lease_rejection_path_explains_a_denial_and_separates_its_reasons() {
        use crate::coordination::leases::LeaseError;
        use refact_work_core::StoreError;

        recorder().reset();
        let fenced = LeaseError::from(StoreError::LeaseFenced {
            lease_key: "merge:/repo".to_string(),
            presented: 3,
            held: 4,
        });
        assert!(matches!(fenced, LeaseError::Fenced { .. }));
        let expired = LeaseError::from(StoreError::LeaseExpired {
            lease_key: "merge:/repo".to_string(),
            expires_at_ms: 10,
            now_ms: 99,
        });
        assert!(matches!(expired, LeaseError::Expired { .. }));

        let snapshot = recorder().snapshot();
        assert_eq!(
            snapshot.counter(STALE_WRITE_REJECTIONS, "reason", "fencing_generation"),
            1
        );
        assert_eq!(
            snapshot.counter(STALE_WRITE_REJECTIONS, "reason", "lease_expired"),
            1,
            "an expired lease is a different rejection from a stale generation"
        );

        let denials = snapshot.explanations_for(Decision::Denial);
        assert_eq!(denials.len(), 2, "every denial is explained");
        assert!(denials
            .iter()
            .any(|entry| entry.rule == "leases.fencing_generation_behind_holder"));
        assert!(denials
            .iter()
            .any(|entry| entry.rule == "leases.authority_expired"));
        for denial in denials {
            assert!(
                denial
                    .inputs
                    .iter()
                    .any(|(name, value)| name == "reason" && !value.is_empty()),
                "a denial without a reason is not an explanation: {denial:?}"
            );
        }
    }

    #[test]
    fn a_provider_bucket_key_carries_a_fingerprint_and_not_the_credential() {
        let secret = "sk-live-ZZQ2ab3Ccd4Eef5Ggh6Iij7Kkl8M";
        let bucket = refact_providers::pressure::bucket_key("openai", secret);
        assert!(!bucket.contains(secret));
        let recorder = MetricsRecorder::new();
        recorder.gauge(
            PERMITS,
            vec![("bucket", bucket), ("kind", "limit".to_string())],
            4,
        );
        let exported = serde_json::to_string(&recorder.snapshot()).unwrap();
        assert!(!exported.contains(secret));
    }
}
