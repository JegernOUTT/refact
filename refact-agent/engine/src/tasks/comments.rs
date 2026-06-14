use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{CardComment, TaskBoard};

pub(crate) const COMMENT_CAP: usize = 500;
pub(crate) const COMMENT_BODY_MAX_CHARS: usize = 4000;

#[derive(Clone)]
pub(crate) struct CreateCardComment {
    pub card_id: String,
    pub body: String,
    pub author_role: String,
    pub author_id: Option<String>,
    pub reply_to: Option<String>,
}

pub(crate) fn add_comment_to_board(
    board: &mut TaskBoard,
    request: CreateCardComment,
) -> Result<CardComment, String> {
    validate_author_role(&request.author_role)?;
    let body = request.body.trim();
    if body.is_empty() {
        return Err("comment body is empty".to_string());
    }
    let card = board
        .get_card_mut(&request.card_id)
        .ok_or_else(|| format!("Card {} not found", request.card_id))?;
    if card.comments.len() >= COMMENT_CAP {
        tracing::warn!("Card {} comment cap reached", card.id);
        return Err(format!(
            "Card {} already has the maximum {} comments",
            card.id, COMMENT_CAP
        ));
    }
    if let Some(reply_id) = request.reply_to.as_deref() {
        if !card.comments.iter().any(|comment| comment.id == reply_id) {
            return Err("reply_to references unknown comment".to_string());
        }
    }
    let comment = CardComment {
        id: Uuid::new_v4().to_string(),
        author_role: request.author_role,
        author_id: request.author_id,
        timestamp: Utc::now().to_rfc3339(),
        body: truncate_chars(body, COMMENT_BODY_MAX_CHARS),
        reply_to: request.reply_to,
    };
    card.last_heartbeat_at = Some(comment.timestamp.clone());
    card.comments.push(comment.clone());
    Ok(comment)
}

pub(crate) async fn create_card_comment(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    request: CreateCardComment,
) -> Result<(TaskBoard, CardComment), String> {
    let (board, comment) = storage::update_board_atomic(gcx, task_id, move |board| {
        Ok(Some(add_comment_to_board(board, request)?))
    })
    .await?;
    let comment = comment.ok_or_else(|| "comment was not created".to_string())?;
    Ok((board, comment))
}

fn validate_author_role(role: &str) -> Result<(), String> {
    match role {
        "planner" | "agents" | "user" | "system" | "http" => Ok(()),
        _ => Err("invalid author_role".to_string()),
    }
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    if max == 0 {
        return String::new();
    }
    format!(
        "{}…",
        value
            .chars()
            .take(max.saturating_sub(1))
            .collect::<String>()
    )
}
