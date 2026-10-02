use crate::chat::evidence::EvidenceDefaults;
use crate::chat::verify_cmd::{instruction_commands, instruction_skip_notes, parse_verification_argv};
use crate::tasks::types::BoardCard;

fn card(instructions: &str) -> BoardCard {
    let mut card: BoardCard = serde_json::from_value(serde_json::json!({
        "id": "L-11",
        "title": "Verifier false-fail control",
        "column": "done",
        "instructions": instructions,
        "created_at": "2026-10-02T00:00:00Z",
        "started_at": null,
        "completed_at": null,
        "assignee": null,
        "agent_chat_id": null
    }))
    .unwrap();
    card.instructions = instructions.to_string();
    card
}

#[test]
fn negative_control_steps_are_not_executed() {
    let instructions = "## Verification\n- Verify: `cargo test --lib -- chat::verifier_false_fail_tests`\n\n## Negative control\nTemporarily remove the gate, run `cargo test --lib -- chat::verifier_false_fail_tests`, paste the failure, then restore it.\n";
    let recorded_runs = Vec::<String>::new();
    assert_eq!(
        instruction_commands(instructions),
        vec!["cargo test --lib -- chat::verifier_false_fail_tests"]
    );
    assert!(recorded_runs.is_empty());
    assert_eq!(
        instruction_skip_notes(instructions),
        vec!["not executed: negative-control step"]
    );
}

#[test]
fn only_verify_lines_are_scraped() {
    let card = card(
        "## Instructions\nRun `cargo test --lib` when useful.\n\n## Verification\n- Verify: `cargo check --lib`\n- cargo test --lib\n",
    );
    let specs = crate::chat::evidence::specs_from_card(
        &card,
        &EvidenceDefaults {
            timeout_secs: 60,
            output_cap_bytes: 4_000,
        },
    );
    assert_eq!(
        specs
            .into_iter()
            .map(|spec| spec.command)
            .collect::<Vec<_>>(),
        vec!["cargo check --lib"]
    );
}

#[test]
fn double_dash_test_threads_is_normalised_or_flagged() {
    let command = "cargo test --lib -- chat::verifier_false_fail_tests -- --test-threads=1";
    let note = parse_verification_argv(command).unwrap_err();
    assert!(note.contains("malformed cargo test separators"));
    assert!(note.contains("command not executed"));
    let normalized = parse_verification_argv(
        "cargo test --lib chat::verifier_false_fail_tests -- --test-threads=1",
    )
    .unwrap();
    assert_eq!(
        normalized.argv,
        vec![
            "cargo",
            "test",
            "--lib",
            "chat::verifier_false_fail_tests",
            "--",
            "--test-threads=1"
        ]
    );
}

#[test]
fn missing_verifier_model_is_unavailable_not_failed_and_merge_force_accepts_it() {
    let report = crate::chat::verifier::unavailable_verifier_report(
        "verifier model 'openai_codex_personal/gpt-5.6-luna' is not available".to_string(),
    );
    assert!(!report.passed);
    assert_eq!(
        report.classification,
        crate::tasks::types::VerifierReportClassification::Unavailable
    );
    assert!(report.concerns[0].contains("infrastructure unavailable"));

    let mut card = card("");
    card.verifier_report = Some(report);
    assert!(
        crate::tools::tool_task_merge_agent::ensure_verifier_allows_merge(&card, false).is_err()
    );
    assert!(crate::tools::tool_task_merge_agent::ensure_verifier_allows_merge(&card, true).is_ok());
}

#[test]
fn unavailable_verifier_is_not_a_genuine_verification_failure() {
    let report = crate::chat::verifier::unavailable_verifier_report(
        "no light/default model configured for verifier".to_string(),
    );
    assert_eq!(
        report.classification,
        crate::tasks::types::VerifierReportClassification::Unavailable
    );
    assert!(!report
        .concerns
        .iter()
        .any(|concern| concern.contains("verification failed")));
}
