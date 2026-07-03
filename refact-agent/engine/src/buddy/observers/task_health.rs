use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_yaml::Value as YamlValue;

use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::app_state::AppState;
use crate::tasks::types::{TaskBoard, TaskMeta, TaskStatus};

pub struct TaskHealthObserver;
pub(crate) const MAX_TASK_CLUSTER_ENTRIES: usize = 200;

pub struct TaskHealthEntry {
    pub meta: TaskMeta,
    pub board: TaskBoard,
    /// Most recent session `last_activity` timestamp across all "doing" cards
    /// for this task. `None` means no agent session has ever been active.
    pub last_heartbeat: Option<DateTime<Utc>>,
    /// Files touched by this task (for cluster-duplicate overlap check).
    /// Empty means file metadata is unavailable; the cluster rule is skipped.
    pub touched_files: Vec<String>,
}

fn title_hash(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:x}", h.finish())
}

fn memory_namespace(content: &str) -> Option<String> {
    let frontmatter = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?
        .split_once("\n---")?
        .0;
    match serde_yaml::from_str::<YamlValue>(frontmatter).ok()? {
        YamlValue::Mapping(mapping) => match mapping.get(&YamlValue::String("namespace".into()))? {
            YamlValue::String(value) => Some(value.clone()),
            _ => None,
        },
        _ => None,
    }
}

async fn detect_orphaned_task_memory_facts(
    task_id: &str,
    board: &TaskBoard,
    memories_dir: &std::path::Path,
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let existing_cards: HashSet<&str> = board.cards.iter().map(|card| card.id.as_str()).collect();
    let mut entries = match tokio::fs::read_dir(memories_dir).await {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut facts = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if !metadata.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let Ok(content) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        let Some(namespace) = memory_namespace(&content) else {
            continue;
        };
        let Some(card_id) = namespace
            .strip_prefix("card:")
            .map(|value| value.trim().to_string())
        else {
            continue;
        };
        if card_id.is_empty() || existing_cards.contains(card_id.as_str()) {
            continue;
        }
        facts.push(BuddyFact {
            kind: BuddyFactKind::MemoryOrphan,
            key: format!("task_memory:orphan:{}:{}", task_id, card_id),
            source: "task_health",
            payload: serde_json::json!({
                "task_id": task_id,
                "card_id": card_id,
                "namespace": namespace,
            }),
            seen_at: now,
            confidence: 0.85,
        });
    }
    facts
}

pub fn detect_task_health_facts(entries: &[TaskHealthEntry], now: DateTime<Utc>) -> Vec<BuddyFact> {
    let mut facts = vec![];
    let stuck_threshold = chrono::Duration::hours(4);
    let abandon_threshold = chrono::Duration::days(7);

    for entry in entries {
        let terminal = matches!(
            entry.meta.status,
            TaskStatus::Completed | TaskStatus::Abandoned
        );
        if terminal {
            continue;
        }

        let has_doing_card = entry
            .board
            .cards
            .iter()
            .any(|c| c.column == "doing" && c.assignee.is_some());

        if has_doing_card {
            // TaskStuck: requires a stale heartbeat. If no heartbeat is available,
            // do NOT emit — absence of a session means "never ran", not "stuck".
            if let Some(heartbeat) = entry.last_heartbeat {
                if now.signed_duration_since(heartbeat) >= stuck_threshold {
                    tracing::debug!("task_health: stuck task {}", entry.meta.id);
                    facts.push(BuddyFact {
                        kind: BuddyFactKind::TaskStuck,
                        key: format!("task:stuck:{}", entry.meta.id),
                        source: "task_health",
                        payload: serde_json::json!({
                            "task_id": entry.meta.id,
                            "last_seen_iso": heartbeat.to_rfc3339(),
                        }),
                        seen_at: now,
                        confidence: 0.8,
                    });
                }
            }
        }

        // TaskAbandoned: old task with no active doing agent and no heartbeat ever recorded.
        if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&entry.meta.created_at) {
            let age = now.signed_duration_since(created.with_timezone(&Utc));
            if age >= abandon_threshold && entry.last_heartbeat.is_none() && !has_doing_card {
                tracing::debug!("task_health: abandoned task {}", entry.meta.id);
                facts.push(BuddyFact {
                    kind: BuddyFactKind::TaskAbandoned,
                    key: format!("task:abandoned:{}", entry.meta.id),
                    source: "task_health",
                    payload: serde_json::json!({
                        "task_id": entry.meta.id,
                        "age_days": age.num_days(),
                    }),
                    seen_at: now,
                    confidence: 0.6,
                });
            }
        }
    }

    let active: Vec<&TaskHealthEntry> = entries
        .iter()
        .filter(|e| !matches!(e.meta.status, TaskStatus::Completed | TaskStatus::Abandoned))
        .take(MAX_TASK_CLUSTER_ENTRIES)
        .collect();

    let mut emitted: HashSet<String> = HashSet::new();
    for i in 0..active.len() {
        for j in (i + 1)..active.len() {
            let a = active[i].meta.name.to_lowercase();
            let a = a.trim();
            let b = active[j].meta.name.to_lowercase();
            let b = b.trim();
            if a.is_empty() || b.is_empty() {
                continue;
            }
            let sim = strsim::normalized_levenshtein(a, b);
            if sim > 0.7 {
                // Require at least one shared file when file metadata is available.
                // Without file metadata (empty touched_files), the cluster rule is
                // skipped to avoid false positives from title similarity alone.
                let a_files = &active[i].touched_files;
                let b_files = &active[j].touched_files;
                let overlap_count = if !a_files.is_empty() && !b_files.is_empty() {
                    a_files.iter().filter(|f| b_files.contains(*f)).count()
                } else {
                    0
                };
                if overlap_count == 0 {
                    continue;
                }
                let rep = if a <= b { a } else { b };
                let key = format!("task_cluster:{}", title_hash(rep));
                if emitted.insert(key.clone()) {
                    tracing::debug!(
                        "task_health: cluster {} ~ {}",
                        active[i].meta.id,
                        active[j].meta.id
                    );
                    facts.push(BuddyFact {
                        kind: BuddyFactKind::TaskClusterDuplicate,
                        key,
                        source: "task_health",
                        payload: serde_json::json!({
                            "task_a": active[i].meta.id,
                            "task_b": active[j].meta.id,
                            "overlap_count": overlap_count,
                            "similarity": sim,
                        }),
                        seen_at: now,
                        confidence: 0.9,
                    });
                }
            }
        }
    }

    facts
}

