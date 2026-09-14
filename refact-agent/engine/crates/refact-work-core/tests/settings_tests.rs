use refact_work_core::{coordination_settings, install_coordination_settings, CoordinationSettings};

#[test]
fn defaults_are_generous_and_valid() {
    let s = CoordinationSettings::default();
    assert!(s.enabled);
    assert!(s.auto_inject_enabled);
    assert_eq!(s.max_active_engine, 32);
    assert_eq!(s.max_active_project, 16);
    assert_eq!(s.max_active_root, 16);
    assert_eq!(s.max_pending, 1024);
    assert_eq!(s.max_outstanding, 4096);
    assert_eq!(s.max_subchat_depth, 5);
    assert_eq!(s.max_read_stamps, 16384);
    assert_eq!(s.max_write_records, 65536);
    assert_eq!(s.overview_default_rows, 50);
    assert_eq!(s.overview_max_rows, 500);
    assert_eq!(s.capsule_token_budget, 1600);
    assert_eq!(s.capsule_max_rows, 24);
    assert_eq!(s.min_interval_secs, 60);
    assert_eq!(s.debounce_secs, 5);
    assert_eq!(s.recent_window_secs, 3600);
    assert_eq!(s.lock_timeout_secs, 30);
    assert_eq!(s.lock_poll_interval_ms, 25);
    assert_eq!(s.launch_context_tokens, 24_000);
    assert_eq!(s.launch_context_messages, 1024);
    assert_eq!(s.launch_context_entries, 256);
    assert_eq!(s.launch_context_bytes, 16 * 1024 * 1024);
    assert_eq!(s.launch_source_bytes, 4 * 1024 * 1024);
    assert!(s.validate().is_ok());
}

