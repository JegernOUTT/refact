use refact_providers::pressure::{
    pressure_registry, uninstall_pressure_registry, FailureReport, Outcome, RequestPriority,
};
use refact_work_core::CoordinationSettings;
use serial_test::serial;

fn fail(bucket: &str, report: FailureReport) {
    pressure_registry()
        .unwrap()
        .try_admit(bucket, RequestPriority::Interactive)
        .unwrap()
        .complete(Outcome::Failure(report));
}

#[test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
fn reinstall_with_same_settings_preserves_cooldown_and_breaker() {
    uninstall_pressure_registry();
    let settings = CoordinationSettings {
        provider_breaker_failure_threshold: 2,
        ..CoordinationSettings::default()
    };
    crate::llm::pressure::install_provider_pressure(&settings).unwrap();
    fail(
        "provider/b",
        FailureReport::from_http(429, Some("30"), "rate limit").unwrap(),
    );
    fail(
        "provider/c",
        FailureReport::from_http(503, None, "unavailable").unwrap(),
    );
    fail(
        "provider/c",
        FailureReport::from_http(503, None, "unavailable").unwrap(),
    );

    let before_b = pressure_registry().unwrap().snapshot("provider/b").unwrap();
    let before_c = pressure_registry().unwrap().snapshot("provider/c").unwrap();
    let before_registry = pressure_registry().unwrap();
    crate::llm::pressure::install_provider_pressure(&settings).unwrap();

    let registry = pressure_registry().unwrap();
    assert_eq!(
        registry
            .snapshot("provider/b")
            .unwrap()
            .retry_after_until_ms,
        before_b.retry_after_until_ms
    );
    assert_eq!(
        registry.snapshot("provider/c").unwrap().breaker,
        before_c.breaker
    );
    assert!(std::sync::Arc::ptr_eq(&before_registry, &registry));
    uninstall_pressure_registry();
}

#[test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
fn reconfigure_applies_new_limits_and_keeps_state() {
    uninstall_pressure_registry();
    let initial = CoordinationSettings {
        provider_initial_concurrency: 8,
        provider_max_concurrency: 10,
        ..CoordinationSettings::default()
    };
    crate::llm::pressure::install_provider_pressure(&initial).unwrap();
    fail(
        "provider/b",
        FailureReport::from_http(429, Some("30"), "rate limit").unwrap(),
    );
    let before = pressure_registry().unwrap().snapshot("provider/b").unwrap();
    assert_eq!(before.limit, 4);

    let changed = CoordinationSettings {
        provider_initial_concurrency: 3,
        provider_max_concurrency: 3,
        provider_max_retry_after_ms: 5_000,
        ..initial
    };
    crate::llm::pressure::install_provider_pressure(&changed).unwrap();

    let registry = pressure_registry().unwrap();
    let after = registry.snapshot("provider/b").unwrap();
    assert_eq!(after.limit, 3);
    assert_eq!(
        after.consecutive_retryable_failures,
        before.consecutive_retryable_failures
    );
    assert_eq!(
        after.retry_after_until_ms,
        before
            .retry_after_until_ms
            .map(|until| until.min(registry.now_ms() + changed.provider_max_retry_after_ms))
    );
    uninstall_pressure_registry();
}

#[test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
fn in_flight_count_survives_reinstall() {
    uninstall_pressure_registry();
    let settings = CoordinationSettings::default();
    crate::llm::pressure::install_provider_pressure(&settings).unwrap();
    let registry = pressure_registry().unwrap();
    let permit = registry
        .try_admit("provider/a", RequestPriority::Interactive)
        .unwrap();

    crate::llm::pressure::install_provider_pressure(&settings).unwrap();
    assert_eq!(
        pressure_registry()
            .unwrap()
            .snapshot("provider/a")
            .unwrap()
            .in_flight,
        1
    );
    drop(permit);
    assert_eq!(
        pressure_registry()
            .unwrap()
            .snapshot("provider/a")
            .unwrap()
            .in_flight,
        0
    );
    uninstall_pressure_registry();
}

#[test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
fn runtime_settings_install_live_preserves_state() {
    uninstall_pressure_registry();
    let settings = crate::runtime_settings::TrajectoryRuntimeSettings::default();
    crate::llm::pressure::install_provider_pressure(&settings.coordination).unwrap();
    fail(
        "provider/b",
        FailureReport::from_http(429, Some("30"), "rate limit").unwrap(),
    );
    let before = pressure_registry()
        .unwrap()
        .snapshot("provider/b")
        .unwrap()
        .retry_after_until_ms;

    crate::runtime_settings::install_live(&settings);

    assert_eq!(
        pressure_registry()
            .unwrap()
            .snapshot("provider/b")
            .unwrap()
            .retry_after_until_ms,
        before
    );
    crate::runtime_settings::reset_for_test();
    uninstall_pressure_registry();
}
