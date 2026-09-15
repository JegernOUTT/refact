use chrono::{TimeDelta, Utc};
use refact_chat_api::{
    CriterionVerdict, DeliverableStatus, SubGoalOutcome, SubGoalOwner, SubGoalReport, VerdictSource,
};
use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};

use crate::agents::types::{BackgroundAgent, BgAgentStatus};
use crate::app_state::AppState;
use crate::chat::internal_roles::{event, EventSubkind};

use crate::postprocessing::pp_command_output::OutputFilter;

const DEFERRED_RETRY_AFTER: TimeDelta = TimeDelta::seconds(10);
const MAX_EVENT_EDITED_FILES: usize = 50;
const MAX_EVENT_DIFF_SUMMARY_CHARS: usize = 2000;
const MAX_EVENT_CONFLICT_SUMMARY_CHARS: usize = 1000;

fn cap_text_chars(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}… [truncated, {total} chars]")
}

fn cap_file_list(files: &[String], max: usize) -> Vec<String> {
    if files.len() <= max {
        return files.to_vec();
    }
    let mut capped: Vec<String> = files.iter().take(max).cloned().collect();
    capped.push(format!("… and {} more", files.len() - max));
    capped
}

fn deliverable_note(status: &DeliverableStatus) -> String {
    if status.met {
        if status.met_after_correction() {
            return format!(
                "{} check passed after {} correction attempt(s)",
                status.check, status.corrections_used
            );
        }
        return format!("{} check passed", status.check);
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(omission) = status.omission.as_ref() {
        parts.push(format!(
            "omitted for context headroom ({} of {} tokens available); full output at {}",
            omission.available_tokens, omission.required_tokens, omission.reference
        ));
    }
    if !status.missing_sections.is_empty() {
        parts.push(format!(
            "missing sections: {}",
            status.missing_sections.join(", ")
        ));
    }
    if let Some(error) = status.schema_error.as_ref() {
        parts.push(format!("schema: {error}"));
    }
    if status.evidence_missing {
        parts.push("no evidence reported".to_string());
    }
    if status.corrections_exhausted {
        parts.push(format!(
            "still unmet after {} correction attempt(s)",
            status.corrections_used
        ));
    }
    if parts.is_empty() {
        format!("{} check failed", status.check)
    } else {
        parts.join("; ")
    }
}

/// The structured outcome a finished child owes its parent: its terminal status
/// plus the structural deliverable check it was held to. `result_summary` rides
/// along as a human-readable companion and is never read by aggregation.
pub(crate) fn sub_goal_report_from_agent(record: &BackgroundAgent) -> SubGoalReport {
    let claimed_outcome = match record.status {
        BgAgentStatus::Completed => SubGoalOutcome::Met,
        BgAgentStatus::Failed => SubGoalOutcome::Failed,
        BgAgentStatus::Cancelled | BgAgentStatus::Interrupted | BgAgentStatus::Closed => {
            SubGoalOutcome::Blocked
        }
        _ => SubGoalOutcome::Dispatched,
    };
    let criteria_verdicts = record
        .deliverable_status
        .as_ref()
        .map(|status| {
            vec![CriterionVerdict {
                id: format!("deliverable:{}", status.check),
                met: status.met,
                note: deliverable_note(status),
                source: VerdictSource::Measured,
                evidence: None,
            }]
        })
        .unwrap_or_default();
    SubGoalReport {
        claimed_outcome,
        criteria_verdicts,
        result_summary: record.result_summary.clone().unwrap_or_default(),
        reported_at_ms: record
            .finished_at
            .unwrap_or(record.created_at)
            .timestamp_millis()
            .max(0) as u64,
    }
}

async fn parent_sub_goal_lane(app: &AppState, record: &BackgroundAgent) -> Option<String> {
    let session_arc = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(&record.parent_chat_id).cloned()
    }?;
    let owner = SubGoalOwner::background_agent(&record.agent_id);
    let session = session_arc.lock().await;
    session
        .sub_goal_graph()
        .nodes
        .iter()
        .find(|state| state.node.owner.as_ref() == Some(&owner))
        .map(|state| state.node.id.clone())
}

