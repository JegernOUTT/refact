use std::path::{Path, PathBuf};

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
