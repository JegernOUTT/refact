use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::evidence::EvidenceVerdict;
use crate::chat::goal_verifier::{
    apply_goal_verdict_guarded_with_delivery, begin_goal_verification_if_needed,
    run_goal_verification, GoalVerificationApplyOutcome, GoalVerificationBegin,
    GoalVerificationFork, GoalVerdict,
};
use crate::chat::types::{ChatSession, PushMode};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolValidateGoal {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolValidateGoal {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "validate_goal".to_string(),
            display_name: "Validate Goal".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Validate the chat's active goal against its success criteria. If met, the goal is marked complete and pursuit stops; otherwise returns the remaining gaps.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"push": PushMode::schema()},
                "required": [],
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let push = PushMode::from_args(args)?;
        let (app, chat_id) = {
            let ccx = ccx.lock().await;
            (ccx.app.clone(), ccx.chat_id.clone())
        };
        let gcx = app.gcx.clone();
        let session_arc = {
            let sessions = gcx.chat_sessions.read().await;
            sessions.get(&chat_id).cloned()
        };
        let session_arc = match session_arc {
            Some(session) => session,
            None if crate::chat::try_restore_session_if_trajectory_exists(
                app.clone(),
                &app.chat.sessions,
                &chat_id,
            )
            .await =>
            {
                let sessions = gcx.chat_sessions.read().await;
                sessions
                    .get(&chat_id)
                    .cloned()
                    .ok_or_else(|| format!("chat session `{chat_id}` not found"))?
            }
            None => return Err(format!("chat session `{chat_id}` not found")),
        };

        let (begin, fork) = {
            let mut session = session_arc.lock().await;
            let begin = begin_validate_goal(&mut session);
            let fork = GoalVerificationFork::capture(&session);
            (begin, fork)
        };
        match begin {
            GoalVerificationBegin::Started => {}
            GoalVerificationBegin::NoActiveGoal => {
                return tool_message_with_extra(
                    tool_call_id,
                    "No active goal to validate.".to_string(),
                    verdict_extra(EvidenceVerdict::NotRun, None),
                );
            }
            GoalVerificationBegin::BudgetExhausted => {
                return tool_message_with_extra(
                    tool_call_id,
                    budget_exhausted_message(),
                    verdict_extra(EvidenceVerdict::Blocked, None),
                );
            }
        }

        let reply = match run_goal_verification(app.clone(), session_arc.clone()).await {
            Ok(reply) => reply,
            Err(error) => {
                reset_after_verifier_error_with_delivery(
                    session_arc.clone(),
                    &error,
                    push,
                    Some(tool_call_id),
                )
                .await;
                return tool_message_with_extra(
                    tool_call_id,
                    format!("Goal validation could not run: {error}"),
                    verdict_extra(EvidenceVerdict::Unavailable, None),
                );
            }
        };

        let verdict = reply.verdict.clone();
        let verifier_reply = reply.verifier_reply.clone();
        let outcome = {
            let mut session = session_arc.lock().await;
            apply_goal_verdict_guarded_with_delivery(
                &mut session,
                "validate_goal",
                reply,
                fork,
                push,
                Some(tool_call_id),
            )
        };
        if outcome == GoalVerificationApplyOutcome::Waiting {
            crate::chat::goal_wait::begin_goal_wait_park(app, session_arc).await;
        }
        let content = validation_content(outcome, &verdict, &verifier_reply);
        let extra = validation_extra(outcome, &verdict);
        tool_message_with_extra(tool_call_id, content, extra)
    }
}

fn begin_validate_goal(session: &mut ChatSession) -> GoalVerificationBegin {
    begin_goal_verification_if_needed(session)
}

fn budget_exhausted_message() -> String {
    "Goal budget exhausted — cannot validate; the goal has hit its configured budget. Adjust the budget (set_goal_budget) or stop the goal.".to_string()
}

#[cfg(test)]
async fn reset_after_verifier_error(session_arc: Arc<AMutex<ChatSession>>, error: &str) {
    reset_after_verifier_error_with_delivery(session_arc, error, PushMode::Append, None).await;
}