/// Files a finished child's structured verdicts on the parent lane it owns. A
/// child closed by its parent supersedes the lane instead of reporting on it.
pub(crate) async fn record_sub_goal_outcome(app: &AppState, record: &BackgroundAgent) {
    let Some(node_id) = parent_sub_goal_lane(app, record).await else {
        return;
    };
    let session_arc = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(&record.parent_chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return;
    };
    let mut session = session_arc.lock().await;
    if record.status == BgAgentStatus::Closed {
        session.supersede_sub_goal(&node_id, "closed by parent");
        return;
    }
    session.record_sub_goal_report(&node_id, sub_goal_report_from_agent(record));
}

/// Opens a lane for a child launched under a parent that is pursuing a goal, so
/// the child's eventual verdicts have somewhere to land. The lane is optional:
/// composing a required lane is the parent's explicit choice, and an automatic
/// one must never start gating a goal that never asked for it.
async fn ensure_sub_goal_lane(app: &AppState, record: &BackgroundAgent) -> Option<String> {
    if let Some(node_id) = parent_sub_goal_lane(app, record).await {
        return Some(node_id);
    }
    let session_arc = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(&record.parent_chat_id).cloned()
    }?;
    let mut session = session_arc.lock().await;
    session.goal.as_ref()?;
    let node_id = format!("agent:{}", record.agent_id);
    let declaration = refact_chat_api::SubGoalDeclaration {
        nodes: vec![
            refact_chat_api::SubGoalNode::new(node_id.clone(), record.title.clone())
                .with_owner(SubGoalOwner::background_agent(&record.agent_id)),
        ],
        edges: vec![refact_chat_api::SubGoalEdge::optional(
            refact_chat_api::ROOT_SUB_GOAL_ID,
            node_id.clone(),
        )],
    };
    session.declare_sub_goals(declaration).ok()?;
    Some(node_id)
}

/// Marks the lane this agent owns as dispatched, so a lane in flight never
/// reads as `not_run`.
pub(crate) async fn mark_sub_goal_dispatched(app: &AppState, record: &BackgroundAgent) {
    let Some(node_id) = ensure_sub_goal_lane(app, record).await else {
        return;
    };
    let session_arc = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(&record.parent_chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return;
    };
    session_arc.lock().await.dispatch_sub_goal(
        &node_id,
        Some(SubGoalOwner::background_agent(&record.agent_id)),
    );
}

pub async fn push_completion_to_parent(
    app: AppState,
    record: &BackgroundAgent,
) -> Result<(), String> {
    if already_pushed(record) {
        return Ok(());
    }
    record_sub_goal_outcome(&app, record).await;

    let delivery = build_completion_delivery(record);
    let id = delivery.id.clone();
    match crate::chat::delivery::deliver_to_chat(app.clone(), &record.parent_chat_id, delivery)
        .await
    {
        Ok(DeliveryOutcome::Queued | DeliveryOutcome::Delivered | DeliveryOutcome::Duplicate) => {
            app.agents
                .set_completion_message_id_for_run(
                    &record.agent_id,
                    Some(record.run_generation),
                    id,
                )
                .await?;
        }
        Ok(DeliveryOutcome::Expired | DeliveryOutcome::Superseded) => {
            app.agents
                .set_completion_message_id_for_run(
                    &record.agent_id,
                    Some(record.run_generation),
                    "deferred".to_string(),
                )
                .await?;
            return Err("completion delivery was dropped before reaching the parent".to_string());
        }
        Err(error) => {
            app.agents
                .set_completion_message_id_for_run(
                    &record.agent_id,
                    Some(record.run_generation),
                    "deferred".to_string(),
                )
                .await?;
            return Err(error);
        }
    }
    crate::chat::goal_wait::settle_goal_wait(
        app,
        &record.parent_chat_id,
        &refact_chat_api::GoalWaitSubject::Child {
            agent_id: record.agent_id.clone(),
        },
        refact_chat_api::GoalWaitResume::Settled,
    )
    .await;
    Ok(())
}

fn build_completion_delivery(record: &BackgroundAgent) -> PendingDelivery {
    PendingDelivery::with_id(
        format!(
            "background-agent-finished-{}-{}",
            record.agent_id,
            record
                .finished_at
                .unwrap_or(record.created_at)
                .timestamp_micros()
        ),
        vec![build_completion_event(record)],
        record.completion_push,
        "agents.completion",
        true,
    )
}

