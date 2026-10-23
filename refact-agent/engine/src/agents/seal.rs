pub use refact_chat_api::SealReason;

use crate::app_state::AppState;
use crate::tasks::storage;

pub async fn is_sealed(app: &AppState, chat_id: &str) -> bool {
    seal_reason(app, chat_id).await.is_some()
}

pub async fn seal_reason(app: &AppState, chat_id: &str) -> Option<SealReason> {
    if let Some((agent_id, status, run_generation)) = app.agents.child_chat_status(chat_id).await {
        return status.is_terminal().then(|| SealReason::AgentTerminal {
            agent_id,
            status: status.as_str().to_string(),
            run_generation,
        });
    }

    let session = app.chat.sessions.read().await.get(chat_id).cloned()?;
    let task_meta = session.lock().await.thread.task_meta.clone()?;
    if task_meta.role != "agents" {
        return None;
    }
    let card_id = task_meta.card_id?;
    let card = match storage::card_seal_state(app.gcx.clone(), &task_meta.task_id, &card_id).await {
        Ok(card) => card,
        Err(error) => {
            tracing::warn!(chat_id, task_id = %task_meta.task_id, %error, "Failed to read task board for chat seal");
            return None;
        }
    };
    let column = match card {
        None => "deleted".to_string(),
        Some(card)
            if card.column != "doing"
                || card
                    .agent_chat_id
                    .as_deref()
                    .is_some_and(|owner| owner != chat_id) =>
        {
            card.column.clone()
        }
        Some(_) => return None,
    };
    Some(SealReason::TaskCardLeftDoing {
        task_id: task_meta.task_id,
        card_id,
        column,
    })
}