async fn reset_after_verifier_error_with_delivery(
    session_arc: Arc<AMutex<ChatSession>>,
    error: &str,
    push: PushMode,
    tool_call_id: Option<&str>,
) {
    let mut session = session_arc.lock().await;
    let _ = crate::chat::goal_verifier::handle_verifier_failure_with_delivery(
        &mut session,
        "validate_goal",
        error,
        push,
        tool_call_id,
    );
}

fn validation_content(
    outcome: GoalVerificationApplyOutcome,
    verdict: &GoalVerdict,
    verifier_reply: &str,
) -> String {
    match outcome {
        GoalVerificationApplyOutcome::Finalized => {
            format!("GOAL MET — goal marked complete; pursuit disabled.\n\n{verifier_reply}")
        }
        GoalVerificationApplyOutcome::Superseded => {
            "Goal state changed during validation; verdict discarded.".to_string()
        }
        GoalVerificationApplyOutcome::Waiting => match verdict {
            GoalVerdict::Wait(wait) => format!(
                "GOAL WAITING on {} — pursuit is parked until it settles; no goal rounds run meanwhile.",
                wait.subject.label()
            ),
            _ => "GOAL WAITING — pursuit is parked until the awaited subject settles.".to_string(),
        },
        GoalVerificationApplyOutcome::Rearmed
        | GoalVerificationApplyOutcome::Continued
        | GoalVerificationApplyOutcome::Stalled => {
            let gaps = match verdict {
                GoalVerdict::Unmet(gaps) => gaps.as_slice(),
                GoalVerdict::Met | GoalVerdict::Wait(_) => &[],
            };
            format!(
                "GOAL NOT YET MET — remaining gaps:\n{}\n\n{}",
                format_gaps(gaps),
                verifier_reply
            )
        }
        GoalVerificationApplyOutcome::NoGoal => "No active goal to validate.".to_string(),
    }
}

/// How this validation reads as evidence. Only a finalized `Met` is `Passed`;
/// everything else either proves a gap or proves no verdict was obtained. Every
/// surface reads this value, and none of them may re-derive it by matching words
/// in the human-readable content, which changes whenever the wording does.
fn validation_evidence_verdict(
    outcome: GoalVerificationApplyOutcome,
    verdict: &GoalVerdict,
) -> EvidenceVerdict {
    match (outcome, verdict) {
        (GoalVerificationApplyOutcome::Finalized, GoalVerdict::Met) => EvidenceVerdict::Passed,
        (GoalVerificationApplyOutcome::Superseded, _) => EvidenceVerdict::Superseded,
        (
            GoalVerificationApplyOutcome::Rearmed
            | GoalVerificationApplyOutcome::Continued
            | GoalVerificationApplyOutcome::Stalled,
            GoalVerdict::Unmet(_),
        ) => EvidenceVerdict::Failed,
        _ => EvidenceVerdict::NotRun,
    }
}

fn verdict_extra(verdict: EvidenceVerdict, marker: Option<&str>) -> Map<String, Value> {
    let mut row = Map::new();
    row.insert("evidence_verdict".to_string(), json!(verdict.label()));
    if let Some(marker) = marker {
        row.insert("verdict".to_string(), json!(marker));
    }
    Map::from_iter([("validate_goal".to_string(), Value::Object(row))])
}

fn validation_extra(
    outcome: GoalVerificationApplyOutcome,
    verdict: &GoalVerdict,
) -> Map<String, Value> {
    let marker = match (outcome, verdict) {
        (GoalVerificationApplyOutcome::Finalized, GoalVerdict::Met) => Some("met"),
        (GoalVerificationApplyOutcome::Waiting, GoalVerdict::Wait(_)) => Some("wait"),
        (
            GoalVerificationApplyOutcome::Rearmed
            | GoalVerificationApplyOutcome::Continued
            | GoalVerificationApplyOutcome::Stalled,
            GoalVerdict::Unmet(_),
        ) => Some("unmet"),
        _ => None,
    };
    verdict_extra(validation_evidence_verdict(outcome, verdict), marker)
}

