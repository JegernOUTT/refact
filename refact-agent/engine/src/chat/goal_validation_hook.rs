use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::chat::goal_verifier::{GoalVerdict, GoalVerifierReply};
use crate::chat::types::ChatSession;
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{HookPayload, get_project_dir_string, run_observation_hooks};

pub(crate) fn goal_validation_extra(
    result: &Result<GoalVerifierReply, String>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut extra = serde_json::Map::new();
    let (outcome, gaps, wait, criteria, judge_tokens, error) = match result {
        Ok(reply) => {
            let (outcome, gaps, wait) = match &reply.verdict {
                GoalVerdict::Met => ("met", Vec::new(), serde_json::Value::Null),
                GoalVerdict::Unmet(gaps) => (
                    "unmet",
                    gaps.iter()
                        .take(20)
                        .map(|gap| truncate_chars(gap, 500))
                        .collect(),
                    serde_json::Value::Null,
                ),
                GoalVerdict::Wait(wait) => (
                    "wait",
                    Vec::new(),
                    serde_json::to_value(wait).unwrap_or(serde_json::Value::Null),
                ),
            };
            let criteria = reply
                .criteria_verdicts
                .iter()
                .take(50)
                .map(|criterion| {
                    serde_json::json!({
                        "id": criterion.id,
                        "met": criterion.met,
                        "evidence_verdict": criterion.evidence.as_ref().map(|evidence| &evidence.verdict),
                    })
                })
                .collect::<Vec<_>>();
            (
                outcome,
                gaps,
                wait,
                criteria,
                reply.tokens,
                serde_json::Value::Null,
            )
        }
        Err(error) => (
            "error",
            Vec::new(),
            serde_json::Value::Null,
            Vec::new(),
            0,
            serde_json::Value::String(truncate_chars(error, 2000)),
        ),
    };
    extra.insert("outcome".into(), serde_json::json!(outcome));
    extra.insert("gaps".into(), serde_json::json!(gaps));
    extra.insert("wait".into(), wait);
    extra.insert("criteria".into(), serde_json::json!(criteria));
    extra.insert("judge_tokens".into(), serde_json::json!(judge_tokens));
    extra.insert("error".into(), error);
    extra
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

pub(crate) async fn emit_goal_validation(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    result: &Result<GoalVerifierReply, String>,
) {
    if !app.gcx.hooks_config.enabled {
        return;
    }
    let (chat_id, worktree_root, goal_version) = {
        let session = session_arc.lock().await;
        (
            session.chat_id.clone(),
            session
                .thread
                .worktree
                .as_ref()
                .map(|worktree| worktree.root.clone()),
            session.goal.as_ref().map(|goal| goal.version),
        )
    };
    let project_dir = match worktree_root {
        Some(root) => root.to_string_lossy().into_owned(),
        None => get_project_dir_string(app.clone()).await,
    };
    let event = HookEvent::GoalValidation;
    let mut payload = HookPayload::new(&event, chat_id, project_dir);
    payload.extra.extend(goal_validation_extra(result));
    payload.extra.insert(
        "goal_version".into(),
        goal_version
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    );
    run_observation_hooks(app.clone(), event, payload).await;
}
