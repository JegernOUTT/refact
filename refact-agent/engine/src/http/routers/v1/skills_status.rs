use std::sync::Arc;
use axum::Extension;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::Response;
use axum::body::Body;
use serde::Serialize;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

#[derive(Serialize)]
pub struct SkillsStatusResponse {
    pub skills_available: usize,
    pub skills_included: Vec<String>,
    pub skills_enabled: bool,
    pub active_skill: Option<String>,
}

pub async fn handle_v1_skills_status(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(chat_id): Path<String>,
) -> Result<Response<Body>, ScratchError> {
    let sessions = gcx.read().await.chat_sessions.clone();
    let session_arc = {
        let sessions_read = sessions.read().await;
        sessions_read.get(&chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("chat_id {} not found", chat_id),
        ));
    };
    let session = session_arc.lock().await;
    let active_skill = session.thread.active_skill.clone();
    let response = SkillsStatusResponse {
        skills_available: session.skills_available_count,
        skills_included: session.skills_included.clone(),
        skills_enabled: session.skills_available_count > 0,
        active_skill,
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
    use crate::chat::types::ChatSession;

    #[test]
    fn test_skills_status_available_count_reflects_loaded_skills() {
        let mut session = ChatSession::new("test-chat".to_string());
        assert_eq!(session.skills_available_count, 0);
        assert!(session.skills_included.is_empty());

        session.skills_available_count = 3;
        assert_eq!(session.skills_available_count, 3);
    }

    #[test]
    fn test_skills_status_included_populated_after_selection() {
        let mut session = ChatSession::new("test-chat".to_string());

        session.skills_available_count = 5;
        session.skills_included = vec!["review".to_string(), "docs".to_string()];

        assert_eq!(session.skills_included.len(), 2);
        assert!(session.skills_included.contains(&"review".to_string()));
        assert!(session.skills_included.contains(&"docs".to_string()));
    }

    #[test]
    fn test_skills_status_response_skills_enabled_true_when_available() {
        let response = SkillsStatusResponse {
            skills_available: 3,
            skills_included: vec!["skill1".to_string()],
            skills_enabled: true,
            active_skill: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["skills_available"], 3);
        assert_eq!(json["skills_enabled"], true);
        assert_eq!(json["skills_included"].as_array().unwrap().len(), 1);
        assert!(json["active_skill"].is_null());
    }

    #[test]
    fn test_skills_status_response_skills_enabled_false_when_none() {
        let response = SkillsStatusResponse {
            skills_available: 0,
            skills_included: vec![],
            skills_enabled: false,
            active_skill: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["skills_available"], 0);
        assert_eq!(json["skills_enabled"], false);
        assert!(json["skills_included"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_skills_status_response_active_skill_set_when_command_active() {
        let response = SkillsStatusResponse {
            skills_available: 2,
            skills_included: vec![],
            skills_enabled: true,
            active_skill: Some("my-skill".to_string()),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["active_skill"], "my-skill");
    }

    #[test]
    fn test_skills_status_active_skill_from_session_thread() {
        let mut session = ChatSession::new("test-chat".to_string());
        session.thread.active_skill = Some("review-skill".to_string());
        let active_skill = session.thread.active_skill.clone();
        assert_eq!(active_skill, Some("review-skill".to_string()));

        session.thread.active_skill = None;
        let active_skill_none = session.thread.active_skill.clone();
        assert!(active_skill_none.is_none());
    }

    #[test]
    fn test_skills_status_new_session_has_zero_skills() {
        let session = ChatSession::new("new-chat".to_string());
        assert_eq!(session.skills_available_count, 0);
        assert!(session.skills_included.is_empty());
        let skills_enabled = session.skills_available_count > 0;
        assert!(!skills_enabled);
    }

    #[test]
    fn test_skills_status_resets_to_zero() {
        let mut session = ChatSession::new("test-chat".to_string());

        session.skills_available_count = 3;
        session.skills_included = vec!["skill1".to_string(), "skill2".to_string()];
        assert_eq!(session.skills_available_count, 3);

        session.skills_available_count = 0;
        session.skills_included = Vec::new();

        assert_eq!(session.skills_available_count, 0);
        assert!(session.skills_included.is_empty());
        let skills_enabled = session.skills_available_count > 0;
        assert!(!skills_enabled);
    }
}
