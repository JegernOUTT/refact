use refact_work_core::{coordination_settings, install_coordination_settings, CoordinationSettings};

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
        max_outstanding: 4096,
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
