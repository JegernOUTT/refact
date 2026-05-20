use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserAction {
    FileOpened {
        path: String,
        ts: DateTime<Utc>,
    },
    SnippetSelected {
        path: String,
        lines: (u32, u32),
        ts: DateTime<Utc>,
    },
    ToolApproved {
        tool_name: String,
        chat_id: String,
        ts: DateTime<Utc>,
    },
    ToolRejected {
        tool_name: String,
        chat_id: String,
        ts: DateTime<Utc>,
    },
    CommandRun {
        command_preview: String,
        chat_id: String,
        ts: DateTime<Utc>,
    },
    WorkspaceChanged {
        folders_added: Vec<String>,
        folders_removed: Vec<String>,
        ts: DateTime<Utc>,
    },
    CommitMade {
        sha: String,
        message_first_line: String,
        files: u32,
        ts: DateTime<Utc>,
    },
    TaskFailed {
        task_id: String,
        reason_short: String,
        ts: DateTime<Utc>,
    },
    ChatStarted {
        chat_id: String,
        first_user_text_preview: String,
        ts: DateTime<Utc>,
    },
}

impl UserAction {
    pub fn ts(&self) -> DateTime<Utc> {
        match self {
            UserAction::FileOpened { ts, .. }
            | UserAction::SnippetSelected { ts, .. }
            | UserAction::ToolApproved { ts, .. }
            | UserAction::ToolRejected { ts, .. }
            | UserAction::CommandRun { ts, .. }
            | UserAction::WorkspaceChanged { ts, .. }
            | UserAction::CommitMade { ts, .. }
            | UserAction::TaskFailed { ts, .. }
            | UserAction::ChatStarted { ts, .. } => *ts,
        }
    }
}
