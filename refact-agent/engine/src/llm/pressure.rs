//! Bridges coordination settings onto the provider pressure controller, and is
//! the seam where admission and retry decisions become measurable.

use std::time::Duration;

use refact_providers::pressure::{
    install_pressure_registry, pressure_registry, uninstall_pressure_registry, AdmissionRefusal,
    FailureReport, Permit, PressureConfig, RequestPriority, RetryDecision,
};
use refact_work_core::CoordinationSettings;

use crate::coordination::metrics::{self, Decision, Explanation};

#[derive(Default)]
pub(crate) struct CapacityWatch {
    waiting: std::sync::Mutex<Option<(tokio::time::Instant, String)>>,
}

impl CapacityWatch {
    pub(crate) fn waiting(&self) -> Option<(Duration, String)> {
        self.waiting
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|(started, provider)| (started.elapsed(), provider.clone()))
    }
}

tokio::task_local! {
    static CAPACITY_WATCH: (std::sync::Arc<CapacityWatch>, std::sync::Arc<std::sync::atomic::AtomicU64>);
}

pub(crate) async fn watch_capacity<F: std::future::Future>(
    watch: std::sync::Arc<CapacityWatch>,
    activity: std::sync::Arc<std::sync::atomic::AtomicU64>,
    work: F,
) -> F::Output {
    CAPACITY_WATCH.scope((watch, activity), work).await
}

struct CapacityWait;

impl CapacityWait {
    fn new(bucket: &str) -> Self {
        let _ = CAPACITY_WATCH.try_with(|(watch, _)| {
            *watch
                .waiting
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some((
                tokio::time::Instant::now(),
                bucket.split('/').next().unwrap_or("unknown").to_string(),
            ));
        });
        Self
    }
}

