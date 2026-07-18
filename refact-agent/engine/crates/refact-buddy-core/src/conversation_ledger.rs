use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tracing::warn;

use crate::autonomous_workflows::{
    autonomous_workflow_meta, BUDDY_ONBOARDING_WORKFLOW_ID, BUDDY_REFACTOR_HUNTER_WORKFLOW_ID,
    BUDDY_SKILL_AUTHOR_WORKFLOW_ID, BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID,
    ERROR_DETECTIVE_WORKFLOW_ID,
};

use crate::types::BuddyConversationEntry;

const MAX_BUDDY_LEDGER_JSON_BYTES: u64 = 1_024 * 1_024;

const TRAJECTORY_INDEX_FILE_NAME: &str = "index.json";

fn is_ledger_json_candidate(path: &Path) -> bool {
    path.extension().map(|e| e == "json").unwrap_or(false)
        && !path
            .file_name()
            .is_some_and(|name| name == TRAJECTORY_INDEX_FILE_NAME)
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LedgerDiagnostics {
    pub invalid_json: u32,
    pub missing_id: u32,
    pub repaired_id_alias: u32,
    pub empty_conversation: u32,
    pub quarantined: u32,
}

impl LedgerDiagnostics {
    pub fn has_corruption(&self) -> bool {
        self.invalid_json > 0 || self.missing_id > 0 || self.quarantined > 0
    }
}

fn quarantine_target(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("ledger.json")
        .to_string();
    name.push_str(".corrupt");
    path.with_file_name(name)
}

async fn quarantine_ledger_file(path: &Path, reason: &str, diagnostics: &mut LedgerDiagnostics) {
    let target = quarantine_target(path);
    match fs::rename(path, &target).await {
        Ok(()) => {
            diagnostics.quarantined = diagnostics.quarantined.saturating_add(1);
            warn!(
                "buddy: quarantined {} ledger file {:?} -> {:?}",
                reason, path, target
            );
        }
        Err(err) => {
            warn!(
                "buddy: failed to quarantine {} ledger file {:?}: {}",
                reason, path, err
            );
        }
    }
}

enum LedgerReadOutcome {
    Value(serde_json::Value),
    InvalidJson,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuddyWorkflowMapping {
    pub kind: &'static str,
    pub icon: &'static str,
    pub badge: Option<&'static str>,
}

pub fn workflow_id_to_mapping(id: &str) -> BuddyWorkflowMapping {
    if id == "buddy_error_detective" {
        return workflow_id_to_mapping(ERROR_DETECTIVE_WORKFLOW_ID);
    }

    match id {
        BUDDY_ONBOARDING_WORKFLOW_ID => {
            return BuddyWorkflowMapping {
                kind: "system",
                icon: "🧭",
                badge: Some("Onboarding"),
            };
        }
        BUDDY_REFACTOR_HUNTER_WORKFLOW_ID => {
            return BuddyWorkflowMapping {
                kind: "system",
                icon: "🛠️",
                badge: Some("Refactor"),
            };
        }
        BUDDY_SKILL_AUTHOR_WORKFLOW_ID => {
            return BuddyWorkflowMapping {
                kind: "system",
                icon: "✍️",
                badge: Some("Skills"),
            };
        }
        BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID => {
            return BuddyWorkflowMapping {
                kind: "system",
                icon: "🧪",
                badge: Some("Coverage"),
            };
        }
        _ => {}
    }

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

async fn read_buddy_ledger_json(path: &Path, label: &str) -> LedgerReadOutcome {
    let metadata = match fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(err) => {
            warn!("buddy: could not stat {} file {:?}: {}", label, path, err);
            return LedgerReadOutcome::Skipped;
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return LedgerReadOutcome::Skipped;
    }
    if metadata.len() > MAX_BUDDY_LEDGER_JSON_BYTES {
        warn!(
            "buddy: skipping oversized {} file {:?}: {} bytes",
            label,
            path,
            metadata.len()
        );
        return LedgerReadOutcome::Skipped;
    }
    let content = match fs::read_to_string(path).await {
        Ok(content) => content,
        Err(err) => {
            warn!("buddy: could not read {} file {:?}: {}", label, path, err);
            return LedgerReadOutcome::Skipped;
        }
    };
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => LedgerReadOutcome::Value(value),
        Err(err) => {
            warn!(
                "buddy: skipping malformed {} file {:?}: {}",
                label, path, err
            );
            LedgerReadOutcome::InvalidJson
        }
    }
}

fn non_empty_string_field(val: &serde_json::Value, key: &str) -> Option<String> {
    let trimmed = val.get(key)?.as_str()?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn conversation_id(val: &serde_json::Value) -> Option<(String, bool)> {
    if let Some(chat_id) = non_empty_string_field(val, "chat_id") {
        return Some((chat_id, false));
    }
    non_empty_string_field(val, "id").map(|id| (id, true))
}

async fn repair_conversation_id_alias(path: &Path, val: &mut serde_json::Value, id: &str) {
    let Some(map) = val.as_object_mut() else {
        return;
    };
    let mut changed = false;
    for key in ["id", "chat_id"] {
        let needs_repair = map
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim() != id)
            .unwrap_or(true);
        if needs_repair {
            map.insert(key.to_string(), serde_json::Value::String(id.to_string()));
            changed = true;
        }
    }
    if !changed {
        return;
    }
    let Ok(content) = serde_json::to_string_pretty(val) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if let Err(err) = fs::write(&tmp, content).await {
        warn!(
            "buddy: failed to write repaired conversation id alias {:?}: {}",
            tmp, err
        );
        return;
    }
    if let Err(err) = fs::rename(&tmp, path).await {
        let _ = fs::remove_file(&tmp).await;
        warn!(
            "buddy: failed to repair conversation id alias {:?}: {}",
            path, err
        );
    }
}

pub async fn list_all_buddy_conversations(
    project_root: &Path,
    kind_filter: Option<Vec<String>>,
) -> Vec<BuddyConversationEntry> {
    list_all_buddy_conversations_with_diagnostics(project_root, kind_filter)
        .await
        .0
}

pub async fn list_all_buddy_conversations_with_diagnostics(
    project_root: &Path,
    kind_filter: Option<Vec<String>>,
) -> (Vec<BuddyConversationEntry>, LedgerDiagnostics) {
    let mut entries = Vec::new();
    let mut diagnostics = LedgerDiagnostics::default();

    let conv_dir = project_root.join(".refact/buddy/chats/conversations");
    if let Ok(mut rd) = fs::read_dir(&conv_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if !is_ledger_json_candidate(&path) {
                continue;
            }
            let mut val = match read_buddy_ledger_json(&path, "conversation").await {
                LedgerReadOutcome::Value(val) => val,
                LedgerReadOutcome::InvalidJson => {
                    diagnostics.invalid_json = diagnostics.invalid_json.saturating_add(1);
                    quarantine_ledger_file(&path, "malformed conversation", &mut diagnostics).await;
                    continue;
                }
                LedgerReadOutcome::Skipped => continue,
            };
            let Some((id, used_alias)) = conversation_id(&val) else {
                diagnostics.missing_id = diagnostics.missing_id.saturating_add(1);
                quarantine_ledger_file(&path, "id-less conversation", &mut diagnostics).await;
                continue;
            };
            if used_alias {
                diagnostics.repaired_id_alias = diagnostics.repaired_id_alias.saturating_add(1);
            }
            repair_conversation_id_alias(&path, &mut val, &id).await;
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
            if msgs == 0 {
                diagnostics.empty_conversation = diagnostics.empty_conversation.saturating_add(1);
            }
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
            if !is_ledger_json_candidate(&path) {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let val = match read_buddy_ledger_json(&path, "workflow").await {
                LedgerReadOutcome::Value(val) => val,
                LedgerReadOutcome::InvalidJson => {
                    diagnostics.invalid_json = diagnostics.invalid_json.saturating_add(1);
                    quarantine_ledger_file(&path, "malformed workflow", &mut diagnostics).await;
                    continue;
                }
                LedgerReadOutcome::Skipped => continue,
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

    if diagnostics != LedgerDiagnostics::default() {
        warn!(
            "buddy: ledger diagnostics invalid_json={} missing_id={} repaired_id_alias={} empty_conversation={} quarantined={}",
            diagnostics.invalid_json,
            diagnostics.missing_id,
            diagnostics.repaired_id_alias,
            diagnostics.empty_conversation,
            diagnostics.quarantined
        );
    }

    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    (entries, diagnostics)
}

fn conversation_entry_from_value(val: &serde_json::Value, id: String) -> BuddyConversationEntry {
    let kind = conversation_kind(val);
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
    let message_count = val
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);
    let badge = conversation_badge(val);
    let icon = conversation_icon(val, &kind);
    BuddyConversationEntry {
        id,
        kind,
        title,
        created_at: created,
        updated_at: updated,
        status: "active".to_string(),
        message_count,
        icon,
        badge,
    }
}

fn workflow_entry_from_value(stem: String, val: &serde_json::Value) -> BuddyConversationEntry {
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
    BuddyConversationEntry {
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
    }
}

pub async fn list_recent_buddy_conversations(
    project_root: &Path,
    max_items: usize,
) -> Vec<BuddyConversationEntry> {
    if max_items == 0 {
        return Vec::new();
    }
    let conv_dir = project_root.join(".refact/buddy/chats/conversations");
    let wf_dir = project_root.join(".refact/buddy/chats/workflows");

    let mut candidates: Vec<(SystemTime, PathBuf, bool)> = Vec::new();
    for (dir, is_workflow) in [(conv_dir, false), (wf_dir, true)] {
        let Ok(mut rd) = fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if !is_ledger_json_candidate(&path) {
                continue;
            }
            let mtime = match entry.metadata().await {
                Ok(meta) => meta.modified().unwrap_or(UNIX_EPOCH),
                Err(_) => UNIX_EPOCH,
            };
            candidates.push((mtime, path, is_workflow));
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates.truncate(max_items.saturating_mul(4).max(max_items));

    let mut entries = Vec::new();
    for (_, path, is_workflow) in candidates {
        let label = if is_workflow {
            "workflow"
        } else {
            "conversation"
        };
        let val = match read_buddy_ledger_json(&path, label).await {
            LedgerReadOutcome::Value(val) => val,
            _ => continue,
        };
        if is_workflow {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            entries.push(workflow_entry_from_value(stem, &val));
        } else if let Some((id, _)) = conversation_id(&val) {
            entries.push(conversation_entry_from_value(&val, id));
        }
    }

    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    entries.truncate(max_items);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autonomous_workflows::AUTONOMOUS_BUDDY_WORKFLOWS;

    async fn write_valid_conversation(dir: &Path, id: &str, updated_at: &str) {
        tokio::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::json!({
                "chat_id": id,
                "kind": "chat",
                "title": id,
                "created_at": "2026-01-01T00:00:00Z",
                "last_message_at": updated_at,
                "messages": [{"role": "user", "content": "hi"}]
            })
            .to_string(),
        )
        .await
        .unwrap();
    }

    async fn write_valid_workflow(dir: &Path, id: &str, timestamp: &str) {
        tokio::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::json!({
                "entries": [{"timestamp": timestamp}]
            })
            .to_string(),
        )
        .await
        .unwrap();
    }

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

    #[tokio::test]
    async fn ledger_listing_skips_huge_json_files_and_keeps_valid_entries() {
        let dir = tempfile::tempdir().unwrap();
        let conv_dir = dir.path().join(".refact/buddy/chats/conversations");
        let wf_dir = dir.path().join(".refact/buddy/chats/workflows");
        tokio::fs::create_dir_all(&conv_dir).await.unwrap();
        tokio::fs::create_dir_all(&wf_dir).await.unwrap();
        write_valid_conversation(&conv_dir, "chat-valid", "2026-01-01T00:00:01Z").await;
        write_valid_workflow(&wf_dir, "buddy_humor", "2026-01-01T00:00:02Z").await;
        tokio::fs::write(
            conv_dir.join("huge.json"),
            format!(
                "{{\"chat_id\":\"huge\",\"pad\":\"{}\"}}",
                "x".repeat(MAX_BUDDY_LEDGER_JSON_BYTES as usize + 1)
            ),
        )
        .await
        .unwrap();
        tokio::fs::write(
            wf_dir.join("huge_workflow.json"),
            format!(
                "{{\"entries\":[],\"pad\":\"{}\"}}",
                "x".repeat(MAX_BUDDY_LEDGER_JSON_BYTES as usize + 1)
            ),
        )
        .await
        .unwrap();

        let entries = list_all_buddy_conversations(dir.path(), None).await;

        assert!(entries.iter().any(|entry| entry.id == "chat-valid"));
        assert!(entries.iter().any(|entry| entry.id == "buddy_humor"));
        assert!(!entries.iter().any(|entry| entry.id == "huge"));
        assert!(!entries.iter().any(|entry| entry.id == "huge_workflow"));
    }

    #[tokio::test]
    async fn trajectory_index_json_is_never_listed_or_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        let conv_dir = dir.path().join(".refact/buddy/chats/conversations");
        let wf_dir = dir.path().join(".refact/buddy/chats/workflows");
        tokio::fs::create_dir_all(&conv_dir).await.unwrap();
        tokio::fs::create_dir_all(&wf_dir).await.unwrap();
        write_valid_conversation(&conv_dir, "chat-valid", "2026-01-01T00:00:01Z").await;
        let conv_index_path = conv_dir.join("index.json");
        tokio::fs::write(
            &conv_index_path,
            serde_json::json!({
                "schema_version": 1,
                "updated_at": "2026-01-01T00:00:00Z",
                "entries": []
            })
            .to_string(),
        )
        .await
        .unwrap();
        let wf_index_path = wf_dir.join("index.json");
        tokio::fs::write(&wf_index_path, "{\"entries\":[]}")
            .await
            .unwrap();

        let (entries, diagnostics) =
            list_all_buddy_conversations_with_diagnostics(dir.path(), None).await;

        assert!(entries.iter().any(|entry| entry.id == "chat-valid"));
        assert!(!entries.iter().any(|entry| entry.id == "index"));
        assert_eq!(diagnostics, LedgerDiagnostics::default());
        assert!(conv_index_path.exists());
        assert!(wf_index_path.exists());
        assert!(!conv_dir.join("index.json.corrupt").exists());

        let recent = list_recent_buddy_conversations(dir.path(), 10).await;
        assert!(recent.iter().any(|entry| entry.id == "chat-valid"));
        assert!(!recent.iter().any(|entry| entry.id == "index"));
    }

    #[tokio::test]
    async fn corrupt_ledger_files_are_quarantined_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let conv_dir = dir.path().join(".refact/buddy/chats/conversations");
        tokio::fs::create_dir_all(&conv_dir).await.unwrap();
        write_valid_conversation(&conv_dir, "chat-good", "2026-01-01T00:00:01Z").await;
        let malformed_path = conv_dir.join("broken.json");
        tokio::fs::write(&malformed_path, "{not json")
            .await
            .unwrap();
        let missing_id_path = conv_dir.join("no_id.json");
        tokio::fs::write(
            &missing_id_path,
            serde_json::json!({
                "title": "orphan",
                "created_at": "2026-01-01T00:00:00Z",
                "messages": [{"role": "user", "content": "hi"}]
            })
            .to_string(),
        )
        .await
        .unwrap();

        let (entries, diagnostics) =
            list_all_buddy_conversations_with_diagnostics(dir.path(), None).await;

        assert!(entries.iter().any(|entry| entry.id == "chat-good"));
        assert_eq!(diagnostics.invalid_json, 1);
        assert_eq!(diagnostics.missing_id, 1);
        assert_eq!(diagnostics.quarantined, 2);
        assert!(diagnostics.has_corruption());
        assert!(!malformed_path.exists());
        assert!(!missing_id_path.exists());
        assert!(conv_dir.join("broken.json.corrupt").exists());
        assert!(conv_dir.join("no_id.json.corrupt").exists());

        let (second_entries, second_diagnostics) =
            list_all_buddy_conversations_with_diagnostics(dir.path(), None).await;
        assert!(second_entries.iter().any(|entry| entry.id == "chat-good"));
        assert_eq!(second_diagnostics, LedgerDiagnostics::default());
    }

    #[tokio::test]
    async fn malformed_huge_ledger_files_do_not_block_valid_entries() {
        let dir = tempfile::tempdir().unwrap();
        let conv_dir = dir.path().join(".refact/buddy/chats/conversations");
        let wf_dir = dir.path().join(".refact/buddy/chats/workflows");
        tokio::fs::create_dir_all(&conv_dir).await.unwrap();
        tokio::fs::create_dir_all(&wf_dir).await.unwrap();
        write_valid_conversation(&conv_dir, "chat-valid", "2026-01-01T00:00:01Z").await;
        write_valid_workflow(&wf_dir, "follow_up", "2026-01-01T00:00:02Z").await;
        tokio::fs::write(
            conv_dir.join("malformed.json"),
            format!("{{\"chat_id\":\"bad\",{}", "x".repeat(256)),
        )
        .await
        .unwrap();
        tokio::fs::write(
            wf_dir.join("malformed_workflow.json"),
            format!("{{\"entries\":[{}", "x".repeat(256)),
        )
        .await
        .unwrap();
        tokio::fs::write(
            conv_dir.join("malformed_huge.json"),
            format!("{{{}", "x".repeat(MAX_BUDDY_LEDGER_JSON_BYTES as usize + 1)),
        )
        .await
        .unwrap();
        tokio::fs::write(
            wf_dir.join("malformed_huge_workflow.json"),
            format!("{{{}", "x".repeat(MAX_BUDDY_LEDGER_JSON_BYTES as usize + 1)),
        )
        .await
        .unwrap();

        let entries = list_all_buddy_conversations(dir.path(), None).await;

        assert!(entries.iter().any(|entry| entry.id == "chat-valid"));
        assert!(entries.iter().any(|entry| entry.id == "follow_up"));
        assert!(!entries.iter().any(|entry| entry.id == "bad"));
        assert!(!entries.iter().any(|entry| entry.id == "malformed_workflow"));
        assert!(!entries
            .iter()
            .any(|entry| entry.id == "malformed_huge_workflow"));
    }
}