fn format_gaps(gaps: &[String]) -> String {
    if gaps.is_empty() {
        return "- verification inconclusive; continue".to_string();
    }
    gaps.iter()
        .map(|gap| format!("- {gap}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_message_with_extra(
    tool_call_id: &str,
    content: String,
    extra: Map<String, Value>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content),
            tool_call_id: tool_call_id.to_string(),
            extra,
            ..Default::default()
        })],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::GoalBudget;

    use crate::app_state::AppState;
    use crate::chat::goal_verifier::{apply_goal_verdict, GoalVerifierReply};
    use crate::chat::types::{GoalStatus, SessionState};
    use crate::tools::tools_list::get_tools_for_mode;

    const CHAT_ID: &str = "validate-goal-chat";

    async fn ccx_for_session(
        session: ChatSession,
    ) -> (
        Arc<crate::global_context::GlobalContext>,
        Arc<AMutex<AtCommandsContext>>,
    ) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();
        gcx.chat_sessions
            .write()
            .await
            .insert(CHAT_ID.to_string(), Arc::new(AMutex::new(session)));
        (gcx.clone(), make_ccx(gcx).await)
    }

    async fn make_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                CHAT_ID.to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn tool_content(result: (bool, Vec<ContextEnum>)) -> String {
        assert!(!result.0);
        match result.1.into_iter().next().expect("tool output") {
            ContextEnum::ChatMessage(message) => message.content.content_text_only(),
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        }
    }

    fn session_with_goal() -> ChatSession {
        let mut session = ChatSession::new(CHAT_ID.to_string());
        session.install_goal(
            "agent",
            "ship feature\n- tests pass",
            true,
            GoalBudget::default(),
        );
        session
    }

    fn finite_budget() -> GoalBudget {
        GoalBudget {
            max_turns: Some(1),
            max_minutes: None,
            max_tokens: None,
            max_cost_cents: None,
            cooldown_ms: 1_500,
            no_progress_token_threshold: 50,
            no_progress_turns: Some(1),
            explicit: true,
        }
    }

    fn session_with_exhausted_goal() -> ChatSession {
        let mut session = ChatSession::new(CHAT_ID.to_string());
        session.install_goal("agent", "ship feature\n- tests pass", true, finite_budget());
        session.goal.as_mut().unwrap().progress.turns_used = 1;
        session.refresh_goal_runtime_mirror();
        session
    }

    #[tokio::test]
    async fn available_in_plan_supporting_modes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();

        for mode in ["agent", "task_agent", "task_planner"] {
            let has_tool = get_tools_for_mode(gcx.clone(), mode, None)
                .await
                .into_iter()
                .any(|tool| tool.tool_description().name == "validate_goal");
            assert!(has_tool, "{mode} should expose validate_goal");
        }
        for mode in ["NO_TOOLS", "shell", "explore"] {
            let has_tool = get_tools_for_mode(gcx.clone(), mode, None)
                .await
                .into_iter()
                .any(|tool| tool.tool_description().name == "validate_goal");
            assert!(!has_tool, "{mode} should not expose validate_goal");
        }
    }

    #[tokio::test]
    async fn no_goal_returns_clean_message() {
        let (gcx, ccx) = ccx_for_session(ChatSession::new(CHAT_ID.to_string())).await;
        let mut tool = ToolValidateGoal {
            config_path: String::new(),
        };

        let content = tool_content(
            tool.tool_execute(ccx, &"call".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(content, "No active goal to validate.");
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        assert!(session.goal.is_none());
        assert!(session.command_queue.is_empty());
    }

    #[tokio::test]
    async fn missing_child_session_is_restored_from_trajectory() {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx.clone()).await;
        let mut session = ChatSession::new("subchat-validate-goal".to_string());
        session.install_goal("agent", "ship feature", true, GoalBudget::default());
        let snapshot = crate::chat::trajectories::trajectory_snapshot_from_session(&session);
        crate::chat::trajectories::save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();
        app.chat
            .sessions
            .write()
            .await
            .remove("subchat-validate-goal");
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "subchat-validate-goal".to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ));
        let mut tool = ToolValidateGoal {
            config_path: String::new(),
        };

        let content = tool_content(
            tool.tool_execute(ccx, &"call".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert!(!content.contains("chat session"));
        assert_ne!(content, "No active goal to validate.");
        assert!(content.contains("GOAL") || content.contains("Goal validation"));
        assert!(app
            .chat
            .sessions
            .read()
            .await
            .contains_key("subchat-validate-goal"));
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get("subchat-validate-goal")
            .cloned()
            .unwrap();
        assert_eq!(
            session.lock().await.goal.as_ref().unwrap().content,
            "ship feature"
        );
    }

    #[tokio::test]
    async fn verifying_goal_returns_clean_message_without_verifier_call() {
        let mut session = session_with_goal();
        session.goal_set_status(GoalStatus::Verifying);
        let (gcx, ccx) = ccx_for_session(session).await;
        let mut tool = ToolValidateGoal {
            config_path: String::new(),
        };

        let content = tool_content(
            tool.tool_execute(ccx, &"call".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(content, "No active goal to validate.");
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        assert_eq!(session.goal_status, Some(GoalStatus::Verifying));
        assert!(session.goal.as_ref().unwrap().attempts.is_empty());
    }

    #[tokio::test]
    async fn inactive_goal_returns_clean_message_without_status_change() {
        let mut session = session_with_goal();
        session.goal_set_status(GoalStatus::Paused);
        let (gcx, ccx) = ccx_for_session(session).await;
        let mut tool = ToolValidateGoal {
            config_path: String::new(),
        };

        let content = tool_content(
            tool.tool_execute(ccx, &"call".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(content, "No active goal to validate.");
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        let goal = session.goal.as_ref().unwrap();
        assert_eq!(goal.status, GoalStatus::Paused);
        assert!(goal.attempts.is_empty());
        assert!(session.command_queue.is_empty());
    }

    #[tokio::test]
    async fn budget_exhausted_goal_returns_distinct_message_without_verifier_call() {
        let (gcx, ccx) = ccx_for_session(session_with_exhausted_goal()).await;
        let mut tool = ToolValidateGoal {
            config_path: String::new(),
        };

        let content = tool_content(
            tool.tool_execute(ccx, &"call".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(content, budget_exhausted_message());
        assert_ne!(content, "No active goal to validate.");
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        let goal = session.goal.as_ref().unwrap();
        assert_eq!(goal.status, GoalStatus::Active);
        assert!(goal.attempts.is_empty());
        assert!(session.post_tool_side_effects.is_empty());
    }

    #[tokio::test]
    async fn verifier_error_cleanup_restores_goal_status_without_forcing_idle() {
        let mut session = session_with_goal();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        session.goal_set_status(GoalStatus::Verifying);
        let session_arc = Arc::new(AMutex::new(session));

        reset_after_verifier_error(session_arc.clone(), "verifier model unavailable").await;

        let session = session_arc.lock().await;
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert_eq!(session.runtime.state, SessionState::ExecutingTools);
        assert!(session.goal_verification_blocked_until_ms.is_some());
    }

    #[test]
    fn every_receipt_path_carries_one_of_the_six_engine_verdicts() {
        let paths = [
            verdict_extra(EvidenceVerdict::NotRun, None),
            verdict_extra(EvidenceVerdict::Blocked, None),
            verdict_extra(EvidenceVerdict::Unavailable, None),
            validation_extra(GoalVerificationApplyOutcome::Finalized, &GoalVerdict::Met),
            validation_extra(GoalVerificationApplyOutcome::Superseded, &GoalVerdict::Met),
            validation_extra(
                GoalVerificationApplyOutcome::Continued,
                &GoalVerdict::Unmet(vec!["gap".to_string()]),
            ),
            validation_extra(GoalVerificationApplyOutcome::NoGoal, &GoalVerdict::Met),
        ];
        let emitted: Vec<String> = paths
            .iter()
            .map(|extra| {
                extra["validate_goal"]["evidence_verdict"]
                    .as_str()
                    .expect("every receipt path carries a structured verdict")
                    .to_string()
            })
            .collect();

        for label in &emitted {
            assert!(
                EvidenceVerdict::ALL
                    .iter()
                    .any(|verdict| verdict.label() == label),
                "{label} is not a verdict the engine vocabulary defines"
            );
        }
        assert_eq!(
            emitted,
            vec![
                "not_run",
                "blocked",
                "unavailable",
                "passed",
                "superseded",
                "failed",
                "not_run",
            ]
        );
    }

    #[test]
    fn begin_validate_goal_sets_verifying_for_active_goal_only() {
        let mut session = session_with_goal();

        let begin = begin_validate_goal(&mut session);

        assert_eq!(begin, GoalVerificationBegin::Started);
        assert_eq!(session.goal_status, Some(GoalStatus::Verifying));

        let mut paused = session_with_goal();
        paused.goal_set_status(GoalStatus::Paused);
        let begin = begin_validate_goal(&mut paused);

        assert_eq!(begin, GoalVerificationBegin::NoActiveGoal);
        assert_eq!(paused.goal_status, Some(GoalStatus::Paused));
    }

    #[test]
    fn met_verdict_finalizes_and_formats_tool_message() {
        let mut session = session_with_goal();
        session.goal_set_status(GoalStatus::Verifying);
        let reply = GoalVerifierReply {
            verdict: GoalVerdict::Met,
            verifier_reply: "GOAL: MET".to_string(),
            tokens: 11,
            criteria_verdicts: Vec::new(),
        };
        let verdict = reply.verdict.clone();
        let verifier_reply = reply.verifier_reply.clone();

        let outcome = apply_goal_verdict(&mut session, "validate_goal", reply, None);
        let content = validation_content(outcome, &verdict, &verifier_reply);

        assert_eq!(outcome, GoalVerificationApplyOutcome::Finalized);
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert_eq!(session.goal_status, Some(GoalStatus::Completed));
        assert_eq!(session.messages.len(), 2);
        assert!(session.post_tool_side_effects.is_empty());
        assert_eq!(
            session.messages.last().unwrap().extra["event"]["payload"]["kind"],
            json!("verified")
        );
        assert_eq!(
            content,
            "GOAL MET — goal marked complete; pursuit disabled.\n\nGOAL: MET"
        );
        assert_eq!(
            validation_extra(outcome, &verdict)["validate_goal"]["verdict"],
            json!("met")
        );
        assert_eq!(
            validation_extra(outcome, &verdict)["validate_goal"]["evidence_verdict"],
            json!("passed")
        );
    }

    #[test]
    fn unmet_verdict_continues_and_formats_gaps() {
        let mut session = session_with_goal();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        session.goal_set_status(GoalStatus::Verifying);
        let reply = GoalVerifierReply {
            verdict: GoalVerdict::Unmet(vec!["missing tests".to_string(), "docs".to_string()]),
            verifier_reply: "GOAL: UNMET\n- missing tests\n- docs".to_string(),
            tokens: 13,
            criteria_verdicts: Vec::new(),
        };
        let verdict = reply.verdict.clone();
        let verifier_reply = reply.verifier_reply.clone();

        let outcome = apply_goal_verdict(&mut session, "validate_goal", reply, None);
        let content = validation_content(outcome, &verdict, &verifier_reply);

        assert_eq!(outcome, GoalVerificationApplyOutcome::Continued);
        assert_eq!(session.runtime.state, SessionState::ExecutingTools);
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 1);
        assert_eq!(
            session.goal.as_ref().unwrap().attempts[0].gaps,
            vec!["missing tests", "docs"]
        );
        assert_eq!(session.messages.len(), 2);
        assert!(session.post_tool_side_effects.is_empty());
        assert_eq!(
            session.messages.last().unwrap().extra["event"]["payload"]["kind"],
            json!("verification_gaps")
        );
        assert!(session.command_queue.is_empty());
        assert_eq!(
            validation_extra(outcome, &verdict)["validate_goal"]["verdict"],
            json!("unmet")
        );
        assert_eq!(
            validation_extra(outcome, &verdict)["validate_goal"]["evidence_verdict"],
            json!("failed")
        );
        assert_eq!(
            content,
            "GOAL NOT YET MET — remaining gaps:\n- missing tests\n- docs\n\nGOAL: UNMET\n- missing tests\n- docs"
        );
    }
}
