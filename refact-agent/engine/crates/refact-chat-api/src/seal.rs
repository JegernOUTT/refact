use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SealReason {
    AgentTerminal {
        agent_id: String,
        status: String,
        run_generation: u64,
    },
    TaskCardLeftDoing {
        task_id: String,
        card_id: String,
        column: String,
    },
}

impl SealReason {
    pub fn message(&self) -> String {
        match self {
            Self::AgentTerminal {
                status,
                run_generation,
                ..
            } => format!(
                "This background agent finished (status: {status}, generation {run_generation}). Its worktree may have been merged or removed. To continue, ask its parent to call agent_lifecycle with action=continue; to start new work, spawn a new agent."
            ),
            Self::TaskCardLeftDoing {
                task_id,
                card_id,
                column,
            } => format!(
                "This task card left doing (task: {task_id}, card: {card_id}, column: {column}). Its task run has ended. Move the card back to doing to continue, or start a new task card for new work."
            ),
        }
    }
}
