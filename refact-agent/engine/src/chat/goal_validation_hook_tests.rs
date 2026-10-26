use std::sync::Arc;

use crate::app_state::AppState;
use crate::chat::goal_verifier::{run_goal_verification, GoalVerdict, GoalVerifierReply};
use crate::chat::types::{
    ChatSession, CriterionEvidence, CriterionVerdict, GoalBudget, GoalCriterion, GoalWait,
    GoalWaitSubject,
};
use crate::ext::hooks_runner::{clear_hook_cache_for_tests, HooksConfig};
use tokio::sync::Mutex as AMutex;

fn reply(verdict: GoalVerdict) -> GoalVerifierReply {
    GoalVerifierReply {
        verdict,
        verifier_reply: String::new(),
        tokens: 17,
        criteria_verdicts: Vec::new(),
    }
}

#[test]
fn goal_validation_extra_encodes_verdicts_and_truncates_values() {
    let met = super::goal_validation_hook::goal_validation_extra(&Ok(reply(GoalVerdict::Met)));
    let mut keys = met.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "criteria",
            "error",
            "gaps",
            "judge_tokens",
            "outcome",
            "wait"
        ]
    );
    assert_eq!(met["outcome"], "met");
    assert_eq!(met["gaps"], serde_json::json!([]));
    assert_eq!(met["wait"], serde_json::Value::Null);
    assert_eq!(met["criteria"], serde_json::json!([]));
    assert_eq!(met["judge_tokens"], 17);
    assert_eq!(met["error"], serde_json::Value::Null);

    let mut unmet_reply = reply(GoalVerdict::Unmet(vec![
        "first gap".into(),
        "second gap".into(),
    ]));
    unmet_reply.criteria_verdicts = vec![
        CriterionVerdict {
            id: "C1".into(),
            met: false,
            evidence: Some(CriterionEvidence {
                spec_id: "criterion:C1".into(),
                command: "false".into(),
                exit_code: Some(1),
                verdict: "failed".into(),
            }),
            ..Default::default()
        },
        CriterionVerdict {
            id: "C2".into(),
            met: true,
            ..Default::default()
        },
    ];
    let unmet = super::goal_validation_hook::goal_validation_extra(&Ok(unmet_reply));
    assert_eq!(unmet["outcome"], "unmet");
    assert_eq!(
        unmet["gaps"],
        serde_json::json!(["first gap", "second gap"])
    );
    assert_eq!(
        unmet["criteria"],
        serde_json::json!([
            {"id":"C1", "met":false, "evidence_verdict":"failed"},
            {"id":"C2", "met":true, "evidence_verdict":null}
        ])
    );
    assert_eq!(unmet["judge_tokens"], 17);
    assert_eq!(unmet["wait"], serde_json::Value::Null);
    assert_eq!(unmet["error"], serde_json::Value::Null);

    let wait_reply = reply(GoalVerdict::Wait(GoalWait::new(
        GoalWaitSubject::Event {
            name: "deployment".into(),
        },
        Some(1234),
    )));
    let expected_wait = match &wait_reply.verdict {
        GoalVerdict::Wait(wait) => serde_json::to_value(wait).unwrap(),
        _ => unreachable!(),
    };
    let wait = super::goal_validation_hook::goal_validation_extra(&Ok(wait_reply));
    assert_eq!(wait["outcome"], "wait");
    assert_eq!(wait["wait"], expected_wait);
    assert_eq!(wait["gaps"], serde_json::json!([]));
    assert_eq!(wait["criteria"], serde_json::json!([]));
    assert_eq!(wait["error"], serde_json::Value::Null);

    let error =
        super::goal_validation_hook::goal_validation_extra(&Err("verification failed".into()));
    assert_eq!(error["outcome"], "error");
    assert_eq!(error["gaps"], serde_json::json!([]));
    assert_eq!(error["wait"], serde_json::Value::Null);
    assert_eq!(error["criteria"], serde_json::json!([]));
    assert_eq!(error["judge_tokens"], 0);
    assert_eq!(error["error"], "verification failed");

    let mut oversized = reply(GoalVerdict::Unmet(vec!["🦀".repeat(501)]));
    oversized.criteria_verdicts = (0..51)
        .map(|index| CriterionVerdict {
            id: index.to_string(),
            ..Default::default()
        })
        .collect();
    let bounded = super::goal_validation_hook::goal_validation_extra(&Ok(oversized));
    assert_eq!(bounded["gaps"][0].as_str().unwrap().chars().count(), 500);
    assert_eq!(bounded["criteria"].as_array().unwrap().len(), 50);

    let long_error = super::goal_validation_hook::goal_validation_extra(&Err("é".repeat(2001)));
    assert_eq!(long_error["error"].as_str().unwrap().chars().count(), 2000);
}

