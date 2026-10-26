use axum::extract::{Query, State};
use refact_chat_api::{GoalStatus, SessionState};
use serde::{Deserialize, Serialize};
use crate::{app_state::AppState, custom_error::ScratchError};

#[derive(Deserialize)]
pub struct NativeWorkQuery {
    after: Option<String>,
}

#[derive(Serialize)]
pub struct NativeWorkSnapshot {
    schema_version: u16,
    project_root: String,
    coverage: &'static str,
    next_cursor: Option<String>,
    sessions: Vec<NativeWorkView>,
}

#[derive(Serialize)]
pub struct NativeGoalView {
    version: u32,
    content: String,
    active: bool,
    status: GoalStatus,
    turns_used: u32,
    tokens_used: u64,
    max_turns: Option<u32>,
    max_tokens: Option<u64>,
    latest_verdict: Option<String>,
    gaps: Vec<String>,
    verification_blocked: bool,
    truncated: bool,
}

#[derive(Serialize)]
pub struct NativeAgentView {
    agent_id: String,
    title: String,
    status: String,
    model: String,
    progress: Option<String>,
}

#[derive(Serialize)]
pub struct NativeWorkView {
    chat_id: String,
    title: String,
    model: String,
    state: SessionState,
    persistence_pending: bool,
    goal: Option<NativeGoalView>,
    agents: Vec<NativeAgentView>,
    agents_truncated: bool,
    pending_questions: usize,
    approval_count: usize,
    browser_decision_pending: bool,
    error: Option<String>,
}

fn excerpt(text: &str, limit: usize) -> String {
    crate::buddy::jobs::autonomous_chats::redact_and_cap_text(text, limit)
}

fn project_session(session: &crate::chat::types::ChatSession) -> NativeWorkView {
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let goal = session.goal.as_ref().map(|goal| {
        let latest = goal.attempts.last();
        NativeGoalView {
            version: goal.version,
            content: excerpt(&goal.content, 4000),
            active: goal.active,
            status: goal.status.clone(),
            turns_used: goal.progress.turns_used,
            tokens_used: goal.progress.tokens_used,
            max_turns: goal.budget.max_turns.filter(|value| *value > 0),
            max_tokens: goal.budget.max_tokens.filter(|value| *value > 0),
            latest_verdict: latest.map(|attempt| excerpt(&attempt.verdict, 80)),
            gaps: latest
                .map(|attempt| {
                    attempt
                        .gaps
                        .iter()
                        .take(10)
                        .map(|gap| excerpt(gap, 500))
                        .collect()
                })
                .unwrap_or_default(),
            verification_blocked: session
                .goal_verification_blocked_until_ms
                .is_some_and(|until| until > now),
            truncated: goal.content.chars().count() > 4000
                || latest.is_some_and(|attempt| {
                    attempt.gaps.len() > 10
                        || attempt.gaps.iter().any(|gap| gap.chars().count() > 500)
                }),
        }
    });
    let mut agents: Vec<_> = session.background_agents.values().collect();
    agents.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    let agents_truncated = agents.len() > 20;
    NativeWorkView {
        chat_id: session.chat_id.clone(),
        title: excerpt(&session.thread.title, 240),
        model: session.thread.model.clone(),
        state: session.runtime.state,
        persistence_pending: session.trajectory_dirty,
        goal,
        agents: agents
            .into_iter()
            .take(20)
            .map(|agent| NativeAgentView {
                agent_id: agent.agent_id.clone(),
                title: excerpt(&agent.title, 240),
                status: agent.status.clone(),
                model: agent.model.clone(),
                progress: agent.progress.as_deref().map(|text| excerpt(text, 500)),
            })
            .collect(),
        agents_truncated,
        pending_questions: session
            .pending_questions
            .iter()
            .filter(|question| question.status == refact_chat_api::PendingQuestionStatus::Pending)
            .count(),
        approval_count: session.runtime.pause_reasons.len(),
        browser_decision_pending: session.pending_browser_message.is_some(),
        error: session
            .runtime
            .error
            .as_deref()
            .map(|text| excerpt(text, 1000)),
    }
}

pub async fn handle_native_work(
    State(app): State<AppState>,
    Query(query): Query<NativeWorkQuery>,
) -> Result<axum::Json<NativeWorkSnapshot>, ScratchError> {
    let project_root = super::buddy_decisions::decision_root(&app).await?;
    if query.after.as_ref().is_some_and(|after| after.len() > 256) {
        return Err(ScratchError::new(
            hyper::StatusCode::BAD_REQUEST,
            "invalid native work cursor".into(),
        ));
    }
    let mut sessions: Vec<_> = app
        .chat
        .sessions
        .read()
        .await
        .iter()
        .filter(|(id, _)| query.after.as_ref().is_none_or(|after| *id > after))
        .map(|(id, session)| (id.clone(), session.clone()))
        .collect();
    sessions.sort_by(|a, b| a.0.cmp(&b.0));
    let has_more = sessions.len() > 20;
    sessions.truncate(20);
    let next_cursor = has_more
        .then(|| sessions.last().map(|row| row.0.clone()))
        .flatten();
    let mut views = Vec::with_capacity(sessions.len());
    for (_, session) in sessions {
        let guard = tokio::time::timeout(std::time::Duration::from_millis(100), session.lock())
            .await
            .map_err(|_| {
                ScratchError::new(
                    hyper::StatusCode::SERVICE_UNAVAILABLE,
                    "native work is busy; refresh the snapshot".into(),
                )
            })?;
        views.push(project_session(&guard));
    }
    Ok(axum::Json(NativeWorkSnapshot {
        schema_version: 1,
        project_root,
        coverage: "loaded_native_sessions",
        next_cursor,
        sessions: views,
    }))
}