#[async_trait::async_trait]
impl BuddyObserver for TaskHealthObserver {
    fn id(&self) -> &'static str {
        "task_health"
    }

    fn cadence_seconds(&self) -> u64 {
        60
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.task_health
    }

    async fn observe(&self, gcx: AppState, ctx: &ObserverContext) -> Vec<BuddyFact> {
        let tasks = match crate::tasks::storage::list_tasks(gcx.gcx.clone()).await {
            Ok(t) => t,
            Err(_) => return vec![],
        };
        let mut entries = vec![];
        let mut memory_facts = vec![];
        for meta in tasks {
            let board = match crate::tasks::storage::load_board(gcx.gcx.clone(), &meta.id).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            if let Ok(task_dir) =
                crate::tasks::storage::find_task_dir(gcx.gcx.clone(), &meta.id).await
            {
                memory_facts.extend(
                    detect_orphaned_task_memory_facts(
                        &meta.id,
                        &board,
                        &task_dir.join("memories"),
                        ctx.now,
                    )
                    .await,
                );
            }
            let mut latest_heartbeat: Option<chrono::DateTime<Utc>> = None;
            for card in &board.cards {
                if card.column == "doing" {
                    let live = if let Some(chat_id) = &card.agent_chat_id {
                        crate::chat::task_agent_monitor::get_last_agent_heartbeat(
                            gcx.clone(),
                            chat_id,
                        )
                        .await
                    } else {
                        None
                    };
                    let hb = live.or_else(|| {
                        card.last_heartbeat_at
                            .as_deref()
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                            .map(|t| t.with_timezone(&Utc))
                    });
                    if let Some(h) = hb {
                        latest_heartbeat = Some(match latest_heartbeat {
                            Some(t) if t > h => t,
                            _ => h,
                        });
                    }
                }
            }
            let touched_files: Vec<String> = board
                .cards
                .iter()
                .flat_map(|c| c.target_files.iter().cloned())
                .collect();
            entries.push(TaskHealthEntry {
                meta,
                board,
                last_heartbeat: latest_heartbeat,
                touched_files,
            });
        }
        let mut facts = detect_task_health_facts(&entries, ctx.now);
        facts.extend(memory_facts);
        facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detects_orphaned_card_scoped_task_memory() {
        let temp = tempfile::tempdir().unwrap();
        let memories_dir = temp.path().join(".refact/tasks/task-1/memories");
        tokio::fs::create_dir_all(&memories_dir).await.unwrap();
        tokio::fs::write(
            memories_dir.join("orphan.md"),
            "---\ntitle: Orphan\nnamespace: card:T-404\n---\n\nBody",
        )
        .await
        .unwrap();

        let facts = detect_orphaned_task_memory_facts(
            "task-1",
            &TaskBoard::default(),
            &memories_dir,
            Utc::now(),
        )
        .await;

        let fact = facts
            .iter()
            .find(|fact| fact.kind == BuddyFactKind::MemoryOrphan)
            .expect("orphaned memory fact must be emitted");
        assert_eq!(fact.payload["task_id"], serde_json::json!("task-1"));
        assert_eq!(fact.payload["card_id"], serde_json::json!("T-404"));
        assert_eq!(fact.payload["namespace"], serde_json::json!("card:T-404"));
    }
}