async fn test_context(enabled: bool) -> (tempfile::TempDir, AppState, Arc<AMutex<ChatSession>>) {
    let dir = tempfile::tempdir().unwrap();
    let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    Arc::get_mut(&mut gcx).unwrap().hooks_config = HooksConfig {
        enabled,
        ..HooksConfig::default()
    };
    let app = AppState::from_gcx(gcx).await;
    let session = Arc::new(AMutex::new(ChatSession::new("goal-validation-test".into())));
    (dir, app, session)
}

async fn write_hook(app: &AppState, marker: &std::path::Path) {
    let command = format!("cat >> {}; echo >> {}", marker.display(), marker.display());
    tokio::fs::write(
        app.paths.config_dir.join("hooks.yaml"),
        format!(
            "hooks:\n  GoalValidation:\n    - hooks:\n        - type: command\n          command: {command:?}\n"
        ),
    )
    .await
    .unwrap();
}

fn test_criterion() -> GoalCriterion {
    GoalCriterion {
        id: "C1".into(),
        text: "verification succeeds".into(),
        verify_hint: None,
        verify_command: Some("cargo test --lib".into()),
    }
}

async fn install_goal(session: &Arc<AMutex<ChatSession>>) {
    session.lock().await.install_goal_with_criteria(
        "agent",
        "complete the task",
        true,
        GoalBudget::default(),
        vec![test_criterion()],
    );
}

async fn payload_lines(marker: &std::path::Path) -> Vec<serde_json::Value> {
    tokio::fs::read_to_string(marker)
        .await
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn gates_blocked_verification_runs_the_hook_once_without_the_judge() {
    let (dir, app, session) = test_context(true).await;
    let marker = dir.path().join("goal-validation-payloads");
    write_hook(&app, &marker).await;
    clear_hook_cache_for_tests();
    install_goal(&session).await;
    let chat_id = session.lock().await.chat_id.clone();

    let first = run_goal_verification(app.clone(), session.clone())
        .await
        .unwrap();
    assert!(matches!(first.verdict, GoalVerdict::Unmet(_)));
    assert_eq!(first.tokens, 0);
    let first_lines = payload_lines(&marker).await;
    assert_eq!(first_lines.len(), 1);
    assert_eq!(first_lines[0]["outcome"], "unmet");
    assert_eq!(first_lines[0]["session_id"], chat_id);
    assert_eq!(first_lines[0]["hook_event_name"], "GoalValidation");
    assert_eq!(
        first_lines[0]["criteria"],
        serde_json::json!([{
            "id": "C1",
            "met": false,
            "evidence_verdict": "unavailable"
        }])
    );

    let second = run_goal_verification(app, session).await.unwrap();
    assert!(matches!(second.verdict, GoalVerdict::Unmet(_)));
    assert_eq!(second.tokens, 0);
    assert_eq!(payload_lines(&marker).await.len(), 2);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn disabled_hooks_config_runs_nothing() {
    let (dir, app, session) = test_context(false).await;
    let marker = dir.path().join("goal-validation-payloads");
    write_hook(&app, &marker).await;
    clear_hook_cache_for_tests();
    install_goal(&session).await;

    let reply = run_goal_verification(app, session).await.unwrap();
    assert!(matches!(reply.verdict, GoalVerdict::Unmet(_)));
    assert_eq!(reply.tokens, 0);
    assert!(!marker.exists());
}