impl Drop for CapacityWait {
    fn drop(&mut self) {
        let _ = CAPACITY_WATCH.try_with(|(watch, activity)| {
            *watch
                .waiting
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
            activity.store(
                chrono::Utc::now().timestamp_millis().max(0) as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PressureRetry {
    Retry {
        delay_ms: u64,
        honoured_retry_after: bool,
        attempt: u32,
        max_attempts: u32,
    },
    GiveUp {
        cause: String,
    },
}

pub fn pressure_retry(
    error: &crate::chat::stream_core::LlmStreamError,
    completed_attempts: u32,
) -> Option<PressureRetry> {
    let (bucket, report) = error.pressure_failure.as_ref()?;
    match retry_decision(bucket, report, completed_attempts)? {
        RetryDecision::Retry {
            delay_ms,
            honoured_retry_after,
            attempt,
            ..
        } => Some(PressureRetry::Retry {
            delay_ms,
            honoured_retry_after,
            attempt,
            max_attempts: refact_providers::pressure::pressure_registry()?
                .config()
                .max_retry_attempts,
        }),
        RetryDecision::GiveUp { cause, .. } => Some(PressureRetry::GiveUp { cause }),
    }
}

pub fn retry_not_before_ms(bucket: &str, report: Option<&FailureReport>) -> Option<u64> {
    let registry = pressure_registry();
    let now_ms = registry
        .as_ref()
        .map(|registry| registry.now_ms())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis().max(0) as u64);
    let own = report
        .and_then(|report| report.retry_after.as_deref())
        .and_then(|raw| refact_providers::pressure::retry_after_delay_ms(raw, now_ms))
        .map(|delay_ms| now_ms.saturating_add(delay_ms));
    let shared = registry
        .as_ref()
        .and_then(|registry| registry.snapshot(bucket))
        .and_then(|snapshot| {
            let breaker = snapshot.breaker.map(|breaker| breaker.until_ms);
            match (snapshot.retry_after_until_ms, breaker) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            }
        });
    let until_ms = match (registry.is_some(), shared, own) {
        (true, Some(shared), _) => Some(shared),
        (true, None, _) => None,
        (false, _, own) => own,
    }?;
    (until_ms > now_ms).then_some(until_ms)
}

fn counter(field: &str, value: usize) -> Result<u32, String> {
    u32::try_from(value)
        .map_err(|_| format!("coordination.{field} does not fit a 32-bit counter: {value}"))
}

pub fn pressure_config(settings: &CoordinationSettings) -> Result<PressureConfig, String> {
    let config = PressureConfig {
        max_concurrency: counter(
            "provider_max_concurrency",
            settings.provider_max_concurrency,
        )?,
        min_concurrency: counter(
            "provider_min_concurrency",
            settings.provider_min_concurrency,
        )?,
        initial_concurrency: counter(
            "provider_initial_concurrency",
            settings.provider_initial_concurrency,
        )?,
        interactive_reserved_slots: counter(
            "provider_interactive_reserved_slots",
            settings.provider_interactive_reserved_slots,
        )?,
        increase_after_successes: counter(
            "provider_aimd_increase_after_successes",
            settings.provider_aimd_increase_after_successes,
        )?,
        decrease_cooldown_ms: settings.provider_aimd_decrease_cooldown_ms,
        max_retry_attempts: counter(
            "provider_retry_max_attempts",
            settings.provider_retry_max_attempts,
        )?,
        base_backoff_ms: settings.provider_retry_base_backoff_ms,
        max_backoff_ms: settings.provider_retry_max_backoff_ms,
        jitter_per_mille: counter(
            "provider_retry_jitter_per_mille",
            settings.provider_retry_jitter_per_mille,
        )?,
        breaker_failure_threshold: counter(
            "provider_breaker_failure_threshold",
            settings.provider_breaker_failure_threshold,
        )?,
        breaker_cooldown_ms: settings.provider_breaker_cooldown_ms,
        max_retry_after_ms: settings.provider_max_retry_after_ms,
        max_buckets: settings.provider_pressure_max_buckets,
    };
    config.validate()?;
    Ok(config)
}

/// Settings saves reconfigure the live registry without discarding provider bucket state.
pub fn install_provider_pressure(settings: &CoordinationSettings) -> Result<(), String> {
    if !settings.provider_pressure_enabled {
        uninstall_pressure_registry();
        return Ok(());
    }
    install_pressure_registry(pressure_config(settings)?).map(|_| ())
}

/// Ask the controller for a slot and record what it decided. A deferral names
/// the bucket's own numbers and the rule that refused it; an admission samples
/// the permit counts the controller already tracks.
pub fn admit(bucket: &str, priority: RequestPriority) -> Option<Result<Permit, AdmissionRefusal>> {
    let registry = pressure_registry()?;
    let decided = registry.try_admit(bucket, priority);
    let priority_label = match priority {
        RequestPriority::Interactive => "interactive",
        RequestPriority::Background => "background",
    };
    match &decided {
        Ok(_) => metrics::count(
            metrics::PERMITS,
            vec![
                ("bucket", bucket.to_string()),
                ("kind", "granted".to_string()),
            ],
        ),
        Err(refusal) => {
            let (rule, reason) = match refusal {
                AdmissionRefusal::CoolingDown { reason, until_ms } => (
                    "provider_pressure.breaker_cooling_down",
                    format!("{reason} until {until_ms}"),
                ),
                AdmissionRefusal::AtCapacity { limit, in_flight } => (
                    "provider_pressure.bucket_at_capacity",
                    format!("{in_flight} in flight of {limit}"),
                ),
                AdmissionRefusal::BackgroundReserved {
                    background_limit,
                    background_in_flight,
                } => (
                    "provider_pressure.interactive_slots_reserved",
                    format!("{background_in_flight} background in flight of {background_limit}"),
                ),
            };
            metrics::count(
                metrics::PERMITS,
                vec![
                    ("bucket", bucket.to_string()),
                    ("kind", "deferred".to_string()),
                ],
            );
            metrics::explain(Explanation::new(
                Decision::BackpressureDeferral,
                "deferred",
                rule,
                vec![
                    ("reason".to_string(), reason),
                    ("bucket".to_string(), bucket.to_string()),
                    ("priority".to_string(), priority_label.to_string()),
                ],
            ));
        }
    }
    metrics::sample_provider_pressure();
    Some(decided)
}

pub fn admit_recheck(
    bucket: &str,
    priority: RequestPriority,
) -> Option<Result<Permit, AdmissionRefusal>> {
    let registry = pressure_registry()?;
    let decided = registry.try_admit(bucket, priority);
    if decided.is_ok() {
        metrics::count(
            metrics::PERMITS,
            vec![
                ("bucket", bucket.to_string()),
                ("kind", "granted".to_string()),
            ],
        );
        metrics::sample_provider_pressure();
    }
    Some(decided)
}

pub async fn acquire(
    bucket: &str,
    priority: RequestPriority,
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    abort_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<Option<Permit>, String> {
    acquire_inner(bucket, priority, abort_flag, abort_notify, None)
        .await
        .map_err(|error| match error {
            AcquireError::Aborted(message) => message,
            AcquireError::Refused(refusal) => format!("Provider admission refused: {refusal:?}"),
        })
}

#[derive(Debug)]
pub enum AcquireError {
    Aborted(String),
    Refused(AdmissionRefusal),
}

pub async fn acquire_for_recovery(
    bucket: &str,
    priority: RequestPriority,
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    abort_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
    budget: &std::sync::Arc<refact_core::recovery::RecoveryBudget>,
) -> Result<Option<Permit>, AcquireError> {
    acquire_inner(bucket, priority, abort_flag, abort_notify, Some(budget)).await
}

async fn acquire_inner(
    bucket: &str,
    priority: RequestPriority,
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    abort_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
    budget: Option<&std::sync::Arc<refact_core::recovery::RecoveryBudget>>,
) -> Result<Option<Permit>, AcquireError> {
    let Some(first) = admit(bucket, priority) else {
        return Ok(None);
    };
    let mut refusal = match first {
        Ok(permit) => return Ok(Some(permit)),
        Err(refusal) => refusal,
    };
    if let Some(budget) = budget {
        budget.start_recovery();
    }
    let _waiting = CapacityWait::new(bucket);
    let mut backoff_ms = 25u64;

    loop {
        if budget.is_some() && matches!(refusal, AdmissionRefusal::CoolingDown { .. }) {
            return Err(AcquireError::Refused(refusal));
        }
        match refusal {
            AdmissionRefusal::CoolingDown { reason, until_ms } => {
                let registry = pressure_registry().ok_or_else(|| {
                    AcquireError::Aborted(
                        "provider pressure registry was removed while waiting".into(),
                    )
                })?;
                if registry
                    .snapshot(bucket)
                    .is_some_and(|snapshot| snapshot.breaker.is_some())
                {
                    return Err(AcquireError::Aborted(format!(
                        "provider bucket {bucket} is blocked: {reason} until {until_ms}"
                    )));
                }
                let remaining_ms = until_ms.saturating_sub(registry.now_ms());
                if remaining_ms > registry.config().max_backoff_ms {
                    return Err(AcquireError::Aborted(format!(
                        "provider bucket {bucket} is cooling down: {reason} until {until_ms} (longer than the {} ms admission wait bound)",
                        registry.config().max_backoff_ms
                    )));
                }
                wait_abortable(
                    Duration::from_millis(remaining_ms.max(25).min(backoff_ms)),
                    abort_flag.clone(),
                    abort_notify.clone(),
                )
                .await
                .map_err(AcquireError::Aborted)?;
                backoff_ms = (backoff_ms * 2).min(500);
            }
            AdmissionRefusal::AtCapacity { .. } | AdmissionRefusal::BackgroundReserved { .. } => {
                wait_abortable(
                    Duration::from_millis(backoff_ms),
                    abort_flag.clone(),
                    abort_notify.clone(),
                )
                .await
                .map_err(AcquireError::Aborted)?;
                backoff_ms = (backoff_ms * 2).min(500);
            }
        }

        if abort_flag
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        {
            return Err(AcquireError::Aborted("Aborted".to_string()));
        }
        let Some(next) = admit_recheck(bucket, priority) else {
            return Ok(None);
        };
        match next {
            Ok(permit) => return Ok(Some(permit)),
            Err(next_refusal) => refusal = next_refusal,
        }
    }
}

async fn wait_abortable(
    duration: std::time::Duration,
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    abort_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    if abort_flag
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::SeqCst))
    {
        return Err("Aborted".to_string());
    }
    let deadline = tokio::time::Instant::now() + duration;
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_millis(50));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if abort_flag
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            return Err("Aborted".to_string());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        if let Some(notify) = abort_notify.as_ref() {
            let notified = notify.notified();
            tokio::pin!(notified);
            tokio::select! {
                _ = tokio::time::sleep(remaining) => return Ok(()),
                _ = &mut notified => return Err("Aborted".to_string()),
                _ = heartbeat.tick() => {},
            }
        } else {
            tokio::select! {
                _ = tokio::time::sleep(remaining) => return Ok(()),
                _ = heartbeat.tick() => {},
            }
        }
    }
}

