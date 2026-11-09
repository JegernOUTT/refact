use super::turn_completion::*;
use std::sync::Arc;
use refact_review_core::ProposalTurnCompletion;
use crate::{app_state::AppState, call_validation::ChatMessage, chat::types::ChatSession};
use crate::call_validation::{ChatToolCall, ChatToolFunction};

fn record() -> ProposalTurnCompletion {
    ProposalTurnCompletion {
        owner_chat_id: "owner".into(),
        turn_id: "turn".into(),
        tool_call_id: "call".into(),
        job_id: None,
        human_message_id: Some("human".into()),
        review_id: "review".into(),
        candidate_id: "candidate".into(),
        proposal_id: "proposal-boundary-test".into(),
        contribution_id: "contribution".into(),
        delivery_id: "delivery".into(),
        result_message_ids: vec!["result".into()],
    }
}

#[tokio::test]
async fn proposal_gate_rejects_missing_live_origin_delivery_result_siblings_and_human() {
    let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
    let mut session = ChatSession::new("owner".into());
    let mut human = ChatMessage::new("user".into(), "explain".into());
    human.message_id = "human".into();
    session.add_message(human);
    let mut turn = ChatMessage::new("assistant".into(), String::new());
    turn.message_id = "turn".into();
    turn.tool_calls = Some(vec![ChatToolCall {
        id: "call".into(),
        index: None,
        function: ChatToolFunction {
            name: "review_update".into(),
            arguments: "{}".into(),
        },
        tool_type: "function".into(),
        extra_content: None,
        started_at_ms: None,
        completed_at_ms: None,
    }]);
    session.add_message(turn);
    let mut result = ChatMessage::new("tool".into(), "success".into());
    result.message_id = "result".into();
    result.tool_call_id = "call".into();
    result.tool_failed = Some(false);
    session.add_message(result);
    let record = record();
    assert!(!eligible(&app, &session, &record));
    origins().lock().unwrap().insert(
        record.proposal_id.clone(),
        (
            std::sync::Weak::new(),
            Arc::downgrade(&session.execution_abort),
        ),
    );
    assert!(!eligible(&app, &session, &record));
    session.delivered_delivery_ids.insert("delivery".into());
    assert!(eligible(&app, &session, &record));
    session.messages.last_mut().unwrap().tool_failed = Some(true);
    assert!(!eligible(&app, &session, &record));
    session.messages.last_mut().unwrap().tool_failed = Some(false);
    let sibling = session.messages[1].tool_calls.as_ref().unwrap()[0].clone();
    session.messages[1]
        .tool_calls
        .as_mut()
        .unwrap()
        .push(sibling);
    assert!(!eligible(&app, &session, &record));
    session.messages[1].tool_calls.as_mut().unwrap().pop();
    session.add_message(ChatMessage::new("user".into(), "new input".into()));
    assert!(!eligible(&app, &session, &record));
    session.messages.pop();
    session.begin_execution_origin();
    assert!(!eligible(&app, &session, &record));
    origins().lock().unwrap().remove(&record.proposal_id);
    app.runtime.tool_jobs.shutdown().await;
}
