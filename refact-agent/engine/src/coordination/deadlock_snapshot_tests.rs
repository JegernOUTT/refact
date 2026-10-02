use crate::coordination::deadlock_identity_tests::{
    app_with_workspace, create_running_record, parked_run, parked_ticket,
};
use refact_chat_api::{AskQuestion, QuestionKind};
use refact_work_core::{WaitKind, WaitTarget};

#[tokio::test]
#[serial_test::serial(test_runner, runtime_settings)]
async fn capture_reports_incomplete_when_the_registry_is_busy() {
    let (app, _temp) = app_with_workspace().await;
    let agent_id = create_running_record(&app, "parent", "child-chat", None).await;
    let _ticket = parked_ticket(&app, parked_run(&agent_id)).await;
    let waiting = app.wait_graph.declare(
        parked_run(&agent_id),
        WaitKind::AgentWait,
        vec![WaitTarget::Run(parked_run("target"))],
        "agent_wait",
    );
    let registry_guard = app.agents.records_write_guard_for_test().await;

    assert_eq!(
        crate::coordination::deadlock::scan(&app),
        refact_work_core::Detection::Incomplete("agent registry busy".to_string())
    );
    assert!(waiting.refusal().is_none());
    assert!(app.wait_graph.latest_report().is_none());
    drop(registry_guard);
}

#[tokio::test]
#[serial_test::serial(test_runner, runtime_settings)]
async fn question_edges_resolve_through_the_snapshot() {
    let (app, _temp) = app_with_workspace().await;
    let asker_id = create_running_record(&app, "parent", "asker-chat", Some("agent-asker")).await;
    let answerer_id =
        create_running_record(&app, "asker-chat", "answerer-chat", Some("agent-answerer")).await;
    let _asker_ticket = parked_ticket(&app, parked_run(&asker_id)).await;
    let _answerer_ticket = parked_ticket(&app, parked_run(&answerer_id)).await;
    app.questions()
        .ask(
            AskQuestion::new(
                QuestionKind::AgentParent,
                &asker_id,
                "answerer-chat",
                "question-1",
                "May I continue?",
            ),
            chrono::Utc::now(),
        )
        .unwrap();

    let graph = crate::coordination::deadlock::capture(&app);

    assert!(graph.edges.iter().any(|edge| {
        edge.kind == WaitKind::Question
            && edge.from == parked_run("agent-asker")
            && edge.to == WaitTarget::Run(parked_run("agent-answerer"))
    }));
}
