use super::evidence::{
    evaluate_gates, instruction_specs, EvidenceContext, EvidenceDefaults, EvidenceService,
    EvidenceVerdict, SystemEvidenceRunner,
};
use super::verifier::classify_verifier_report;
use super::verify_cmd::{instruction_commands, instruction_skip_notes, VerifyCommandPolicy};
use crate::tasks::types::{
    BoardCard, VerificationOutcome, VerifierReport, VerifierReportClassification,
};
use crate::tools::tool_task_merge_agent::ensure_verifier_allows_merge;

fn defaults() -> EvidenceDefaults {
    EvidenceDefaults {
        timeout_secs: 10,
        output_cap_bytes: 4000,
    }
}

fn card_with_report(report: VerifierReport) -> BoardCard {
    let mut card: BoardCard = serde_json::from_value(serde_json::json!({
        "id": "H-7", "title": "Verify lines", "column": "done",
        "created_at": "now", "started_at": null, "completed_at": null,
        "assignee": null, "agent_chat_id": null
    }))
    .unwrap();
    card.verifier_report = Some(report);
    card
}

#[cfg(unix)]
#[tokio::test]
async fn two_backticked_commands_on_one_verify_line_both_run() {
    let temp = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let mut runner = SystemEvidenceRunner {
        gcx,
    };
    let service = EvidenceService::new(
        VerifyCommandPolicy::permissive(),
        EvidenceContext {
            worktree: temp.path().to_path_buf(),
            ..Default::default()
        },
    );
    for separator in [" and ", ", "] {
        let instructions = format!("- [ ] Verify: `echo first`{separator}`echo second`");
        let specs = instruction_specs("H-7", &instructions, &defaults());
        assert_eq!(specs.len(), 2, "both commands must be extracted");
        let records = service.accept_and_execute_all(specs, &mut runner).await;
        assert_eq!(records.len(), 2);
        for (record, expected) in records.iter().zip(["first", "second"]) {
            assert_eq!(record.verdict, EvidenceVerdict::Passed, "{record:?}");
            assert_eq!(record.exit_code, Some(0));
            assert_eq!(record.output_tail.trim(), expected);
        }
    }
}

#[test]
fn unparseable_verify_line_is_reported_not_dropped() {
    for line in [
        "- Verify: run the checks",
        "- Verify: `unclosed",
        "- Verify: ``",
    ] {
        assert!(instruction_commands(line).is_empty());
        assert_eq!(
            instruction_skip_notes(line),
            vec![format!("Verify line not executable: {line}")]
        );
    }
}

#[test]
fn never_run_acceptance_check_is_unavailable_not_failed() {
    let specs = instruction_specs("H-7", "- Verify: `cargo check`", &defaults());
    let gates = evaluate_gates(&specs, &[]);
    assert_eq!(gates.blocks[0].verdict, EvidenceVerdict::NotRun);
    assert!(gates.blocks[0].reason.starts_with("not run:"));
    assert!(!gates.has_contract_violation());
    let classification = classify_verifier_report(&gates, false, false);
    assert_eq!(classification, VerifierReportClassification::Unavailable);
    let card = card_with_report(VerifierReport {
        classification,
        concerns: gates.reasons(),
        ..Default::default()
    });
    assert!(ensure_verifier_allows_merge(&card, false).is_err());
    assert!(ensure_verifier_allows_merge(&card, true).is_ok());
}

#[test]
fn superseded_failed_evidence_is_unavailable_not_a_current_failure() {
    let spec = instruction_specs("H-7", "- Verify: `cargo test`", &defaults()).remove(0);
    let mut record = super::evidence::unexecuted_record(
        &spec,
        EvidenceVerdict::Superseded,
        VerificationOutcome::CommandFailed,
        "inputs changed since execution".to_string(),
    );
    record.exit_code = Some(1);
    let gates = evaluate_gates(&[spec], &[record.clone()]);
    let classification = classify_verifier_report(&gates, false, false);
    assert_eq!(classification, VerifierReportClassification::Unavailable);
    let card = card_with_report(VerifierReport {
        classification,
        command_results: vec![record.to_verification_result()],
        concerns: gates.reasons(),
        ..Default::default()
    });
    assert!(ensure_verifier_allows_merge(&card, true).is_ok());
}

#[test]
fn all_commands_passed_plus_judge_concerns_is_needs_review_and_force_mergeable() {
    let gates = super::evidence::gate_outcome_for_verdicts(&[(true, EvidenceVerdict::Passed)]);
    let classification = classify_verifier_report(&gates, false, true);
    assert_eq!(classification, VerifierReportClassification::HumanReview);
    let concerns = vec![
        "ADVISORY: not run: check absent from card".to_string(),
        "not executed: negative-control step".to_string(),
    ];
    let card = card_with_report(VerifierReport {
        classification,
        command_results: vec![crate::tasks::types::VerificationResult {
            command: "cargo test".to_string(),
            exit_code: Some(0),
            passed: true,
            outcome: VerificationOutcome::Passed,
            ..Default::default()
        }],
        concerns: concerns.clone(),
        ..Default::default()
    });
    assert!(ensure_verifier_allows_merge(&card, false).is_err());
    let warning = ensure_verifier_allows_merge(&card, true).unwrap().unwrap();
    for concern in concerns {
        assert!(warning.contains(&concern), "{warning}");
    }
    assert_eq!(
        classify_verifier_report(&gates, false, false),
        VerifierReportClassification::Passed
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ran_and_failed_command_is_still_a_genuine_failure_force_cannot_bypass() {
    let temp = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let mut runner = SystemEvidenceRunner {
        gcx,
    };
    let service = EvidenceService::new(
        VerifyCommandPolicy::permissive(),
        EvidenceContext {
            worktree: temp.path().to_path_buf(),
            ..Default::default()
        },
    );
    let specs = instruction_specs("H-7", "- Verify: `false`", &defaults());
    let evidence = service
        .accept_and_execute_all(specs.clone(), &mut runner)
        .await;
    assert_eq!(evidence[0].verdict, EvidenceVerdict::Failed);
    assert_eq!(evidence[0].exit_code, Some(1));
    let gates = evaluate_gates(&specs, &evidence);
    let classification = classify_verifier_report(&gates, false, true);
    assert_eq!(
        classification,
        VerifierReportClassification::VerificationFailed
    );
    let mut card = card_with_report(VerifierReport {
        classification,
        command_results: evidence
            .iter()
            .map(|record| record.to_verification_result())
            .collect(),
        concerns: gates.reasons(),
        ..Default::default()
    });
    for classification in [classification, VerifierReportClassification::HumanReview] {
        card.verifier_report.as_mut().unwrap().classification = classification;
        let error = ensure_verifier_allows_merge(&card, true).unwrap_err();
        assert!(
            error.contains("force=true cannot bypass verification failures"),
            "{error}"
        );
    }
}
