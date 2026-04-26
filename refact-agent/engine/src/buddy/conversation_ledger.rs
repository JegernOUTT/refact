use std::path::Path;
use tokio::fs;
use tracing::warn;

use super::types::BuddyConversationEntry;

fn workflow_id_to_kind(id: &str) -> (&str, &str, Option<&str>) {
    match id {
        "commit_message" => ("workflow", "🔄", Some("Commit Msg")),
        "follow_up" => ("workflow", "💡", Some("Follow-up")),
        "compress_trajectory" => ("system", "🤖", Some("Compress")),
        "memo_extraction" => ("system", "🧠", Some("Memo")),
        "kg_enrich" | "kg_deprecate" => ("system", "📚", Some("Knowledge")),
        _ => ("workflow", "🔄", None),
    }
}

pub async fn list_all_buddy_conversations(
    project_root: &Path,
    kind_filter: Option<Vec<String>>,
) -> Vec<BuddyConversationEntry> {
    let mut entries = Vec::new();

    // Scan conversations
    let conv_dir = project_root.join(".refact/buddy/chats/conversations");
    if let Ok(mut rd) = fs::read_dir(&conv_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(&path).await {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        let id = val.get("chat_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let kind = val.get("kind").and_then(|v| v.as_str()).unwrap_or("chat").to_string();
                        let title = val.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
                        let created = val.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let updated = val.get("last_message_at").and_then(|v| v.as_str()).unwrap_or(&created).to_string();
                        let msgs = val.get("messages").and_then(|v| v.as_array()).map(|a| a.len() as u32).unwrap_or(0);
                        let badge = val.get("badge").and_then(|v| v.as_str()).map(|s| s.to_string());
                        let icon = match kind.as_str() {
                            "setup" => "⚙️".to_string(),
                            "analysis" => "🔍".to_string(),
                            _ => "💬".to_string(),
                        };
                        entries.push(BuddyConversationEntry {
                            id, kind, title, created_at: created, updated_at: updated,
                            status: "active".to_string(), message_count: msgs, icon, badge,
                        });
                    }
                }
            }
        }
    }

    // Scan workflows
    let wf_dir = project_root.join(".refact/buddy/chats/workflows");
    if let Ok(mut rd) = fs::read_dir(&wf_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                if let Ok(content) = fs::read_to_string(&path).await {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        let (kind, icon, badge) = workflow_id_to_kind(&stem);
                        let entry_count = val.get("entries").and_then(|v| v.as_array()).map(|a| a.len() as u32).unwrap_or(0);
                        let last_ts = val.get("entries")
                            .and_then(|v| v.as_array())
                            .and_then(|arr| arr.last())
                            .and_then(|e| e.get("timestamp"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        entries.push(BuddyConversationEntry {
                            id: stem.clone(),
                            kind: kind.to_string(),
                            title: format!("{}{}", stem.replace('_', " "), badge.map(|b| format!(" ({})", b)).unwrap_or_default()),
                            created_at: last_ts.clone(),
                            updated_at: last_ts,
                            status: "completed".to_string(),
                            message_count: entry_count,
                            icon: icon.to_string(),
                            badge: badge.map(|s| s.to_string()),
                        });
                    }
                }
            }
        }
    }

    // Filter
    if let Some(filter) = &kind_filter {
        entries.retain(|e| filter.iter().any(|f| f == &e.kind));
    }

    // Sort by updated_at desc
    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    entries
}
