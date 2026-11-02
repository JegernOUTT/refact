//! Outbound provider pressure control: failure taxonomy, AIMD concurrency per
//! credential bucket, `Retry-After` honouring, bounded jittered retries and
//! inspectable circuit breakers.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

const CAUSE_MAX_CHARS: usize = 400;

/// What a provider failure actually was. Only `RateLimit` is provider pressure;
/// collapsing the other five into "backoff" is the failure mode this taxonomy
/// exists to prevent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureClass {
    RateLimit,
    QuotaExhausted,
    Auth,
    ClientError,
    Transport,
    ServerError,
}

impl PressureClass {
    pub fn as_str(self) -> &'static str {
        match self {
            PressureClass::RateLimit => "rate_limit",
            PressureClass::QuotaExhausted => "quota_exhausted",
            PressureClass::Auth => "auth",
            PressureClass::ClientError => "client_error",
            PressureClass::Transport => "transport",
            PressureClass::ServerError => "server_error",
        }
    }

    pub fn retryable(self) -> bool {
        matches!(
            self,
            PressureClass::RateLimit | PressureClass::Transport | PressureClass::ServerError
        )
    }

    /// Only genuine rate-limit pressure may drive concurrency down.
    pub fn drives_concurrency_decrease(self) -> bool {
        matches!(self, PressureClass::RateLimit)
    }
}

impl fmt::Display for PressureClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

const QUOTA_MARKERS: &[&str] = &[
    "insufficient_quota",
    "insufficient quota",
    "exceeded your current quota",
    "quota exceeded",
    "out of credits",
    "credit balance is too low",
    "billing hard limit",
    "payment required",
    "no remaining credits",
];

fn body_reports_quota_exhaustion(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    QUOTA_MARKERS.iter().any(|marker| body.contains(marker))
}

/// `None` means the response was a success and carries no pressure signal.
pub fn classify_http_failure(status: u16, body: &str) -> Option<PressureClass> {
    if (200..300).contains(&status) {
        return None;
    }
    if status == 401 || status == 403 {
        return Some(PressureClass::Auth);
    }
    if status == 402 {
        return Some(PressureClass::QuotaExhausted);
    }
    if status == 429 {
        return Some(if body_reports_quota_exhaustion(body) {
            PressureClass::QuotaExhausted
        } else {
            PressureClass::RateLimit
        });
    }
    if (500..600).contains(&status) {
        return Some(PressureClass::ServerError);
    }
    if body_reports_quota_exhaustion(body) {
        return Some(PressureClass::QuotaExhausted);
    }
    Some(PressureClass::ClientError)
}

/// Accepts both `Retry-After` forms: delta-seconds and an HTTP-date.
pub fn retry_after_delay_ms(value: &str, now_ms: u64) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.chars().all(|character| character.is_ascii_digit()) {
        return value
            .parse::<u64>()
            .ok()
            .map(|seconds| seconds.saturating_mul(1000));
    }
    http_date_ms(value).map(|at_ms| at_ms.saturating_sub(now_ms))
}

fn http_date_ms(value: &str) -> Option<u64> {
    for format in [
        "%a, %d %b %Y %H:%M:%S GMT",
        "%A, %d-%b-%y %H:%M:%S GMT",
        "%a %b %e %H:%M:%S %Y",
    ] {
        if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(value, format) {
            return u64::try_from(parsed.and_utc().timestamp_millis()).ok();
        }
    }
    chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .and_then(|parsed| u64::try_from(parsed.timestamp_millis()).ok())
}

/// Time and jitter are injected so behaviour is testable without sleeping.
pub trait PressureEnv: Send + Sync + fmt::Debug {
    fn now_ms(&self) -> u64;
    /// Uniform value in `[0, 1]` used to spread retries.
    fn jitter_unit(&self) -> f64;
}

#[derive(Debug, Default)]
pub struct SystemPressureEnv;

impl PressureEnv for SystemPressureEnv {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0)
    }

    fn jitter_unit(&self) -> f64 {
        rand::random::<f64>()
    }
}

#[derive(Debug)]
pub struct ManualPressureEnv {
    now_ms: AtomicU64,
    jitter_per_mille: AtomicU64,
}

impl ManualPressureEnv {
    pub fn new(now_ms: u64) -> Self {
        ManualPressureEnv {
            now_ms: AtomicU64::new(now_ms),
            jitter_per_mille: AtomicU64::new(0),
        }
    }

    pub fn advance(&self, millis: u64) {
        self.now_ms.fetch_add(millis, Ordering::SeqCst);
    }

    pub fn set_jitter_unit(&self, unit: f64) {
        let clamped = (unit.clamp(0.0, 1.0) * 1000.0).round() as u64;
        self.jitter_per_mille.store(clamped, Ordering::SeqCst);
    }
}

