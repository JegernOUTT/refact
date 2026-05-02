use std::path::Path;
use tokio::fs;
use tracing::warn;

use super::autonomous_workflows::autonomous_workflow_meta;
use super::types::BuddyConversationEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuddyWorkflowMapping {
    pub kind: &'static str,
    pub icon: &'static str,
    pub badge: Option<&'static str>,
}

pub fn workflow_id_to_mapping(id: &str) -> BuddyWorkflowMapping {
    if let Some(meta) = autonomous_workflow_meta(id) {
        return BuddyWorkflowMapping {
            kind: meta.kind,
            icon: meta.icon,
            badge: Some(meta.badge),
        };
    }

    match id {
        "buddy_humor" => BuddyWorkflowMapping {
            kind: "system",
            icon: "🎭",
            badge: Some("Humor"),
        },
        "commit_message" => BuddyWorkflowMapping {
            kind: "workflow",
            icon: "🔄",
            badge: Some("Commit Msg"),
        },
        "follow_up" => BuddyWorkflowMapping {
            kind: "workflow",
            icon: "💡",
            badge: Some("Follow-up"),
        },
        "compress_trajectory" => BuddyWorkflowMapping {
            kind: "system",
            icon: "🤖",
            badge: Some("Compress"),
        },
        "memo_extraction" => BuddyWorkflowMapping {
            kind: "system",
            icon: "🧠",
            badge: Some("Memo"),
        },
        "kg_enrich" | "kg_deprecate" => BuddyWorkflowMapping {
            kind: "system",
            icon: "📚",
            badge: Some("Knowledge"),
        },
        _ => BuddyWorkflowMapping {
            kind: "workflow",
            icon: "🔄",
            badge: None,
        },
    }
}

fn buddy_meta<'a>(val: &'a serde_json::Value) -> Option<&'a serde_json::Value> {
    let meta = val.get("buddy_meta")?;
    meta.get("is_buddy_chat")?.as_bool()?.then_some(meta)
}

fn buddy_meta_workflow_id(meta: &serde_json::Value) -> Option<&str> {
    meta.get("workflow_id").and_then(|v| v.as_str())
}

fn conversation_kind(val: &serde_json::Value) -> String {
    buddy_meta(val)
        .and_then(|meta| meta.get("buddy_chat_kind"))
        .and_then(|v| v.as_str())
        .or_else(|| val.get("kind").and_then(|v| v.as_str()))
        .unwrap_or("chat")
        .to_string()
}

