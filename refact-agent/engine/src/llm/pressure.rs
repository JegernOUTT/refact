//! Bridges coordination settings onto the provider pressure controller.

use refact_providers::pressure::{
    install_pressure_registry, uninstall_pressure_registry, PressureConfig,
};
use refact_work_core::CoordinationSettings;

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
