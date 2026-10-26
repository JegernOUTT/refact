use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use super::goal_review::{GoalReviewOutcome, GoalReviewRequest, GoalReviewSettings, GoalReviewer};
use super::goal_verifier::{
    apply_goal_review_result, apply_goal_verdict, goal_review_attempts_used, maybe_run_goal_review,
    review_then_judge, GoalEvidence, GoalVerificationApplyOutcome, GoalVerdict, GoalVerifierReply,
};
use super::types::{
    ChatEvent, ChatSession, CriterionEvidence, CriterionVerdict, GoalAttempt, GoalBudget,
    GoalStatus, SessionState, VerdictSource,
};
use crate::chat::evidence::{EvidenceRecord, EvidenceVerdict};
use crate::chat::goal_review::{self, GoalReviewSettings as ReviewSettings};
use crate::tools::review_types::{
    ReviewDiffSummary, ReviewFinding, ReviewOutcome, ReviewReport, ReviewScopeSummary,
    ReviewSeverity, StageRun,
};
use refact_chat_api::{GoalSnapshot, SubGoalEdgeKind, SubGoalLane, SubGoalOutcome, SubGoalRollup};

const REVIEWER_MODEL: &str = "reviewer-test-model";

struct FakeReviewer {
    report: Result<ReviewReport, String>,
    calls: usize,
}

#[async_trait]
impl GoalReviewer for FakeReviewer {
    async fn review(&mut self, _request: GoalReviewRequest) -> Result<ReviewReport, String> {
        self.calls += 1;
        self.report.clone()
    }

    async fn identity(
        &self,
        _request: &GoalReviewRequest,
    ) -> Option<crate::chat::review_reuse::ReviewIdentity> {
        None
    }
}

fn settings(max_repair_attempts: usize) -> GoalReviewSettings {
    GoalReviewSettings {
        enabled: true,
        max_repair_attempts,
        spot_check_percent: 0,
    }
}

fn clean_report() -> ReviewReport {
    ReviewReport {
        depth: "normal".to_string(),
        outcome: ReviewOutcome::Reviewed,
        scope: ReviewScopeSummary::default(),
        diff: ReviewDiffSummary::default(),
        stages: vec![StageRun::ok(
            "mechanical",
            Some(REVIEWER_MODEL.to_string()),
            1,
        )],
        findings: Vec::new(),
        duration_ms: 1,
        duplicates_merged: 0,
        scratch_dir: None,
    }
}

fn blocking_report() -> ReviewReport {
    ReviewReport {
        findings: vec![ReviewFinding {
            id: "finding-1".to_string(),
            stage: "diff".to_string(),
            model: Some(REVIEWER_MODEL.to_string()),
            title: "review finding blocks completion".to_string(),
            severity: ReviewSeverity::Blocker,
            file: "src/lib.rs".to_string(),
            line_start: 3,
            line_end: 4,
            claim: "The change drops an error.".to_string(),
            evidence: "return Ok(());".to_string(),
            evidence_present: true,
            reproduction: Some("cargo test".to_string()),
            fix: Some("return the error".to_string()),
            introduced_by_diff: true,
            out_of_scope: false,
            reported_by: vec!["diff".to_string()],
            locations: Vec::new(),
            disputed: None,
        }],
        ..clean_report()
    }
}

fn reviewer(report: ReviewReport) -> FakeReviewer {
    FakeReviewer {
        report: Ok(report),
        calls: 0,
    }
}

fn blocked_lane_rollup() -> SubGoalRollup {
    SubGoalRollup {
        required_total: 1,
        required_met: 0,
        lanes: vec![SubGoalLane {
            node_id: "lane-1".to_string(),
            title: "required child".to_string(),
            kind: SubGoalEdgeKind::Required,
            owner: None,
            outcome: SubGoalOutcome::Failed,
            dispatched: true,
            unmet_criteria: vec!["child-test".to_string()],
            reason: "required child lane failed".to_string(),
        }],
    }
}

fn review_record(evidence: &GoalEvidence) -> &EvidenceRecord {
    evidence
        .records
        .iter()
        .find(|record| record.spec_id == goal_review::GOAL_REVIEW_SPEC_ID)
        .expect("goal review evidence is attached")
}

fn gaps(reply: &GoalVerifierReply) -> &[String] {
    match &reply.verdict {
        GoalVerdict::Unmet(gaps) => gaps,
        other => panic!("expected unmet, got {other:?}"),
    }
}

fn fake_finding_report() -> ReviewReport {
    ReviewReport {
        findings: vec![ReviewFinding {
            id: "finding-1".to_string(),
            stage: "diff".to_string(),
            model: Some(REVIEWER_MODEL.to_string()),
            title: "review finding blocks completion".to_string(),
            severity: ReviewSeverity::Blocker,
            file: "src/lib.rs".to_string(),
            line_start: 3,
            line_end: 4,
            claim: "The change drops an error.".to_string(),
            evidence: "return Ok(());".to_string(),
            evidence_present: true,
            reproduction: Some("cargo test".to_string()),
            fix: Some("return the error".to_string()),
            introduced_by_diff: true,
            out_of_scope: false,
            reported_by: vec!["diff".to_string()],
            locations: Vec::new(),
            disputed: None,
        }],
        ..clean_report()
    }
}

