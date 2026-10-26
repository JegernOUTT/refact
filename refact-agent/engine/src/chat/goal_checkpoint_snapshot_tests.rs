use crate::chat::internal_roles;
use crate::chat::types::{ChatSession, GoalBudget};
use refact_chat_api::role_projection::{fold_role_messages, ProjectedRole, ProjectionLimits};
use refact_core::active_context::{make_reconstruction_report, ReconstructionMetadata};
use serde_json::json;

const BASE_GOAL: &str = " \n# Goal — preserve this exact base\n\n  trailing whitespace \n";
const DELTA_COUNT: usize = 60;

fn rebuilt_goal_session() -> ChatSession {
    let mut goal = internal_roles::goal("agent", 1, BASE_GOAL, None, true, GoalBudget::default());
    goal.message_id = "goal-base".into();
    let mut messages = vec![goal];
    messages.extend((1..=DELTA_COUNT).map(|seq| {
        let mut message = internal_roles::goal_delta(
            "tool.update_goal",
            json!({"seq": seq}),
            format!("goal update {seq}"),
        );
        message.message_id = format!("goal-delta-{seq}");
        message
    }));

    let folded = fold_role_messages(
        ProjectedRole::Goal,
        &messages,
        ProjectionLimits {
            checkpoint_after_deltas: 24,
            max_chars: 32_768,
        },
    );
    let report = make_reconstruction_report(folded, ReconstructionMetadata::default()).unwrap();
    let mut session = ChatSession::new("goal-checkpoint-snapshot".into());
    session.messages = messages;
    session.rebuild_goal_projection_from_messages();
    session.messages = vec![report];
    session.rebuild_goal_projection_from_messages();
    session
}

#[test]
fn goal_events_keep_folded_deltas() {
    let session = rebuilt_goal_session();
    let events: Vec<_> = session
        .goal
        .as_ref()
        .unwrap()
        .events
        .iter()
        .filter(|event| event.kind == "goal_delta")
        .collect();
    assert_eq!(events.len(), DELTA_COUNT);
    for seq in 1..=DELTA_COUNT {
        assert!(events
            .iter()
            .any(|event| event.text == format!("goal update {seq}")));
    }
}
