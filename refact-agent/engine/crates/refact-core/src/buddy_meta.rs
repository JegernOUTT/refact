use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyThreadMeta {
    pub is_buddy_chat: bool,
    pub buddy_chat_kind: String,
    pub workflow_id: Option<String>,
}