pub async fn push_notice_to_chat(app: AppState, chat_id: &str, text: String) -> Result<(), String> {
    push_notice_to_chat_with_mode(app, chat_id, text, PushMode::Append).await
}

pub async fn push_notice_to_chat_with_mode(
    app: AppState,
    chat_id: &str,
    text: String,
    push: PushMode,
) -> Result<(), String> {
    crate::chat::delivery::deliver_to_chat(
        app,
        chat_id,
        PendingDelivery::new(
            vec![event(
                EventSubkind::SystemNotice,
                "agents.notice",
                serde_json::json!({}),
                text,
            )],
            push,
            "agents.notice".to_string(),
            true,
        ),
    )
    .await?;
    Ok(())
}

pub async fn flush_pending_pushes_for_parent(
    app: AppState,
    parent_chat_id: &str,
) -> Result<usize, String> {
    let records = app
        .agents
        .list_for_parent(
            parent_chat_id,
            crate::agents::types::AgentListFilter::default(),
        )
        .await;
    let mut pushed = 0usize;
    for record in records {
        if should_retry_completion_push(&record) {
            push_completion_to_parent(app.clone(), &record).await?;
            pushed += 1;
        }
    }
    Ok(pushed)
}

pub(crate) fn should_retry_completion_push(record: &BackgroundAgent) -> bool {
    match record.completion_message_id.as_deref() {
        Some("pending") => true,
        Some("deferred") => record
            .deferred_at
            .map(|deferred_at| deferred_at <= Utc::now() - DEFERRED_RETRY_AFTER)
            .unwrap_or(false),
        _ => false,
    }
}

fn already_pushed(record: &BackgroundAgent) -> bool {
    record
        .completion_message_id
        .as_deref()
        .map(|id| id != "pending" && id != "deferred")
        .unwrap_or(false)
}

fn build_completion_event(record: &BackgroundAgent) -> refact_core::chat_types::ChatMessage {
    let edited_files = cap_file_list(&record.edited_files, MAX_EVENT_EDITED_FILES);
    let diff_summary = record
        .diff_summary
        .as_deref()
        .map(|text| cap_text_chars(text, MAX_EVENT_DIFF_SUMMARY_CHARS));
    let conflict_summary = record
        .conflict_summary
        .as_deref()
        .map(|text| cap_text_chars(text, MAX_EVENT_CONFLICT_SUMMARY_CHARS));
    let mut message = event(
        EventSubkind::SystemNotice,
        "agents.spawn",
        serde_json::json!({
            "agent_id": record.agent_id,
            "run_generation": record.run_generation,
            "parent_chat_id": record.parent_chat_id,
            "parent_root_chat_id": record.parent_root_chat_id,
            "parent_tool_call_id": record.parent_tool_call_id,
            "kind": record.kind.as_str(),
            "status": record.status.as_str(),
            "title": record.title,
            "config_name": record.config_name,
            "target_files": record.target_files,
            "model": record.model,
            "model_type": record.model_type,
            "goal_present": record.goal_summary.is_some(),
            "child_chat_id": record.child_chat_id,
            "edited_files": edited_files,
            "diff_summary": diff_summary,
            "conflict_summary": conflict_summary,
            "worktree_branch": record.worktree_branch,
            "merge_status": record.merge_status,
            "tokens_used": record.tokens_used,
            "cost_usd": record.cost_usd,
            "deliverable_status": record.deliverable_status,
            "profile_name": record.profile_name,
            "handoff": record.handoff,
            "created_at": record.created_at,
            "last_update_at": record.last_update_at,
        }),
        build_push_message(record),
    );
    message.preserve = Some(true);
    message.output_filter = Some(OutputFilter::no_limits());
    message
}