/// Ask the controller whether to retry and record the rule behind the answer.
pub fn retry_decision(
    bucket: &str,
    report: &FailureReport,
    completed_attempts: u32,
) -> Option<RetryDecision> {
    let registry = pressure_registry()?;
    let decided = registry.retry_decision(report, completed_attempts);
    let (outcome, rule, reason) = match &decided {
        RetryDecision::Retry {
            class,
            attempt,
            delay_ms,
            honoured_retry_after,
        } => (
            "retry",
            match honoured_retry_after {
                true => "provider_pressure.honoured_retry_after",
                false => "provider_pressure.retryable_class_backoff",
            },
            format!(
                "attempt {attempt} of class {} waits {delay_ms}ms",
                class.as_str()
            ),
        ),
        RetryDecision::GiveUp {
            class,
            attempts,
            cause,
        } => (
            "give_up",
            match class.retryable() {
                true => "provider_pressure.retry_attempts_exhausted",
                false => "provider_pressure.class_not_retryable",
            },
            format!("{} after {attempts} attempts: {cause}", class.as_str()),
        ),
    };
    metrics::explain(Explanation::new(
        Decision::Retry,
        outcome,
        rule,
        vec![
            ("reason".to_string(), reason),
            ("bucket".to_string(), bucket.to_string()),
        ],
    ));
    Some(decided)
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_providers::pressure::pressure_registry;
    use serial_test::serial;

    #[test]
    #[serial(provider_pressure, runtime_settings)]
    fn enabling_installs_a_registry_and_disabling_removes_it() {
        let settings = CoordinationSettings::default();
        install_provider_pressure(&settings).unwrap();

        let registry = pressure_registry().unwrap();
        assert_eq!(
            registry.config().max_concurrency as usize,
            settings.provider_max_concurrency
        );

        let mut disabled = CoordinationSettings::default();
        disabled.provider_pressure_enabled = false;
        install_provider_pressure(&disabled).unwrap();
        assert!(pressure_registry().is_none());
    }

    #[tokio::test(start_paused = true)]
    #[serial(provider_pressure, coordination_metrics, runtime_settings)]
    async fn waiting_for_capacity_is_abortable() {
        uninstall_pressure_registry();
        let mut settings = CoordinationSettings::default();
        settings.provider_initial_concurrency = 1;
        settings.provider_min_concurrency = 1;
        settings.provider_max_concurrency = 1;
        settings.provider_interactive_reserved_slots = 0;
        install_provider_pressure(&settings).unwrap();

        let bucket = "abort-wait/credential";
        let permit = admit(bucket, RequestPriority::Interactive)
            .unwrap()
            .unwrap();
        let abort_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let abort_notify = std::sync::Arc::new(tokio::sync::Notify::new());
        let waiting_flag = abort_flag.clone();
        let waiting_notify = abort_notify.clone();
        let waiter = tokio::spawn(async move {
            acquire(
                bucket,
                RequestPriority::Interactive,
                Some(waiting_flag),
                Some(waiting_notify),
            )
            .await
        });
        tokio::task::yield_now().await;
        abort_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        abort_notify.notify_one();
        assert_eq!(waiter.await.unwrap().unwrap_err(), "Aborted");
        drop(permit);
        uninstall_pressure_registry();
    }
}
