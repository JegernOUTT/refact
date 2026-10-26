use crate::goal_role::{
    current_base_goal, effective_goal_delta_count, goal_delta_events, goal_delta_history,
    project_current_goal, GoalRoleSession,
};
use crate::internal_roles;
use crate::plan_role::{
    effective_plan_delta_count, plan_delta_events, project_current_plan, PlanRoleSession,
};
use crate::role_projection::{
    checkpoint_provenance, fold_role_messages, ProjectedRole, ProjectionLimits,
};
use crate::GoalBudget;
use refact_core::chat_types::ChatMessage;

const LIMITS: ProjectionLimits = ProjectionLimits {
    checkpoint_after_deltas: 24,
    max_chars: 32_768,
};

#[derive(Default)]
struct Session {
    messages: Vec<ChatMessage>,
}

impl PlanRoleSession for Session {
    fn plan_role_messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    fn add_plan_role_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
    }
}

impl GoalRoleSession for Session {
    fn goal_role_messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    fn add_goal_role_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
    }
}

fn messages(role: ProjectedRole, count: usize) -> Vec<ChatMessage> {
    let mut base = match role {
        ProjectedRole::Plan => internal_roles::plan("task_planner", 1, "Approved base", None),
        ProjectedRole::Goal => internal_roles::goal(
            "task_planner",
            1,
            "Approved base",
            None,
            true,
            GoalBudget::default(),
        ),
    };
    base.message_id = "approved-base".to_string();
    let mut messages = vec![base];
    for seq in 1..=count {
        let mut delta = match role {
            ProjectedRole::Plan => internal_roles::plan_delta(
                "tool.update_plan",
                serde_json::json!({"seq": seq}),
                format!("update {seq}"),
            ),
            ProjectedRole::Goal => internal_roles::goal_delta(
                "tool.update_goal",
                serde_json::json!({"seq": seq}),
                format!("update {seq}"),
            ),
        };
        delta.message_id = format!("delta-{seq}");
        messages.push(delta);
    }
    messages
}

fn role_checkpoint(messages: &[ChatMessage], role: ProjectedRole) -> &ChatMessage {
    let subkind = match role {
        ProjectedRole::Plan => "plan_delta",
        ProjectedRole::Goal => "goal_delta",
    };
    messages
        .iter()
        .find(|message| {
            checkpoint_provenance(message).is_some() && message.extra["event"]["subkind"] == subkind
        })
        .expect("checkpoint")
}

#[test]
fn afolding_hundred_deltas_preserves_base_and_records_checkpoint_provenance() {
    let original = messages(ProjectedRole::Plan, 100);
    let base = original[0].clone();
    let folded = fold_role_messages(ProjectedRole::Plan, &original, LIMITS);
    let checkpoint = role_checkpoint(&folded, ProjectedRole::Plan);
    let provenance = checkpoint_provenance(checkpoint).unwrap();

    assert_eq!(
        serde_json::to_value(&folded[0]).unwrap(),
        serde_json::to_value(base).unwrap()
    );
    assert_eq!(provenance.folded_deltas, 76);
    assert_eq!(provenance.revision, 76);
    assert_eq!(provenance.approved_message_id, "approved-base");
    assert_eq!(provenance.approved_version, 1);
    assert_eq!(
        checkpoint.extra["event"]["source"],
        "context_rebuild.checkpoint"
    );
    assert_eq!(checkpoint.extra["event"]["payload"]["seq"], 76);
    assert_eq!(
        checkpoint.extra["event"]["payload"]["first_folded_message_id"],
        "delta-1"
    );
    assert_eq!(
        checkpoint.extra["event"]["payload"]["last_folded_message_id"],
        "delta-76"
    );
    assert_eq!(checkpoint.message_id, "plan-checkpoint-approved-base-76");
    assert_eq!(
        folded
            .iter()
            .filter(|m| checkpoint_provenance(m).is_some())
            .count(),
        1
    );
    assert_eq!(
        plan_delta_events(&Session {
            messages: folded.clone()
        })
        .len(),
        25
    );
}

#[test]
fn aprojection_is_equal_before_and_after_folding_for_plan_and_goal() {
    let mut plan = Session {
        messages: messages(ProjectedRole::Plan, 100),
    };
    let unfolded_plan = project_current_plan(&plan, LIMITS).unwrap();
    plan.messages = fold_role_messages(ProjectedRole::Plan, &plan.messages, LIMITS);
    assert_eq!(project_current_plan(&plan, LIMITS).unwrap(), unfolded_plan);

    let mut goal = Session {
        messages: messages(ProjectedRole::Goal, 100),
    };
    let unfolded_goal = project_current_goal(&goal, LIMITS).unwrap();
    goal.messages = fold_role_messages(ProjectedRole::Goal, &goal.messages, LIMITS);
    assert_eq!(project_current_goal(&goal, LIMITS).unwrap(), unfolded_goal);
}

#[test]
fn arefolding_merges_checkpoint_counts_and_retains_the_newest_updates() {
    let mut folded = fold_role_messages(
        ProjectedRole::Plan,
        &messages(ProjectedRole::Plan, 100),
        LIMITS,
    );
    folded.extend(
        messages(ProjectedRole::Plan, 150)
            .into_iter()
            .skip(1)
            .skip(100),
    );
    let refolded = fold_role_messages(ProjectedRole::Plan, &folded, LIMITS);
    let checkpoints: Vec<_> = refolded.iter().filter_map(checkpoint_provenance).collect();

    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].folded_deltas, 126);
    assert!(refolded
        .last()
        .unwrap()
        .content
        .content_text_only()
        .contains("update 150"));
    let session = Session {
        messages: refolded.clone(),
    };
    let projection = project_current_plan(&session, LIMITS).unwrap();
    assert!(projection.text.chars().count() <= LIMITS.max_chars);
    assert_eq!(effective_plan_delta_count(&session), 150);
    assert_eq!(
        projection.checkpoint.unwrap().revision,
        checkpoint_provenance(role_checkpoint(&refolded, ProjectedRole::Plan))
            .unwrap()
            .revision
    );
}

#[test]
fn afewer_than_threshold_deltas_are_unchanged() {
    let original = messages(ProjectedRole::Plan, 10);
    assert_eq!(
        serde_json::to_value(fold_role_messages(ProjectedRole::Plan, &original, LIMITS)).unwrap(),
        serde_json::to_value(original).unwrap()
    );
}

#[test]
fn goal_checkpoint_history_and_effective_count_exclude_checkpoint_event() {
    let mut session = Session {
        messages: messages(ProjectedRole::Goal, 100),
    };
    session.messages = fold_role_messages(ProjectedRole::Goal, &session.messages, LIMITS);
    assert_eq!(
        current_base_goal(&session).unwrap().message_id,
        "approved-base"
    );
    assert_eq!(goal_delta_history(&session).len(), 24);
    assert_eq!(effective_goal_delta_count(&session), 100);
    assert_eq!(goal_delta_events(&session).len(), 25);
}