fn build_push_message(record: &BackgroundAgent) -> String {
    let mut lines = vec![
        "[background subagent finished]".to_string(),
        format!("agent_id: {}", record.agent_id),
        format!("status: {}", record.status.as_str()),
        format!("title: {}", record.title),
        format!(
            "child_chat_id: {}",
            record.child_chat_id.as_deref().unwrap_or("")
        ),
        String::new(),
    ];
    if !record.target_files.is_empty() {
        lines.push("Target files:".to_string());
        lines.extend(record.target_files.iter().map(|file| format!("- {}", file)));
        lines.push(String::new());
    }
    if let Some(status) = &record.deliverable_status {
        lines.push(format!(
            "deliverable_contract: {} ({} check)",
            if status.met { "met" } else { "UNMET" },
            status.check
        ));
        if !status.missing_sections.is_empty() {
            lines.push(format!(
                "- missing sections: {}",
                status.missing_sections.join(", ")
            ));
        }
        if let Some(error) = &status.schema_error {
            lines.push(format!("- output schema: {error}"));
        }
        if status.evidence_missing {
            lines.push("- evidence section missing or empty; claims are unverified".into());
        }
        lines.push(String::new());
    } else if record.deliverable.is_some() {
        lines.push("deliverable_contract: not evaluated (no completed final message)".into());
        lines.push(String::new());
    }
    match &record.handoff {
        Some(report) => {
            lines.extend(report.summary_lines());
            lines.push(
                "(the child's claim about the brief it was given; it changes nothing on its own)"
                    .into(),
            );
        }
        None if record.status == BgAgentStatus::Completed => {
            lines.push("handoff_sufficiency: unknown (the child declared nothing)".into());
        }
        None => {}
    }
    lines.push(String::new());
    if !record.edited_files.is_empty() {
        lines.push("Edited files:".to_string());
        lines.extend(
            cap_file_list(&record.edited_files, MAX_EVENT_EDITED_FILES)
                .iter()
                .map(|file| format!("- {}", file)),
        );
        lines.push(String::new());
    }

    if let Some(branch) = record.worktree_branch.as_deref() {
        lines.push(format!("worktree_branch: {}", branch));
    }
    if let Some(merge_status) = record.merge_status.as_deref() {
        lines.push(format!("merge_status: {}", merge_status));
    }
    lines.push(format!("tokens_used: {}", record.tokens_used));
    if let Some(cost_usd) = record.cost_usd {
        lines.push(format!("cost_usd: {:.6}", cost_usd));
    }
    if record.goal_summary.is_some() {
        lines.push("goal: present".to_string());
    }
    lines.push(String::new());

    if let Some(child_chat_id) = &record.child_chat_id {
        lines.push(format!(
            "Open the child trajectory: [view](refact://chat/{child_chat_id})"
        ));
        lines.push(String::new());
    }

    match record.status {
        BgAgentStatus::Completed => {
            lines.push("Summary:".to_string());
            lines.push(record.result_summary.as_deref().unwrap_or("").to_string());
        }
        BgAgentStatus::Failed => {
            lines.push("Error:".to_string());
            lines.push(record.error.as_deref().unwrap_or("").to_string());
            if record
                .error
                .as_deref()
                .and_then(crate::agents::types::CapStop::from_error)
                .is_some_and(|stop| stop.parent_can_raise_steps())
            {
                lines.push("Use agent_resume with action inspect to check this child's limits, then raise_and_resume with the returned change_seq. This preserves the same child task and trajectory.".into());
            }
        }
        BgAgentStatus::Cancelled => {
            lines.push("Reason:".to_string());
            lines.push(record.error.as_deref().unwrap_or("cancelled").to_string());
        }
        BgAgentStatus::Interrupted => {
            lines.push(
                "Note: engine restarted before this agent finished. State is preserved; the change cannot be auto-resumed."
                    .to_string(),
            );
        }
        BgAgentStatus::Closed => {
            lines.push("Reason:".to_string());
            lines.push(
                record
                    .close_reason
                    .as_deref()
                    .unwrap_or("closed by the parent")
                    .to_string(),
            );
            lines.push(
                "This child is closed, not finished. Its history, goal and plan are preserved; use agent_resume with action inspect, then resume with the returned change_seq to continue the same child."
                    .to_string(),
            );
        }
        BgAgentStatus::Queued | BgAgentStatus::Running | BgAgentStatus::WaitingForApproval => {
            lines.push("Summary:".to_string());
            lines.push(record.progress.as_deref().unwrap_or("").to_string());
        }
    }

    if let Some(conflict_summary) = record
        .conflict_summary
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        lines.push(String::new());
        lines.push("Conflicts:".to_string());
        lines.push(cap_text_chars(
            conflict_summary,
            MAX_EVENT_CONFLICT_SUMMARY_CHARS,
        ));
    }

    lines.push(String::new());
    lines.push(format!(
        "(call agent_result(agent_id=\"{}\", include_details=true) for stored payload/details, or open the child trajectory)",
        record.agent_id
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::types::{BgAgentKind, CreateAgentRequest};

    async fn completed_record(kind: BgAgentKind) -> BackgroundAgent {
        let temp = tempfile::tempdir().unwrap();
        let registry =
            crate::agents::registry::BackgroundAgentRegistry::new(temp.path().to_path_buf())
                .await
                .unwrap();
        let (record, _, _) = registry
            .create(CreateAgentRequest {
                parent_chat_id: "parent".to_string(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind,
                config_name: kind.as_str().to_string(),
                title: "Fix retry parsing".to_string(),
                prompt: "prompt".to_string(),
                target_files: vec!["src/auth/retry.ts".to_string()],
                model: "model".to_string(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        registry
            .mark_completed(
                &record.agent_id,
                crate::agents::types::AgentCompletion {
                    result_summary: "done".to_string(),
                    edited_files: vec!["src/auth/retry.ts".to_string()],
                    diff_summary: None,
                    conflict_summary: None,
                    child_chat_id: Some("child".to_string()),
                    deliverable_status: None,
                    handoff: None,
                },
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn completion_delivery_defaults_to_append_and_keeps_stable_identity() {
        let mut record = completed_record(BgAgentKind::Subagent).await;
        let first = build_completion_delivery(&record);
        assert_eq!(first.push, PushMode::Append);
        assert!(first.wake);
        record.completion_push = PushMode::WhenIdle;
        let retry = build_completion_delivery(&record);
        assert_eq!(retry.id, first.id);
        assert_eq!(retry.push, PushMode::WhenIdle);
        record.completion_push = PushMode::Preempt;
        assert_eq!(build_completion_delivery(&record).push, PushMode::Preempt);
        record.finished_at = record
            .finished_at
            .map(|time| time + TimeDelta::milliseconds(1));
        assert_ne!(build_completion_delivery(&record).id, first.id);
    }

    #[tokio::test]
    async fn legacy_delegate_push_message_uses_unified_noun_and_includes_edited_files() {
        let record = completed_record(BgAgentKind::Delegate).await;
        let message = build_push_message(&record);
        assert!(message.contains("[background subagent finished]"));
        assert!(message.contains("Edited files:"));
        assert!(message.contains("src/auth/retry.ts"));
        assert!(message.contains("Open the child trajectory: [view](refact://chat/child)"));
    }

    #[tokio::test]
    async fn subagent_push_message_includes_edited_files() {
        let record = completed_record(BgAgentKind::Subagent).await;
        let message = build_push_message(&record);
        assert!(message.contains("[background subagent finished]"));
        assert!(message.contains("Edited files:"));
        assert!(message.contains("Open the child trajectory: [view](refact://chat/child)"));
    }

    #[tokio::test]
    async fn push_message_preserves_long_summary() {
        let temp = tempfile::tempdir().unwrap();
        let registry =
            crate::agents::registry::BackgroundAgentRegistry::new(temp.path().to_path_buf())
                .await
                .unwrap();
        let (record, _, _) = registry
            .create(CreateAgentRequest {
                parent_chat_id: "parent".to_string(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "subagent".to_string(),
                title: "Long report".to_string(),
                prompt: "prompt".to_string(),
                target_files: Vec::new(),
                model: "model".to_string(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        let summary = "full-report-line\n".repeat(200);
        let record = registry
            .mark_completed(
                &record.agent_id,
                crate::agents::types::AgentCompletion {
                    result_summary: summary.clone(),
                    edited_files: Vec::new(),
                    diff_summary: None,
                    conflict_summary: None,
                    child_chat_id: Some("child".to_string()),
                    deliverable_status: None,
                    handoff: None,
                },
            )
            .await
            .unwrap();

        let message = build_push_message(&record);

        assert!(message.contains(&summary));
        assert!(!message.contains('…'));

        let event = build_completion_event(&record);
        assert!(event.content.content_text_only().contains(&summary));
        assert_eq!(event.preserve, Some(true));
        assert_eq!(
            event
                .output_filter
                .as_ref()
                .map(|filter| filter.limit_chars),
            Some(usize::MAX)
        );
    }
}
