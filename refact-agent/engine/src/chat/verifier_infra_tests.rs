use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use crate::chat::evidence::{
    EvidenceContext, EvidenceDefaults, EvidenceRunner, EvidenceService, EvidenceVerdict,
    VerificationSpec,
};
use crate::chat::verify_cmd::VerifyCommandPolicy;
use crate::tasks::types::{
    BoardCard, FinalReport, VerificationOutcome, VerificationResult, VerifierReportClassification,
};

struct MarkerRunner {
    reported_command: String,
    marker: PathBuf,
    calls: Vec<String>,
}

#[async_trait]
impl EvidenceRunner for MarkerRunner {
    async fn run(&mut self, _worktree: &Path, spec: &VerificationSpec) -> VerificationResult {
        self.calls.push(spec.command.clone());
        if spec.command == self.reported_command {
            tokio::fs::write(&self.marker, "executed").await.unwrap();
        }
        VerificationResult {
            command: spec.command.clone(),
            exit_code: Some(0),
            passed: true,
            output_tail: "ok".to_string(),
            outcome: VerificationOutcome::Passed,
            from_cache: false,
            executed_at: None,
        }
    }
}

fn card(instructions: &str) -> BoardCard {
    let mut card: BoardCard = serde_json::from_value(serde_json::json!({
        "id": "L-11b",
        "title": "Verifier infrastructure classification",
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

fn report_classification(verdict: EvidenceVerdict) -> VerifierReportClassification {
    let gates = crate::chat::evidence::gate_outcome_for_verdicts(&[(true, verdict)]);
    crate::chat::verifier::classify_verifier_report(&gates, false, false)
}

#[test]
fn timeout_only_report_is_not_a_failure() {
    assert_eq!(
        crate::chat::verifier::verification_outcome_for_exit(
            None,
            "command timed out after 600 seconds"
        ),
        VerificationOutcome::InfrastructureFailed
    );
    assert_eq!(
        report_classification(EvidenceVerdict::Unavailable),
        VerifierReportClassification::Unavailable
    );
    assert_eq!(
        crate::chat::verifier::verification_outcome_for_exit(
            Some(0),
            "Blocking waiting for file lock on build directory"
        ),
        VerificationOutcome::InfrastructureFailed
    );
}

#[test]
fn missing_cargo_toml_is_infrastructure() {
    let output = "error: could not find `Cargo.toml` in /worktree or any parent directory";
    assert!(crate::chat::verifier::verification_failure_is_infrastructure(output));
    assert_eq!(
        crate::chat::verifier::verification_outcome_for_exit(Some(101), output),
        VerificationOutcome::InfrastructureFailed
    );
}

#[tokio::test]
async fn agent_reported_commands_are_not_executed() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("agent-reported-command-ran");
    let reported_command = format!("touch {}", marker.display());
    let mut card = card("## Verification\n- Verify: `cargo check`");
    card.final_report_structured = Some(FinalReport {
        verification: vec![VerificationResult {
            command: reported_command.clone(),
            passed: true,
            ..Default::default()
        }],
        ..Default::default()
    });
    let specs = crate::tasks::assignment::specs_for_validation(
        None,
        &card,
        &EvidenceDefaults {
            timeout_secs: 60,
            output_cap_bytes: 4_000,
        },
    );
    let mut runner = MarkerRunner {
        reported_command,
        marker: marker.clone(),
        calls: Vec::new(),
    };
    let service = EvidenceService::new(
        VerifyCommandPolicy::permissive(),
        EvidenceContext {
            worktree: temp.path().to_path_buf(),
            commit: None,
            scope: card.id.clone(),
        },
    );

    let records = service.accept_and_execute_all(specs, &mut runner).await;

    assert_eq!(runner.calls, vec!["cargo check"]);
    assert_eq!(records.len(), 1);
    assert!(!marker.exists());
}

#[test]
fn skip_gui_build_env_is_inherited_when_the_worktree_was_built_with_it() {
    let env = crate::chat::verifier::verifier_command_env("cargo", Vec::new(), Some("1"));
    assert_eq!(
        env.get("REFACT_SKIP_GUI_BUILD").map(String::as_str),
        Some("1")
    );
    assert_eq!(env.get("CARGO_INCREMENTAL").map(String::as_str), Some("0"));
    assert_eq!(
        env.get("RUST_MIN_STACK").map(String::as_str),
        Some("16777216")
    );

    let explicit = crate::chat::verifier::verifier_command_env(
        "cargo",
        vec![
            ("CARGO_INCREMENTAL".to_string(), "1".to_string()),
            ("RUST_MIN_STACK".to_string(), "64".to_string()),
        ],
        Some("1"),
    );
    assert_eq!(
        explicit.get("CARGO_INCREMENTAL").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        explicit.get("RUST_MIN_STACK").map(String::as_str),
        Some("64")
    );
}

#[test]
fn explicit_skip_gui_build_on_the_verify_line_wins() {
    for (explicit, default) in [("1", "0"), ("0", "1")] {
        let env = crate::chat::verifier::verifier_command_env(
            "cargo",
            vec![("REFACT_SKIP_GUI_BUILD".to_string(), explicit.to_string())],
            Some(default),
        );
        assert_eq!(
            env.get("REFACT_SKIP_GUI_BUILD").map(String::as_str),
            Some(explicit)
        );
    }
}

#[test]
fn skip_gui_build_default_comes_from_the_worktree_build_output() {
    let temp = tempfile::tempdir().unwrap();
    let build_dir = temp.path().join("target/debug/build");
    let older_output = build_dir.join("refact-lsp-aaa/output");
    fs::create_dir_all(older_output.parent().unwrap()).unwrap();
    fs::write(&older_output, "cargo:rustc-env=REFACT_SKIP_GUI_BUILD=1\n").unwrap();
    let newer_output = build_dir.join("refact-lsp-bbb/output");
    fs::create_dir_all(newer_output.parent().unwrap()).unwrap();
    fs::write(&newer_output, "cargo:rustc-env=REFACT_SKIP_GUI_BUILD=0\n").unwrap();
    fs::File::open(&older_output)
        .unwrap()
        .set_modified(SystemTime::now())
        .unwrap();
    fs::File::open(&newer_output)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(10))
        .unwrap();

    assert_eq!(
        crate::chat::verifier::worktree_skip_gui_build(temp.path(), &HashMap::new()).as_deref(),
        Some("0")
    );
}

#[test]
fn no_build_output_leaves_the_variable_unset() {
    let temp = tempfile::tempdir().unwrap();
    let target_dir = temp.path().join("empty-target");
    let command_env = HashMap::from([(
        "CARGO_TARGET_DIR".to_string(),
        target_dir.to_string_lossy().to_string(),
    )]);
    let default = crate::chat::verifier::worktree_skip_gui_build(temp.path(), &command_env);
    let env = crate::chat::verifier::verifier_command_env("cargo", Vec::new(), default.as_deref());

    assert!(!env.contains_key("REFACT_SKIP_GUI_BUILD"));
}

#[test]
fn cargo_target_dir_is_respected() {
    let temp = tempfile::tempdir().unwrap();
    let target_dir = temp.path().join("external-target");
    let output = target_dir.join("debug/build/refact-lsp-aaa/output");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, "cargo:rustc-env=REFACT_SKIP_GUI_BUILD=1\n").unwrap();
    let command_env = HashMap::from([(
        "CARGO_TARGET_DIR".to_string(),
        target_dir.to_string_lossy().to_string(),
    )]);

    assert_eq!(
        crate::chat::verifier::worktree_skip_gui_build(temp.path(), &command_env).as_deref(),
        Some("1")
    );
}

#[test]
fn genuine_failure_still_fails() {
    assert_eq!(
        crate::chat::verifier::verification_outcome_for_exit(Some(1), "test assertion failed"),
        VerificationOutcome::CommandFailed
    );
    assert_eq!(
        report_classification(EvidenceVerdict::Failed),
        VerifierReportClassification::VerificationFailed
    );
    assert_eq!(
        crate::chat::verifier::classify_verifier_report(
            &crate::chat::evidence::gate_outcome_for_verdicts(&[
                (true, EvidenceVerdict::Unavailable),
                (true, EvidenceVerdict::Failed),
            ]),
            false,
            false,
        ),
        VerifierReportClassification::VerificationFailed
    );
}
