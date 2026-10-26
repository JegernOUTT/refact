use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::Response;
use axum::body::Body;
use axum::extract::State;
use serde::Serialize;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;

#[derive(Serialize, Default)]
pub struct SkillsStatusResponse {
    pub skills_available: usize,
    pub skills_included: Vec<String>,
    pub skills_enabled: bool,
    pub active_skill: Option<String>,
}

pub async fn handle_v1_skills_status(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let sessions = gcx.chat_sessions.clone();
    let session_arc = {
        let sessions_read = sessions.read().await;
        sessions_read.get(&chat_id).cloned()
    };
    let response = match session_arc {
        Some(session_arc) => {
            let session = session_arc.lock().await;
            SkillsStatusResponse {
                skills_available: session.skills_available_count,
                skills_included: session.skills_included.clone(),
                skills_enabled: session.skills_available_count > 0,
                active_skill: session.thread.active_skill.clone(),
            }
        }
        None => SkillsStatusResponse::default(),
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

#[cfg(test)]
mod tests {

    use super::*;
}