async fn review_reply(
    review: ReviewReport,
    attempts_used: usize,
    max_repair_attempts: usize,
    evidence: &mut GoalEvidence,
) -> GoalVerifierReply {
    let result = maybe_run_goal_review::<FakeReviewer, _, _>(
        &settings(max_repair_attempts),
        GoalReviewRequest::default(),
        attempts_used,
        || async move { Ok(reviewer(review)) },
    )
    .await;
    review_then_judge(evidence, &SubGoalRollup::default(), result, || async {
        Ok(GoalVerifierReply {
            verdict: GoalVerdict::Met,
            verifier_reply: "GOAL: MET".to_string(),
            tokens: 1,
            criteria_verdicts: Vec::new(),
        })
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn disabled_review_preserves_the_judge_reply_without_constructing_a_reviewer() {
    let constructors = Arc::new(AtomicUsize::new(0));
    let observed = constructors.clone();
    let result = maybe_run_goal_review::<FakeReviewer, _, _>(
        &ReviewSettings {
            enabled: false,
            max_repair_attempts: 2,
            spot_check_percent: 0,
        },
        GoalReviewRequest::default(),
        0,
        move || {
            observed.fetch_add(1, Ordering::SeqCst);
            async { Err("must not construct".to_string()) }
        },
    )
    .await;
    assert!(matches!(result.outcome, GoalReviewOutcome::Skipped { .. }));
    assert_eq!(constructors.load(Ordering::SeqCst), 0);

    let expected = GoalVerifierReply {
        verdict: GoalVerdict::Met,
        verifier_reply: "GOAL: MET\nCRITERION C1: MET".to_string(),
        tokens: 7,
        criteria_verdicts: Vec::new(),
    };
    let baseline = super::goal_verifier::judge_after_evidence(&GoalEvidence::default(), || async {
        Ok(expected.clone())
    })
    .await
    .unwrap();
    let actual = review_then_judge(
        &mut GoalEvidence::default(),
        &SubGoalRollup::default(),
        result,
        || async { Ok(expected.clone()) },
    )
    .await
    .unwrap();
    assert_eq!(actual, baseline);
}

#[tokio::test]
async fn clean_review_is_attached_and_allows_the_semantic_judge() {
    let constructors = Arc::new(AtomicUsize::new(0));
    let observed = constructors.clone();
    let mut evidence = GoalEvidence::default();
    let result = maybe_run_goal_review::<FakeReviewer, _, _>(
        &settings(2),
        GoalReviewRequest::default(),
        0,
        move || {
            observed.fetch_add(1, Ordering::SeqCst);
            async { Ok(reviewer(clean_report())) }
        },
    )
    .await;
    let judged = GoalVerifierReply {
        verdict: GoalVerdict::Met,
        verifier_reply: "GOAL: MET".to_string(),
        tokens: 5,
        criteria_verdicts: Vec::new(),
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let judge_calls = calls.clone();
    let reply = review_then_judge(
        &mut evidence,
        &SubGoalRollup::default(),
        result,
        move || {
            judge_calls.fetch_add(1, Ordering::SeqCst);
            async move { Ok(judged) }
        },
    )
    .await
    .unwrap();

    assert_eq!(reply.verdict, GoalVerdict::Met);
    assert_eq!(constructors.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(review_record(&evidence).verdict, EvidenceVerdict::Passed);
    assert_eq!(
        review_record(&evidence).spec_id,
        goal_review::GOAL_REVIEW_SPEC_ID
    );
}

#[tokio::test]
async fn blocking_review_finding_stops_before_the_judge() {
    let result = maybe_run_goal_review::<FakeReviewer, _, _>(
        &settings(2),
        GoalReviewRequest::default(),
        0,
        || async { Ok(reviewer(blocking_report())) },
    )
    .await;
    let mut evidence = GoalEvidence::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let judge_calls = calls.clone();
    let reply = review_then_judge(
        &mut evidence,
        &SubGoalRollup::default(),
        result,
        move || {
            judge_calls.fetch_add(1, Ordering::SeqCst);
            async {
                Ok(GoalVerifierReply {
                    verdict: GoalVerdict::Met,
                    verifier_reply: "GOAL: MET".to_string(),
                    tokens: 1,
                    criteria_verdicts: Vec::new(),
                })
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(review_record(&evidence).verdict, EvidenceVerdict::Failed);
    assert!(gaps(&reply)
        .iter()
        .any(|gap| gap.contains("review finding blocks completion")));
}

#[tokio::test]
async fn review_and_sub_goal_blocks_both_survive_gate_attachment() {
    let result = maybe_run_goal_review::<FakeReviewer, _, _>(
        &settings(2),
        GoalReviewRequest::default(),
        0,
        || async { Ok(reviewer(blocking_report())) },
    )
    .await;
    let mut evidence = GoalEvidence::default();
    let reply = apply_goal_review_result(&mut evidence, &blocked_lane_rollup(), result)
        .expect("a blocking review returns without invoking a judge");
    let reply_gaps = gaps(&reply);

    assert!(reply_gaps
        .iter()
        .any(|gap| gap.contains("review finding blocks completion")));
    assert!(reply_gaps
        .iter()
        .any(|gap| gap.contains("required child lane failed")));
    assert!(evidence.gates.blocks.iter().any(|block| {
        block.spec_id == "subgoal:lane-1" && block.verdict == EvidenceVerdict::Failed
    }));
}

#[tokio::test]
async fn repair_count_survives_restore_and_escalation_stops_autonomy() {
    let mut session = ChatSession::new("goal-review-reload".to_string());
    session.install_goal("agent", "ship the change", true, GoalBudget::default());

    for attempt_index in 0..2 {
        let attempts_used = goal_review_attempts_used(session.goal.as_ref().unwrap());
        assert_eq!(attempts_used, attempt_index);
        session.goal_set_status(GoalStatus::Verifying);
        let mut evidence = GoalEvidence::default();
        let reply = review_reply(fake_finding_report(), attempts_used, 2, &mut evidence).await;
        assert!(matches!(reply.verdict, GoalVerdict::Unmet(_)));
        assert_eq!(review_record(&evidence).verdict, EvidenceVerdict::Failed);
        assert_ne!(
            apply_goal_verdict(&mut session, "finish", reply, None),
            GoalVerificationApplyOutcome::Finalized
        );

        if attempt_index == 0 {
            let ChatEvent::Snapshot {
                thread,
                messages,
                goal: Some(goal),
                ..
            } = session.snapshot()
            else {
                panic!("snapshot must persist the active goal")
            };
            let mut goal =
                serde_json::from_value::<GoalSnapshot>(serde_json::to_value(goal).unwrap())
                    .unwrap();
            let goal_ledger: Vec<refact_chat_api::GoalLedgerEntry> =
                serde_json::from_value(serde_json::to_value(&session.goal_ledger).unwrap())
                    .unwrap();
            refact_chat_api::reduce_goal_ledger(&goal_ledger)
                .expect("the persisted goal ledger is installed")
                .apply_to_snapshot(&mut goal);
            session = ChatSession::new_with_trajectory(
                "goal-review-reload".to_string(),
                messages,
                thread,
                session.created_at.clone(),
                None,
                Vec::new(),
                Some(goal),
            );
            session.set_goal_ledger(goal_ledger);
            assert_eq!(goal_review_attempts_used(session.goal.as_ref().unwrap()), 1);
        }
    }

    let attempts_used = goal_review_attempts_used(session.goal.as_ref().unwrap());
    assert_eq!(attempts_used, 2);
    session.goal_set_status(GoalStatus::Verifying);
    let mut evidence = GoalEvidence::default();
    let reply = review_reply(fake_finding_report(), attempts_used, 2, &mut evidence).await;
    let reply_gaps = gaps(&reply);
    assert_eq!(
        reply_gaps.first().map(String::as_str),
        Some("review escalated after 2 repair attempts")
    );
    assert!(reply_gaps
        .iter()
        .any(|gap| gap.contains("review finding blocks completion")));
    assert_eq!(
        apply_goal_verdict(&mut session, "finish", reply, None),
        GoalVerificationApplyOutcome::Stalled
    );
    assert_eq!(session.goal_status, Some(GoalStatus::NoProgress));
    assert_eq!(session.runtime.state, SessionState::Completed);
    assert!(session.command_queue.is_empty());
    assert_eq!(goal_review_attempts_used(session.goal.as_ref().unwrap()), 3);
}

#[tokio::test]
async fn reviewer_construction_failure_is_unavailable_evidence_and_unmet() {
    let mut evidence = GoalEvidence::default();
    let result = maybe_run_goal_review::<FakeReviewer, _, _>(
        &settings(2),
        GoalReviewRequest::default(),
        0,
        || async { Err("no reviewer configuration".to_string()) },
    )
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let judge_calls = calls.clone();
    let reply = review_then_judge(
        &mut evidence,
        &SubGoalRollup::default(),
        result,
        move || {
            judge_calls.fetch_add(1, Ordering::SeqCst);
            async {
                Ok(GoalVerifierReply {
                    verdict: GoalVerdict::Met,
                    verifier_reply: "GOAL: MET".to_string(),
                    tokens: 1,
                    criteria_verdicts: Vec::new(),
                })
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        review_record(&evidence).verdict,
        EvidenceVerdict::Unavailable
    );
    assert!(matches!(reply.verdict, GoalVerdict::Unmet(_)));
    assert!(reply.criteria_verdicts.iter().any(|verdict| {
        verdict.id == goal_review::GOAL_REVIEW_SPEC_ID
            && verdict
                .evidence
                .as_ref()
                .is_some_and(|evidence| evidence.verdict == EvidenceVerdict::Unavailable.label())
    }));
}