#[test]
fn launch_bounds_require_positive_values_and_inherit_unspecified_defaults() {
    for name in [
        "launch_context_tokens",
        "launch_context_messages",
        "launch_context_entries",
        "launch_context_bytes",
        "launch_source_bytes",
    ] {
        let mut json = serde_json::to_value(CoordinationSettings::default()).unwrap();
        json[name] = serde_json::json!(0);
        let settings: CoordinationSettings = serde_json::from_value(json).unwrap();
        assert!(settings
            .validate()
            .unwrap_err()
            .violations
            .iter()
            .any(|entry| entry.field == name));
    }
    let parsed: CoordinationSettings =
        serde_json::from_str(r#"{"launch_context_tokens":48000}"#).unwrap();
    assert_eq!(parsed.launch_context_tokens, 48_000);
    assert_eq!(
        parsed.launch_context_bytes,
        CoordinationSettings::default().launch_context_bytes
    );
}

#[test]
fn empty_json_yields_defaults() {
    let parsed: CoordinationSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(parsed, CoordinationSettings::default());
}

#[test]
fn partial_json_overrides_only_named_fields() {
    let parsed: CoordinationSettings =
        serde_json::from_str(r#"{"max_active_engine": 4, "lock_timeout_secs": 90}"#).unwrap();
    assert_eq!(parsed.max_active_engine, 4);
    assert_eq!(parsed.lock_timeout_secs, 90);
    assert_eq!(parsed.max_pending, 1024);
}

#[test]
fn unknown_fields_are_rejected() {
    let err = serde_json::from_str::<CoordinationSettings>(r#"{"nope": 1}"#).unwrap_err();
    assert!(err.to_string().contains("unknown field"), "{err}");
}

#[test]
fn root_weights_validate_identity_and_bounded_entries() {
    let mut settings = CoordinationSettings::default();
    assert_eq!(settings.root_quantum("unlisted"), 1);
    settings.root_weights.insert("root".into(), 3);
    assert_eq!(settings.root_quantum("root"), 3);
    assert!(settings.validate().is_ok());
    settings.root_weights.insert("bad".into(), 0);
    assert!(settings.validate().is_err());
    settings.root_weights.remove("bad");
    settings.root_weights.insert(" ".into(), 1);
    assert!(settings.validate().is_err());
    settings.root_weights.remove(" ");
    settings.max_root_weight_entries = 1;
    settings.root_weights.insert("second".into(), 1);
    assert!(settings.validate().is_err());
}

#[test]
fn max_depth_alias_is_accepted() {
    let parsed: CoordinationSettings = serde_json::from_str(r#"{"max_depth": 9}"#).unwrap();
    assert_eq!(parsed.max_subchat_depth, 9);
}

#[test]
fn round_trips_through_json() {
    let s = CoordinationSettings::default();
    let text = serde_json::to_string(&s).unwrap();
    let back: CoordinationSettings = serde_json::from_str(&text).unwrap();
    assert_eq!(s, back);
}

#[test]
fn zero_caps_are_invalid() {
    let s = CoordinationSettings {
        max_active_engine: 0,
        ..CoordinationSettings::default()
    };
    let err = s.validate().unwrap_err();
    assert!(err
        .violations
        .iter()
        .any(|v| v.field == "max_active_engine"));
}

#[test]
fn default_child_budget_leaves_delegated_resume_headroom() {
    let defaults = CoordinationSettings::default();
    assert!(defaults.subagent_default_steps < defaults.subagent_max_steps);
    let invalid = CoordinationSettings {
        subagent_default_steps: defaults.subagent_max_steps + 1,
        ..defaults
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn project_cap_above_engine_cap_is_invalid() {
    let s = CoordinationSettings {
        max_active_engine: 4,
        max_active_project: 8,
        max_active_root: 8,
        ..CoordinationSettings::default()
    };
    let err = s.validate().unwrap_err();
    let fields: Vec<&str> = err.violations.iter().map(|v| v.field.as_str()).collect();
    assert!(fields.contains(&"max_active_project"));
    assert!(fields.contains(&"max_active_root"));
}

#[test]
fn pending_above_outstanding_is_invalid() {
    let s = CoordinationSettings {
        max_pending: 8192,
        ..CoordinationSettings::default()
    };
    assert!(s
        .validate()
        .unwrap_err()
        .violations
        .iter()
        .any(|v| v.field == "max_pending"));
}

#[test]
fn overview_and_interval_relationships_are_checked() {
    let s = CoordinationSettings {
        overview_default_rows: 900,
        debounce_secs: 600,
        ..CoordinationSettings::default()
    };
    let err = s.validate().unwrap_err();
    let fields: Vec<&str> = err.violations.iter().map(|v| v.field.as_str()).collect();
    assert!(fields.contains(&"overview_default_rows"));
    assert!(fields.contains(&"debounce_secs"));
}

#[test]
fn poll_interval_may_not_exceed_lock_timeout() {
    let s = CoordinationSettings {
        lock_timeout_secs: 1,
        lock_poll_interval_ms: 5000,
        ..CoordinationSettings::default()
    };
    assert!(s
        .validate()
        .unwrap_err()
        .violations
        .iter()
        .any(|v| v.field == "lock_poll_interval_ms"));
}

#[test]
fn overview_rows_clamps_requests() {
    let s = CoordinationSettings::default();
    assert_eq!(s.overview_rows(None), 50);
    assert_eq!(s.overview_rows(Some(0)), 1);
    assert_eq!(s.overview_rows(Some(120)), 120);
    assert_eq!(s.overview_rows(Some(100_000)), 500);
}

#[test]
fn install_rejects_invalid_and_keeps_previous() {
    let good = CoordinationSettings {
        max_active_engine: 7,
        max_active_project: 7,
        max_active_root: 7,
        ..CoordinationSettings::default()
    };
    let effective = install_coordination_settings(good.clone()).unwrap();
    assert_eq!(effective.max_active_engine, 7);
    assert_eq!(coordination_settings().max_active_engine, 7);

    let bad = CoordinationSettings {
        max_outstanding: 0,
        ..CoordinationSettings::default()
    };
    assert!(install_coordination_settings(bad).is_err());
    assert_eq!(coordination_settings().max_active_engine, 7);

    install_coordination_settings(CoordinationSettings::default()).unwrap();
}

#[test]
fn duration_helpers_match_fields() {
    let s = CoordinationSettings::default();
    assert_eq!(s.lock_timeout(), std::time::Duration::from_secs(30));
    assert_eq!(s.lock_poll_interval(), std::time::Duration::from_millis(25));
    assert_eq!(s.recent_window(), std::time::Duration::from_secs(3600));
}
