use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::goal_verifier::{
    apply_goal_verdict, begin_goal_verification_if_needed, run_goal_verifier,
    GoalVerificationApplyOutcome, GoalVerificationBegin, GoalVerdict,
};
use crate::chat::types::{ChatSession, GoalStatus, SessionState};
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
                "properties": {},
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
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (app, chat_id) = {
            let ccx = ccx.lock().await;
            (ccx.app.clone(), ccx.chat_id.clone())
        };
        let gcx = app.gcx.clone();
        let session_arc = {
            let sessions = gcx.chat_sessions.read().await;
            sessions.get(&chat_id).cloned()
        }
        .ok_or_else(|| format!("chat session `{chat_id}` not found"))?;

        let begin = {
            let mut session = session_arc.lock().await;
            begin_validate_goal(&mut session)
        };
        if begin != GoalVerificationBegin::Started {
            return tool_message(tool_call_id, "No active goal to validate.".to_string());
        }

        let reply = match run_goal_verifier(app, session_arc.clone()).await {
            Ok(reply) => reply,
            Err(error) => {
                reset_after_verifier_error(session_arc.clone()).await;
                return tool_message(
                    tool_call_id,
                    format!("Goal validation could not run: {error}"),
                );
            }
        };

        let verdict = reply.verdict.clone();
        let verifier_reply = reply.verifier_reply.clone();
        let content = {
            let mut session = session_arc.lock().await;
            let outcome = apply_goal_verdict(&mut session, "validate_goal", reply);
            validation_content(outcome, &verdict, &verifier_reply)
        };
        tool_message(tool_call_id, content)
    }
}

fn has_active_goal(session: &ChatSession) -> bool {
    session
        .goal
        .as_ref()
        .is_some_and(|goal| goal.active && goal.status == GoalStatus::Active)
}

fn begin_validate_goal(session: &mut ChatSession) -> GoalVerificationBegin {
    if !has_active_goal(session) {
        return GoalVerificationBegin::NoActiveGoal;
    }
    begin_goal_verification_if_needed(session)
}

async fn reset_after_verifier_error(session_arc: Arc<AMutex<ChatSession>>) {
    let mut session = session_arc.lock().await;
    if session.goal_status == Some(GoalStatus::Verifying) {
        session.goal_set_status(GoalStatus::Active);
    }
    session.set_runtime_state(SessionState::Idle, None);
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
        GoalVerificationApplyOutcome::Rearmed => {
            let gaps = match verdict {
                GoalVerdict::Unmet(gaps) => gaps.as_slice(),
                GoalVerdict::Met => &[],
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

fn format_gaps(gaps: &[String]) -> String {
    if gaps.is_empty() {
        return "- verification inconclusive; continue".to_string();
    }
    gaps.iter()
        .map(|gap| format!("- {gap}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_message(tool_call_id: &str, content: String) -> Result<(bool, Vec<ContextEnum>), String> {
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        })],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::GoalBudget;

    use crate::app_state::AppState;
    use crate::chat::goal_verifier::GoalVerifierReply;
    use crate::chat::types::ChatCommand;
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
        };
        let verdict = reply.verdict.clone();
        let verifier_reply = reply.verifier_reply.clone();

        let outcome = apply_goal_verdict(&mut session, "validate_goal", reply);
        let content = validation_content(outcome, &verdict, &verifier_reply);

        assert_eq!(outcome, GoalVerificationApplyOutcome::Finalized);
        assert_eq!(session.runtime.state, SessionState::Completed);
        assert_eq!(session.goal_status, Some(GoalStatus::Completed));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.post_tool_side_effects.len(), 1);
        assert_eq!(
            session.post_tool_side_effects[0].extra["event"]["payload"]["kind"],
            json!("verified")
        );
        assert_eq!(
            content,
            "GOAL MET — goal marked complete; pursuit disabled.\n\nGOAL: MET"
        );
    }

    #[test]
    fn unmet_verdict_rearms_and_formats_gaps() {
        let mut session = session_with_goal();
        session.goal_set_status(GoalStatus::Verifying);
        let reply = GoalVerifierReply {
            verdict: GoalVerdict::Unmet(vec!["missing tests".to_string(), "docs".to_string()]),
            verifier_reply: "GOAL: UNMET\n- missing tests\n- docs".to_string(),
            tokens: 13,
        };
        let verdict = reply.verdict.clone();
        let verifier_reply = reply.verifier_reply.clone();

        let outcome = apply_goal_verdict(&mut session, "validate_goal", reply);
        let content = validation_content(outcome, &verdict, &verifier_reply);

        assert_eq!(outcome, GoalVerificationApplyOutcome::Rearmed);
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert_eq!(
            session.goal.as_ref().unwrap().attempts[0].gaps,
            vec!["missing tests", "docs"]
        );
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.post_tool_side_effects.len(), 1);
        assert_eq!(
            session.post_tool_side_effects[0].extra["event"]["payload"]["kind"],
            json!("verification_gaps")
        );
        assert!(session
            .command_queue
            .iter()
            .any(|request| matches!(request.command, ChatCommand::Regenerate {})));
        assert_eq!(
            content,
            "GOAL NOT YET MET — remaining gaps:\n- missing tests\n- docs\n\nGOAL: UNMET\n- missing tests\n- docs"
        );
    }
}
