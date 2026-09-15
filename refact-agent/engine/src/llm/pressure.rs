//! Bridges coordination settings onto the provider pressure controller, and is
//! the seam where admission and retry decisions become measurable.

use refact_providers::pressure::{
    install_pressure_registry, pressure_registry, uninstall_pressure_registry, AdmissionRefusal,
    FailureReport, Permit, PressureConfig, RequestPriority, RetryDecision,
};
use refact_work_core::CoordinationSettings;

use crate::coordination::metrics::{self, Decision, Explanation};

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
                    format!(
                        "{background_in_flight} background in flight of {background_limit}"
                    ),
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
    fn defaults_map_onto_a_valid_controller_config() {
        let settings = CoordinationSettings::default();
        let config = pressure_config(&settings).unwrap();

        assert_eq!(
            config.max_concurrency as usize,
            settings.provider_max_concurrency
        );
        assert_eq!(
            config.min_concurrency as usize,
            settings.provider_min_concurrency
        );
        assert_eq!(
            config.initial_concurrency as usize,
            settings.provider_initial_concurrency
        );
        assert_eq!(
            config.interactive_reserved_slots as usize,
            settings.provider_interactive_reserved_slots
        );
        assert_eq!(
            config.increase_after_successes as usize,
            settings.provider_aimd_increase_after_successes
        );
        assert_eq!(
            config.decrease_cooldown_ms,
            settings.provider_aimd_decrease_cooldown_ms
        );
        assert_eq!(
            config.max_retry_attempts as usize,
            settings.provider_retry_max_attempts
        );
        assert_eq!(
            config.base_backoff_ms,
            settings.provider_retry_base_backoff_ms
        );
        assert_eq!(
            config.max_backoff_ms,
            settings.provider_retry_max_backoff_ms
        );
        assert_eq!(
            config.jitter_per_mille as usize,
            settings.provider_retry_jitter_per_mille
        );
        assert_eq!(
            config.breaker_failure_threshold as usize,
            settings.provider_breaker_failure_threshold
        );
        assert_eq!(
            config.breaker_cooldown_ms,
            settings.provider_breaker_cooldown_ms
        );
        assert_eq!(
            config.max_retry_after_ms,
            settings.provider_max_retry_after_ms
        );
        assert_eq!(config.max_buckets, settings.provider_pressure_max_buckets);
    }

    #[test]
    fn invalid_settings_are_rejected_instead_of_coerced() {
        let mut settings = CoordinationSettings::default();
        settings.provider_initial_concurrency = settings.provider_max_concurrency + 1;
        assert!(pressure_config(&settings).is_err());

        let mut settings = CoordinationSettings::default();
        settings.provider_max_concurrency = usize::MAX;
        let error = pressure_config(&settings).unwrap_err();
        assert!(
            error.contains("coordination.provider_max_concurrency"),
            "unexpected error: {error}"
        );
    }

    #[test]
    #[serial(provider_pressure)]
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
}