impl PressureEnv for ManualPressureEnv {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }

    fn jitter_unit(&self) -> f64 {
        self.jitter_per_mille.load(Ordering::SeqCst) as f64 / 1000.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PressureConfig {
    pub max_concurrency: u32,
    pub min_concurrency: u32,
    pub initial_concurrency: u32,
    pub interactive_reserved_slots: u32,
    pub increase_after_successes: u32,
    pub decrease_cooldown_ms: u64,
    pub max_retry_attempts: u32,
    pub base_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub jitter_per_mille: u32,
    pub breaker_failure_threshold: u32,
    pub breaker_cooldown_ms: u64,
    pub max_retry_after_ms: u64,
    pub max_buckets: usize,
}

impl Default for PressureConfig {
    fn default() -> Self {
        PressureConfig {
            max_concurrency: 16,
            min_concurrency: 1,
            initial_concurrency: 4,
            interactive_reserved_slots: 1,
            increase_after_successes: 8,
            decrease_cooldown_ms: 1_000,
            max_retry_attempts: 4,
            base_backoff_ms: 500,
            max_backoff_ms: 60_000,
            jitter_per_mille: 250,
            breaker_failure_threshold: 8,
            breaker_cooldown_ms: 30_000,
            max_retry_after_ms: 300_000,
            max_buckets: 4_096,
        }
    }
}

impl PressureConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("max_concurrency", self.max_concurrency as u64),
            ("min_concurrency", self.min_concurrency as u64),
            ("initial_concurrency", self.initial_concurrency as u64),
            (
                "increase_after_successes",
                self.increase_after_successes as u64,
            ),
            ("max_retry_attempts", self.max_retry_attempts as u64),
            ("base_backoff_ms", self.base_backoff_ms),
            ("max_backoff_ms", self.max_backoff_ms),
            (
                "breaker_failure_threshold",
                self.breaker_failure_threshold as u64,
            ),
            ("breaker_cooldown_ms", self.breaker_cooldown_ms),
            ("max_retry_after_ms", self.max_retry_after_ms),
            ("max_buckets", self.max_buckets as u64),
        ] {
            if value < 1 {
                return Err(format!("provider pressure {field} must be at least 1"));
            }
        }
        if self.min_concurrency > self.initial_concurrency {
            return Err(
                "provider pressure min_concurrency must not exceed initial_concurrency".to_string(),
            );
        }
        if self.initial_concurrency > self.max_concurrency {
            return Err(
                "provider pressure initial_concurrency must not exceed max_concurrency".to_string(),
            );
        }
        if self.interactive_reserved_slots >= self.max_concurrency {
            return Err(
                "provider pressure interactive_reserved_slots must leave a background slot"
                    .to_string(),
            );
        }
        if self.base_backoff_ms > self.max_backoff_ms {
            return Err(
                "provider pressure base_backoff_ms must not exceed max_backoff_ms".to_string(),
            );
        }
        if self.jitter_per_mille > 1000 {
            return Err("provider pressure jitter_per_mille must not exceed 1000".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestPriority {
    Interactive,
    Background,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionRefusal {
    CoolingDown {
        reason: String,
        until_ms: u64,
    },
    AtCapacity {
        limit: u32,
        in_flight: u32,
    },
    BackgroundReserved {
        background_limit: u32,
        background_in_flight: u32,
    },
}

impl fmt::Display for AdmissionRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdmissionRefusal::CoolingDown { reason, until_ms } => {
                write!(f, "cooling down because {reason} until {until_ms}")
            }
            AdmissionRefusal::AtCapacity { limit, in_flight } => {
                write!(f, "at capacity: {in_flight} in flight of {limit}")
            }
            AdmissionRefusal::BackgroundReserved {
                background_limit,
                background_in_flight,
            } => write!(
                f,
                "background traffic capped at {background_limit}, {background_in_flight} in flight"
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BreakerState {
    pub class: PressureClass,
    pub reason: String,
    pub opened_at_ms: u64,
    pub until_ms: u64,
}

impl BreakerState {
    pub fn describe(&self) -> String {
        format!(
            "cooling down because {} until {}",
            self.reason, self.until_ms
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BucketSnapshot {
    pub bucket: String,
    pub limit: u32,
    pub in_flight: u32,
    pub interactive_in_flight: u32,
    pub consecutive_successes: u32,
    pub consecutive_retryable_failures: u32,
    pub retry_after_until_ms: Option<u64>,
    pub breaker: Option<BreakerState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureReport {
    pub class: PressureClass,
    pub retry_after: Option<String>,
    pub cause: String,
}

impl FailureReport {
    /// `None` for a successful response.
    pub fn from_http(status: u16, retry_after: Option<&str>, body: &str) -> Option<Self> {
        let class = classify_http_failure(status, body)?;
        Some(FailureReport {
            class,
            retry_after: retry_after.map(|value| value.to_string()),
            cause: cap_cause(&format!("HTTP {status}: {}", body.trim())),
        })
    }

    pub fn transport(cause: impl AsRef<str>) -> Self {
        FailureReport {
            class: PressureClass::Transport,
            retry_after: None,
            cause: cap_cause(cause.as_ref()),
        }
    }
}

fn cap_cause(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= CAUSE_MAX_CHARS {
        return text.to_string();
    }
    text.chars().take(CAUSE_MAX_CHARS).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure(FailureReport),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    Retry {
        class: PressureClass,
        attempt: u32,
        delay_ms: u64,
        honoured_retry_after: bool,
    },
    GiveUp {
        class: PressureClass,
        attempts: u32,
        cause: String,
    },
}

#[derive(Debug)]
struct BucketState {
    limit: u32,
    in_flight: u32,
    interactive_in_flight: u32,
    consecutive_successes: u32,
    consecutive_retryable_failures: u32,
    last_decrease_ms: Option<u64>,
    retry_after_until_ms: Option<u64>,
    breaker: Option<BreakerState>,
    last_touched_ms: u64,
}

impl BucketState {
    fn new(limit: u32, now_ms: u64) -> Self {
        BucketState {
            limit,
            in_flight: 0,
            interactive_in_flight: 0,
            consecutive_successes: 0,
            consecutive_retryable_failures: 0,
            last_decrease_ms: None,
            retry_after_until_ms: None,
            breaker: None,
            last_touched_ms: now_ms,
        }
    }

    fn refresh(&mut self, now_ms: u64) {
        if self
            .breaker
            .as_ref()
            .is_some_and(|open| now_ms >= open.until_ms)
        {
            self.breaker = None;
            self.consecutive_retryable_failures = 0;
        }
        if self
            .retry_after_until_ms
            .is_some_and(|until| now_ms >= until)
        {
            self.retry_after_until_ms = None;
        }
    }

    fn snapshot(&self, bucket: &str) -> BucketSnapshot {
        BucketSnapshot {
            bucket: bucket.to_string(),
            limit: self.limit,
            in_flight: self.in_flight,
            interactive_in_flight: self.interactive_in_flight,
            consecutive_successes: self.consecutive_successes,
            consecutive_retryable_failures: self.consecutive_retryable_failures,
            retry_after_until_ms: self.retry_after_until_ms,
            breaker: self.breaker.clone(),
        }
    }
}

#[derive(Debug)]
struct Shared {
    config: RwLock<PressureConfig>,
    env: Arc<dyn PressureEnv>,
    buckets: Mutex<HashMap<String, BucketState>>,
}

impl Shared {
    fn config(&self) -> RwLockReadGuard<'_, PressureConfig> {
        self.config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn buckets(&self) -> MutexGuard<'_, HashMap<String, BucketState>> {
        self.buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn release(&self, bucket: &str, priority: RequestPriority) {
        let mut buckets = self.buckets();
        if let Some(state) = buckets.get_mut(bucket) {
            state.in_flight = state.in_flight.saturating_sub(1);
            if priority == RequestPriority::Interactive {
                state.interactive_in_flight = state.interactive_in_flight.saturating_sub(1);
            }
        }
    }

    fn record(&self, bucket: &str, outcome: Outcome) {
        let now_ms = self.env.now_ms();
        let config = self.config();
        let mut buckets = self.buckets();
        let state = bucket_entry(&mut buckets, &config, bucket, now_ms);
        match outcome {
            Outcome::Success => {
                state.consecutive_retryable_failures = 0;
                state.consecutive_successes = state.consecutive_successes.saturating_add(1);
                if state.consecutive_successes >= config.increase_after_successes {
                    state.limit = state.limit.saturating_add(1).min(config.max_concurrency);
                    state.consecutive_successes = 0;
                }
            }
            Outcome::Failure(report) => {
                state.consecutive_successes = 0;
                if let Some(delay_ms) = report
                    .retry_after
                    .as_deref()
                    .and_then(|raw| retry_after_delay_ms(raw, now_ms))
                {
                    let until_ms = now_ms.saturating_add(delay_ms);
                    state.retry_after_until_ms =
                        Some(state.retry_after_until_ms.unwrap_or(0).max(until_ms));
                }
                if report.class.drives_concurrency_decrease() {
                    let cooled = state.last_decrease_ms.is_none_or(|last| {
                        now_ms.saturating_sub(last) >= config.decrease_cooldown_ms
                    });
                    if cooled {
                        state.limit = (state.limit / 2).max(config.min_concurrency);
                        state.last_decrease_ms = Some(now_ms);
                    }
                }
                if report.class.retryable() {
                    state.consecutive_retryable_failures =
                        state.consecutive_retryable_failures.saturating_add(1);
                    if state.breaker.is_none()
                        && state.consecutive_retryable_failures >= config.breaker_failure_threshold
                    {
                        let until_ms = now_ms
                            .saturating_add(config.breaker_cooldown_ms)
                            .max(state.retry_after_until_ms.unwrap_or(0));
                        state.breaker = Some(BreakerState {
                            class: report.class,
                            reason: format!("{} from {bucket}: {}", report.class, report.cause),
                            opened_at_ms: now_ms,
                            until_ms,
                        });
                    }
                }
            }
        }
    }
}

fn bucket_entry<'a>(
    buckets: &'a mut HashMap<String, BucketState>,
    config: &PressureConfig,
    bucket: &str,
    now_ms: u64,
) -> &'a mut BucketState {
    if !buckets.contains_key(bucket) && buckets.len() >= config.max_buckets {
        let idle = buckets
            .iter()
            .filter(|(_, state)| state.in_flight == 0)
            .min_by_key(|(_, state)| state.last_touched_ms)
            .map(|(key, _)| key.clone());
        if let Some(key) = idle {
            buckets.remove(&key);
        }
    }
    let state = buckets
        .entry(bucket.to_string())
        .or_insert_with(|| BucketState::new(config.initial_concurrency, now_ms));
    state.last_touched_ms = now_ms;
    state.refresh(now_ms);
    state
}

/// An admitted request. Dropping it frees the slot; `complete` additionally
/// feeds the outcome back into AIMD and the breaker.
#[derive(Debug)]
pub struct Permit {
    shared: Arc<Shared>,
    bucket: String,
    priority: RequestPriority,
}

impl Permit {
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn priority(&self) -> RequestPriority {
        self.priority
    }

    pub fn complete(self, outcome: Outcome) {
        self.shared.record(&self.bucket, outcome);
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.shared.release(&self.bucket, self.priority);
    }
}

#[derive(Debug)]
pub struct PressureRegistry {
    shared: Arc<Shared>,
}

impl PressureRegistry {
    pub fn new(config: PressureConfig, env: Arc<dyn PressureEnv>) -> Result<Self, String> {
        config.validate()?;
        Ok(PressureRegistry {
            shared: Arc::new(Shared {
                config: RwLock::new(config),
                env,
                buckets: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn with_system_env(config: PressureConfig) -> Result<Self, String> {
        PressureRegistry::new(config, Arc::new(SystemPressureEnv))
    }

    pub fn config(&self) -> PressureConfig {
        self.shared.config().clone()
    }

    pub fn reconfigure(&self, config: PressureConfig) -> Result<(), String> {
        config.validate()?;
        let mut current = self
            .shared
            .config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut buckets = self.shared.buckets();
        for state in buckets.values_mut() {
            state.limit = state
                .limit
                .clamp(config.min_concurrency, config.max_concurrency);
        }
        *current = config;
        Ok(())
    }

    pub fn now_ms(&self) -> u64 {
        self.shared.env.now_ms()
    }

    pub fn try_admit(
        &self,
        bucket: &str,
        priority: RequestPriority,
    ) -> Result<Permit, AdmissionRefusal> {
        let now_ms = self.shared.env.now_ms();
        let config = self.shared.config();
        let mut buckets = self.shared.buckets();
        let state = bucket_entry(&mut buckets, &config, bucket, now_ms);
        if let Some(open) = &state.breaker {
            return Err(AdmissionRefusal::CoolingDown {
                reason: open.reason.clone(),
                until_ms: open.until_ms,
            });
        }
        if let Some(until_ms) = state.retry_after_until_ms {
            return Err(AdmissionRefusal::CoolingDown {
                reason: format!("{bucket} asked to wait"),
                until_ms,
            });
        }
        if state.in_flight >= state.limit {
            return Err(AdmissionRefusal::AtCapacity {
                limit: state.limit,
                in_flight: state.in_flight,
            });
        }
        if priority == RequestPriority::Background {
            let background_limit = state
                .limit
                .saturating_sub(config.interactive_reserved_slots);
            let background_in_flight = state.in_flight - state.interactive_in_flight;
            if background_in_flight >= background_limit {
                return Err(AdmissionRefusal::BackgroundReserved {
                    background_limit,
                    background_in_flight,
                });
            }
        }
        state.in_flight = state.in_flight.saturating_add(1);
        if priority == RequestPriority::Interactive {
            state.interactive_in_flight = state.interactive_in_flight.saturating_add(1);
        }
        Ok(Permit {
            shared: self.shared.clone(),
            bucket: bucket.to_string(),
            priority,
        })
    }

    /// An explicit `Retry-After` outranks the computed backoff; a class that
    /// cannot be fixed by waiting is never retried and keeps its cause.
    pub fn retry_decision(&self, report: &FailureReport, completed_attempts: u32) -> RetryDecision {
        let config = self.shared.config();
        if !report.class.retryable() || completed_attempts >= config.max_retry_attempts {
            return RetryDecision::GiveUp {
                class: report.class,
                attempts: completed_attempts,
                cause: report.cause.clone(),
            };
        }
        let now_ms = self.shared.env.now_ms();
        if let Some(delay_ms) = report
            .retry_after
            .as_deref()
            .and_then(|raw| retry_after_delay_ms(raw, now_ms))
        {
            return RetryDecision::Retry {
                class: report.class,
                attempt: completed_attempts + 1,
                delay_ms: delay_ms,
                honoured_retry_after: true,
            };
        }
        let doubling = 1_u64.checked_shl(completed_attempts).unwrap_or(u64::MAX);
        let base_ms = config
            .base_backoff_ms
            .saturating_mul(doubling)
            .min(config.max_backoff_ms);
        let span_ms = base_ms.saturating_mul(config.jitter_per_mille as u64) / 1000;
        let jitter_ms = (span_ms as f64 * self.shared.env.jitter_unit()) as u64;
        RetryDecision::Retry {
            class: report.class,
            attempt: completed_attempts + 1,
            delay_ms: base_ms.saturating_add(jitter_ms),
            honoured_retry_after: false,
        }
    }

    pub fn snapshot(&self, bucket: &str) -> Option<BucketSnapshot> {
        let now_ms = self.shared.env.now_ms();
        let mut buckets = self.shared.buckets();
        let state = buckets.get_mut(bucket)?;
        state.refresh(now_ms);
        Some(state.snapshot(bucket))
    }

    pub fn snapshots(&self) -> Vec<BucketSnapshot> {
        let now_ms = self.shared.env.now_ms();
        let mut buckets = self.shared.buckets();
        let mut rows: Vec<BucketSnapshot> = buckets
            .iter_mut()
            .map(|(bucket, state)| {
                state.refresh(now_ms);
                state.snapshot(bucket)
            })
            .collect();
        rows.sort_by(|left, right| left.bucket.cmp(&right.bucket));
        rows
    }
}

/// Buckets are per provider credential, never per model: the rate limit lives
/// with the credential. The credential itself is fingerprinted, never stored.
pub fn bucket_key(provider_id: &str, credential: &str) -> String {
    let provider = match provider_id.trim() {
        "" => "unknown",
        trimmed => trimmed,
    };
    let credential = credential.trim();
    if credential.is_empty() {
        return format!("{provider}/anonymous");
    }
    let digest = Sha256::digest(credential.as_bytes());
    let fingerprint: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{provider}/{fingerprint}")
}

fn global_slot() -> &'static RwLock<Option<Arc<PressureRegistry>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<PressureRegistry>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

pub fn install_pressure_registry(config: PressureConfig) -> Result<Arc<PressureRegistry>, String> {
    config.validate()?;
    let mut guard = global_slot()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(registry) = guard.as_ref() {
        registry.reconfigure(config)?;
        return Ok(registry.clone());
    }
    let registry = Arc::new(PressureRegistry::with_system_env(config)?);
    *guard = Some(registry.clone());
    Ok(registry)
}

pub fn uninstall_pressure_registry() {
    let mut guard = global_slot()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = None;
}

pub fn pressure_registry() -> Option<Arc<PressureRegistry>> {
    global_slot()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000_000;

    struct FakeProvider {
        script: Vec<(Option<u16>, Option<String>, String)>,
        served: usize,
    }

    impl FakeProvider {
        fn new(script: Vec<(Option<u16>, Option<String>, String)>) -> Self {
            FakeProvider { script, served: 0 }
        }

        fn http(status: u16, body: &str) -> Self {
            FakeProvider::new(vec![(Some(status), None, body.to_string())])
        }

        fn http_with_retry_after(status: u16, retry_after: &str, body: &str) -> Self {
            FakeProvider::new(vec![(
                Some(status),
                Some(retry_after.to_string()),
                body.to_string(),
            )])
        }

        fn call(&mut self) -> Outcome {
            let index = self.served.min(self.script.len().saturating_sub(1));
            self.served += 1;
            let (status, retry_after, body) = self.script[index].clone();
            match status {
                None => Outcome::Failure(FailureReport::transport("connection reset by peer")),
                Some(status) => {
                    match FailureReport::from_http(status, retry_after.as_deref(), &body) {
                        None => Outcome::Success,
                        Some(report) => Outcome::Failure(report),
                    }
                }
            }
        }
    }

    fn registry(config: PressureConfig) -> (PressureRegistry, Arc<ManualPressureEnv>) {
        let env = Arc::new(ManualPressureEnv::new(NOW));
        let registry = PressureRegistry::new(config, env.clone()).unwrap();
        (registry, env)
    }

    fn drive(
        registry: &PressureRegistry,
        bucket: &str,
        provider: &mut FakeProvider,
    ) -> Option<FailureReport> {
        let permit = registry
            .try_admit(bucket, RequestPriority::Interactive)
            .unwrap();
        match provider.call() {
            Outcome::Success => {
                permit.complete(Outcome::Success);
                None
            }
            Outcome::Failure(report) => {
                permit.complete(Outcome::Failure(report.clone()));
                Some(report)
            }
        }
    }

    #[test]
    fn six_failure_classes_are_distinguishable() {
        assert_eq!(
            classify_http_failure(429, "rate limit reached for gpt-4"),
            Some(PressureClass::RateLimit)
        );
        assert_eq!(
            classify_http_failure(429, r#"{"error":{"code":"insufficient_quota"}}"#),
            Some(PressureClass::QuotaExhausted)
        );
        assert_eq!(
            classify_http_failure(401, "bad key"),
            Some(PressureClass::Auth)
        );
        assert_eq!(
            classify_http_failure(400, "unknown field 'tools'"),
            Some(PressureClass::ClientError)
        );
        assert_eq!(
            FailureReport::transport("connection reset").class,
            PressureClass::Transport
        );
        assert_eq!(
            classify_http_failure(503, "service unavailable"),
            Some(PressureClass::ServerError)
        );
        assert_eq!(classify_http_failure(200, "{}"), None);
    }

    #[test]
    fn only_rate_limit_drives_concurrency_down_and_quota_is_not_retried() {
        for class in [
            PressureClass::QuotaExhausted,
            PressureClass::Auth,
            PressureClass::ClientError,
        ] {
            assert!(!class.retryable(), "{class} must not be retried");
            assert!(!class.drives_concurrency_decrease());
        }
        for class in [PressureClass::Transport, PressureClass::ServerError] {
            assert!(class.retryable());
            assert!(!class.drives_concurrency_decrease());
        }
        assert!(PressureClass::RateLimit.retryable());
        assert!(PressureClass::RateLimit.drives_concurrency_decrease());
    }

    #[test]
    fn rate_limit_with_delta_seconds_halves_concurrency_and_blocks_earlier_retry() {
        let (registry, env) = registry(PressureConfig::default());
        let mut provider = FakeProvider::http_with_retry_after(429, "5", "rate limit reached");

        let report = drive(&registry, "openai/aaa", &mut provider).unwrap();
        assert_eq!(report.class, PressureClass::RateLimit);

        let snapshot = registry.snapshot("openai/aaa").unwrap();
        assert_eq!(snapshot.limit, 2);
        assert_eq!(snapshot.retry_after_until_ms, Some(NOW + 5_000));

        assert_eq!(
            registry.retry_decision(&report, 0),
            RetryDecision::Retry {
                class: PressureClass::RateLimit,
                attempt: 1,
                delay_ms: 5_000,
                honoured_retry_after: true,
            }
        );

        env.advance(4_999);
        assert!(matches!(
            registry.try_admit("openai/aaa", RequestPriority::Interactive),
            Err(AdmissionRefusal::CoolingDown { until_ms, .. }) if until_ms == NOW + 5_000
        ));

        env.advance(1);
        assert!(registry
            .try_admit("openai/aaa", RequestPriority::Interactive)
            .is_ok());
    }

    #[test]
    fn retry_after_http_date_is_parsed_and_honoured() {
        let at_ms = 1_445_412_480_000_u64;
        let env = Arc::new(ManualPressureEnv::new(at_ms - 30_000));
        let registry = PressureRegistry::new(PressureConfig::default(), env).unwrap();
        let mut provider = FakeProvider::http_with_retry_after(
            429,
            "Wed, 21 Oct 2015 07:28:00 GMT",
            "rate limit reached",
        );

        let report = drive(&registry, "openai/aaa", &mut provider).unwrap();

        assert_eq!(report.class, PressureClass::RateLimit);
        assert_eq!(
            registry
                .snapshot("openai/aaa")
                .unwrap()
                .retry_after_until_ms,
            Some(at_ms)
        );
        assert_eq!(
            registry.retry_decision(&report, 0),
            RetryDecision::Retry {
                class: PressureClass::RateLimit,
                attempt: 1,
                delay_ms: 30_000,
                honoured_retry_after: true,
            }
        );
        assert_eq!(
            retry_after_delay_ms("Wed, 21 Oct 2015 07:28:00 GMT", at_ms + 5_000),
            Some(0)
        );
    }

    #[test]
    fn long_retry_after_is_not_shortened_by_backoff_cap() {
        let (registry, env) = registry(PressureConfig::default());
        let mut provider = FakeProvider::http_with_retry_after(429, "900", "rate limit reached");
        drive(&registry, "openai/long", &mut provider);
        env.advance(899_999);
        assert!(matches!(registry.try_admit("openai/long", RequestPriority::Interactive),
            Err(AdmissionRefusal::CoolingDown { until_ms, .. }) if until_ms == NOW + 900_000));
        env.advance(1);
        assert!(registry.try_admit("openai/long", RequestPriority::Interactive).is_ok());
    }

    #[test]
    fn reconfigure_preserves_provider_retry_after_deadline() {
        let (registry, env) = registry(PressureConfig::default());
        let mut provider = FakeProvider::http_with_retry_after(429, "900", "rate limit reached");
        drive(&registry, "openai/reconfigure", &mut provider);
        let mut config = PressureConfig::default();
        config.max_retry_after_ms = 1_000;
        registry.reconfigure(config).unwrap();
        env.advance(300_000);
        assert!(matches!(registry.try_admit("openai/reconfigure", RequestPriority::Interactive),
            Err(AdmissionRefusal::CoolingDown { until_ms, .. }) if until_ms == NOW + 900_000));
    }

    #[test]
    fn quota_exhaustion_neither_backs_off_nor_retries() {
        let (registry, _env) = registry(PressureConfig::default());
        let mut provider = FakeProvider::http(429, r#"{"error":{"code":"insufficient_quota"}}"#);

        let report = drive(&registry, "openai/aaa", &mut provider).unwrap();
        assert_eq!(report.class, PressureClass::QuotaExhausted);

        let snapshot = registry.snapshot("openai/aaa").unwrap();
        assert_eq!(
            snapshot.limit,
            PressureConfig::default().initial_concurrency
        );
        assert_eq!(snapshot.retry_after_until_ms, None);
        assert!(snapshot.breaker.is_none());

        assert_eq!(
            registry.retry_decision(&report, 0),
            RetryDecision::GiveUp {
                class: PressureClass::QuotaExhausted,
                attempts: 0,
                cause: report.cause.clone(),
            }
        );
    }

    #[test]
    fn auth_failure_is_never_retried() {
        let (registry, _env) = registry(PressureConfig::default());
        let mut provider = FakeProvider::http(401, "invalid api key");

        let report = drive(&registry, "openai/aaa", &mut provider).unwrap();

        assert_eq!(report.class, PressureClass::Auth);
        assert_eq!(
            registry.retry_decision(&report, 0),
            RetryDecision::GiveUp {
                class: PressureClass::Auth,
                attempts: 0,
                cause: report.cause.clone(),
            }
        );
        assert_eq!(
            registry.snapshot("openai/aaa").unwrap().limit,
            PressureConfig::default().initial_concurrency
        );
    }

    #[test]
    fn malformed_request_is_not_retried_and_keeps_its_cause() {
        let (registry, _env) = registry(PressureConfig::default());
        let mut provider = FakeProvider::http(400, "unknown field 'tolls'");

        let report = drive(&registry, "openai/aaa", &mut provider).unwrap();

        assert_eq!(report.class, PressureClass::ClientError);
        match registry.retry_decision(&report, 0) {
            RetryDecision::GiveUp { cause, .. } => {
                assert!(cause.contains("unknown field 'tolls'"), "{cause}");
                assert!(cause.contains("HTTP 400"), "{cause}");
            }
            other => panic!("client errors must not be retried: {other:?}"),
        }
    }

    #[test]
    fn transport_failures_retry_with_bounded_jitter_then_surface_the_original_cause() {
        let config = PressureConfig {
            max_retry_attempts: 3,
            base_backoff_ms: 1_000,
            jitter_per_mille: 500,
            ..PressureConfig::default()
        };
        let (registry, env) = registry(config);
        let report = FailureReport::transport("connection reset by peer");

        env.set_jitter_unit(0.0);
        assert_eq!(
            registry.retry_decision(&report, 0),
            RetryDecision::Retry {
                class: PressureClass::Transport,
                attempt: 1,
                delay_ms: 1_000,
                honoured_retry_after: false,
            }
        );
        env.set_jitter_unit(1.0);
        assert_eq!(
            registry.retry_decision(&report, 1),
            RetryDecision::Retry {
                class: PressureClass::Transport,
                attempt: 2,
                delay_ms: 3_000,
                honoured_retry_after: false,
            }
        );
        assert_eq!(
            registry.retry_decision(&report, 3),
            RetryDecision::GiveUp {
                class: PressureClass::Transport,
                attempts: 3,
                cause: "connection reset by peer".to_string(),
            }
        );
    }

    #[test]
    fn sustained_success_grows_additively_up_to_the_ceiling() {
        let config = PressureConfig {
            max_concurrency: 6,
            initial_concurrency: 4,
            increase_after_successes: 2,
            ..PressureConfig::default()
        };
        let (registry, _env) = registry(config);
        let mut provider = FakeProvider::http(200, "{}");

        for _ in 0..4 {
            assert!(drive(&registry, "openai/aaa", &mut provider).is_none());
        }
        assert_eq!(registry.snapshot("openai/aaa").unwrap().limit, 6);

        for _ in 0..10 {
            assert!(drive(&registry, "openai/aaa", &mut provider).is_none());
        }
        assert_eq!(registry.snapshot("openai/aaa").unwrap().limit, 6);
    }

    #[test]
    fn aimd_is_per_credential_bucket() {
        let (registry, _env) = registry(PressureConfig::default());
        let mut pressured = FakeProvider::http(429, "rate limit reached");
        let mut healthy = FakeProvider::http(200, "{}");

        let first = bucket_key("openai", "key-one");
        let second = bucket_key("openai", "key-two");
        assert_ne!(first, second);
        assert!(!first.contains("key-one"));

        drive(&registry, &first, &mut pressured);
        drive(&registry, &second, &mut healthy);

        assert_eq!(registry.snapshot(&first).unwrap().limit, 2);
        assert_eq!(
            registry.snapshot(&second).unwrap().limit,
            PressureConfig::default().initial_concurrency
        );
    }

    #[test]
    fn breaker_opens_with_reason_and_expiry_then_closes_after_cooldown() {
        let config = PressureConfig {
            breaker_failure_threshold: 3,
            breaker_cooldown_ms: 10_000,
            min_concurrency: 1,
            ..PressureConfig::default()
        };
        let (registry, env) = registry(config);
        let mut provider = FakeProvider::http(503, "service unavailable");

        for _ in 0..3 {
            drive(&registry, "openai/aaa", &mut provider);
        }

        let breaker = registry.snapshot("openai/aaa").unwrap().breaker.unwrap();
        assert_eq!(breaker.class, PressureClass::ServerError);
        assert_eq!(breaker.until_ms, NOW + 10_000);
        assert!(
            breaker.reason.contains("server_error"),
            "{}",
            breaker.reason
        );
        assert!(breaker.describe().contains("cooling down because"));
        assert!(matches!(
            registry.try_admit("openai/aaa", RequestPriority::Interactive),
            Err(AdmissionRefusal::CoolingDown { .. })
        ));

        env.advance(10_000);
        assert!(registry.snapshot("openai/aaa").unwrap().breaker.is_none());
        assert!(registry
            .try_admit("openai/aaa", RequestPriority::Interactive)
            .is_ok());
    }

    #[test]
    fn a_background_surge_cannot_starve_interactive_traffic() {
        let config = PressureConfig {
            max_concurrency: 4,
            initial_concurrency: 4,
            interactive_reserved_slots: 1,
            ..PressureConfig::default()
        };
        let (registry, _env) = registry(config);

        let mut background = Vec::new();
        for _ in 0..3 {
            background.push(
                registry
                    .try_admit("openai/aaa", RequestPriority::Background)
                    .unwrap(),
            );
        }
        assert!(matches!(
            registry.try_admit("openai/aaa", RequestPriority::Background),
            Err(AdmissionRefusal::BackgroundReserved {
                background_limit: 3,
                background_in_flight: 3
            })
        ));

        let interactive = registry
            .try_admit("openai/aaa", RequestPriority::Interactive)
            .unwrap();
        assert_eq!(interactive.priority(), RequestPriority::Interactive);
        drop(interactive);
        drop(background);

        assert_eq!(registry.snapshot("openai/aaa").unwrap().in_flight, 0);
    }

    #[test]
    fn concurrent_pressure_within_the_cooldown_halves_only_once() {
        let config = PressureConfig {
            initial_concurrency: 8,
            max_concurrency: 8,
            decrease_cooldown_ms: 1_000,
            ..PressureConfig::default()
        };
        let (registry, env) = registry(config);
        let mut provider = FakeProvider::http(429, "rate limit reached");

        drive(&registry, "openai/aaa", &mut provider);
        drive(&registry, "openai/aaa", &mut provider);
        assert_eq!(registry.snapshot("openai/aaa").unwrap().limit, 4);

        env.advance(1_000);
        drive(&registry, "openai/aaa", &mut provider);
        assert_eq!(registry.snapshot("openai/aaa").unwrap().limit, 2);
    }

    #[test]
    fn invalid_configuration_is_rejected_not_coerced() {
        for config in [
            PressureConfig {
                min_concurrency: 8,
                initial_concurrency: 4,
                ..PressureConfig::default()
            },
            PressureConfig {
                initial_concurrency: 32,
                max_concurrency: 16,
                ..PressureConfig::default()
            },
            PressureConfig {
                interactive_reserved_slots: 16,
                max_concurrency: 16,
                ..PressureConfig::default()
            },
            PressureConfig {
                base_backoff_ms: 90_000,
                max_backoff_ms: 60_000,
                ..PressureConfig::default()
            },
            PressureConfig {
                jitter_per_mille: 1_001,
                ..PressureConfig::default()
            },
            PressureConfig {
                max_retry_attempts: 0,
                ..PressureConfig::default()
            },
        ] {
            assert!(
                PressureRegistry::with_system_env(config.clone()).is_err(),
                "{config:?} must be rejected"
            );
        }
        PressureConfig::default().validate().unwrap();
    }

    #[test]
    fn idle_buckets_are_evicted_once_the_ceiling_is_reached() {
        let config = PressureConfig {
            max_buckets: 2,
            ..PressureConfig::default()
        };
        let (registry, env) = registry(config);

        registry
            .try_admit("a", RequestPriority::Interactive)
            .unwrap();
        env.advance(10);
        registry
            .try_admit("b", RequestPriority::Interactive)
            .unwrap();
        env.advance(10);
        registry
            .try_admit("c", RequestPriority::Interactive)
            .unwrap();

        assert_eq!(registry.snapshots().len(), 2);
        assert!(registry.snapshot("a").is_none());
    }
}
