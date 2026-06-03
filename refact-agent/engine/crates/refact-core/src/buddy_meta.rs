use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuddyThreadMeta {
    pub is_buddy_chat: bool,
    pub buddy_chat_kind: String,
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_buddy_thread_meta_without_goal_id_deserializes() {
        let meta: BuddyThreadMeta = serde_json::from_str(
            r#"{
                "is_buddy_chat": true,
                "buddy_chat_kind": "system",
                "workflow_id": null
            }"#,
        )
        .unwrap();

        assert!(meta.goal_id.is_none());
    }

    #[test]
    fn buddy_thread_meta_goal_id_round_trips() {
        let meta = BuddyThreadMeta {
            is_buddy_chat: true,
            buddy_chat_kind: "conductor".to_string(),
            workflow_id: Some("workflow-1".to_string()),
            goal_id: Some("goal-1".to_string()),
        };

        let encoded = serde_json::to_string(&meta).unwrap();
        let decoded: BuddyThreadMeta = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, meta);
    }
}