fn conversation_badge(val: &serde_json::Value) -> Option<String> {
    if let Some(meta) = buddy_meta(val) {
        return buddy_meta_workflow_id(meta)
            .and_then(|workflow_id| workflow_id_to_mapping(workflow_id).badge)
            .map(ToString::to_string);
    }

    val.get("badge")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

fn conversation_icon(val: &serde_json::Value, kind: &str) -> String {
    if let Some(icon) = buddy_meta(val)
        .and_then(buddy_meta_workflow_id)
        .map(|workflow_id| workflow_id_to_mapping(workflow_id).icon.to_string())
    {
        return icon;
    }

    match kind {
        "setup" => "⚙️".to_string(),
        "analysis" => "🔍".to_string(),
        "system" => "🤖".to_string(),
        _ => "💬".to_string(),
    }
}

pub async fn list_all_buddy_conversations(
    project_root: &Path,
    kind_filter: Option<Vec<String>>,
) -> Vec<BuddyConversationEntry> {
    let mut entries = Vec::new();

    let conv_dir = project_root.join(".refact/buddy/chats/conversations");
    if let Ok(mut rd) = fs::read_dir(&conv_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if !path.extension().map(|e| e == "json").unwrap_or(false) {
                continue;
            }
            let content = match fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(_) => continue,
            };
            let val = match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(v) => v,
                Err(_) => {
                    warn!("buddy: skipping malformed conversation file: {:?}", path);
                    continue;
                }
            };
            let id = val
                .get("chat_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                warn!("buddy: conversation file missing chat_id: {:?}", path);
                continue;
            }
            let kind = conversation_kind(&val);
            let title = val
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Untitled")
                .to_string();
            let created = val
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let updated = val
                .get("last_message_at")
                .and_then(|v| v.as_str())
                .unwrap_or(&created)
                .to_string();
            let msgs = val
                .get("messages")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u32)
                .unwrap_or(0);
            let badge = conversation_badge(&val);
            let icon = conversation_icon(&val, &kind);
            entries.push(BuddyConversationEntry {
                id,
                kind,
                title,
                created_at: created,
                updated_at: updated,
                status: "active".to_string(),
                message_count: msgs,
                icon,
                badge,
            });
        }
    }

    let wf_dir = project_root.join(".refact/buddy/chats/workflows");
    if let Ok(mut rd) = fs::read_dir(&wf_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if !path.extension().map(|e| e == "json").unwrap_or(false) {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let content = match fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(_) => continue,
            };
            let val = match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(v) => v,
                Err(_) => {
                    warn!("buddy: skipping malformed workflow file: {:?}", path);
                    continue;
                }
            };
            let mapping = workflow_id_to_mapping(&stem);
            let entry_count = val
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u32)
                .unwrap_or(0);
            let last_ts = val
                .get("entries")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.last())
                .and_then(|e| e.get("timestamp"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            entries.push(BuddyConversationEntry {
                id: stem.clone(),
                kind: mapping.kind.to_string(),
                title: format!(
                    "{}{}",
                    stem.replace('_', " "),
                    mapping
                        .badge
                        .map(|b| format!(" ({})", b))
                        .unwrap_or_default()
                ),
                created_at: last_ts.clone(),
                updated_at: last_ts,
                status: "completed".to_string(),
                message_count: entry_count,
                icon: mapping.icon.to_string(),
                badge: mapping.badge.map(|s| s.to_string()),
            });
        }
    }

    if let Some(filter) = &kind_filter {
        entries.retain(|e| filter.iter().any(|f| f == &e.kind));
    }

    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::autonomous_workflows::AUTONOMOUS_BUDDY_WORKFLOWS;

    #[test]
    fn autonomous_workflow_ids_have_system_mappings() {
        for meta in AUTONOMOUS_BUDDY_WORKFLOWS {
            let mapping = workflow_id_to_mapping(meta.id);
            assert_eq!(mapping.kind, "system");
            assert_eq!(mapping.icon, meta.icon);
            assert_eq!(mapping.badge, Some(meta.badge));
            assert_ne!(mapping.icon, "🔄");
            assert!(mapping.badge.is_some());
        }
    }

    #[test]
    fn unknown_workflow_mapping_remains_workflow_fallback() {
        let mapping = workflow_id_to_mapping("custom_workflow");

        assert_eq!(mapping.kind, "workflow");
        assert_eq!(mapping.icon, "🔄");
        assert_eq!(mapping.badge, None);
    }

    #[tokio::test]
    async fn buddy_meta_overrides_top_level_chat_kind_for_saved_conversations() {
        let dir = tempfile::tempdir().unwrap();
        let conv_dir = dir.path().join(".refact/buddy/chats/conversations");
        tokio::fs::create_dir_all(&conv_dir).await.unwrap();
        tokio::fs::write(
            conv_dir.join("chat-a.json"),
            serde_json::json!({
                "chat_id": "chat-a",
                "kind": "chat",
                "title": "Security report",
                "created_at": "2026-01-01T00:00:00Z",
                "last_message_at": "2026-01-01T00:00:01Z",
                "messages": [{"role": "user", "content": "hi"}],
                "buddy_meta": {
                    "is_buddy_chat": true,
                    "buddy_chat_kind": "system",
                    "workflow_id": "buddy_security_whisperer"
                }
            })
            .to_string(),
        )
        .await
        .unwrap();

        let entries = list_all_buddy_conversations(dir.path(), None).await;
        let entry = entries.iter().find(|entry| entry.id == "chat-a").unwrap();

        assert_eq!(entry.kind, "system");
        assert_eq!(entry.icon, "🛡️");
        assert_eq!(entry.badge.as_deref(), Some("Security"));
    }
}
