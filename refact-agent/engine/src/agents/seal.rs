pub use refact_chat_api::SealReason;

use crate::app_state::AppState;

pub async fn seal_reason(app: &AppState, chat_id: &str) -> Option<SealReason> {
    let record = app
        .agents
        .list_all()
        .await
        .into_iter()
        .find(|record| record.child_chat_id.as_deref() == Some(chat_id))?;
    if !record.status.is_terminal() {
        return None;
    }
    Some(SealReason::AgentTerminal {
        agent_id: record.agent_id,
        status: record.status.as_str().to_string(),
        run_generation: record.run_generation,
    })
}
