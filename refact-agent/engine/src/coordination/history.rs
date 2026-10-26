use std::path::Path;

use crate::app_state::AppState;
use crate::chat::trajectory_index::{TrajectoryIndexEntry, TrajectoryIndexSource};
use refact_work_core::WorkKey;
use serde::{Deserialize, Serialize};

use super::roots::{self, ProjectRoots};
use super::types::{Disclosure, Freshness, WorkKindTag, WorkMetadata, WorkRow, WorkStateTag};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkIndexMetadata {
    pub records: Vec<refact_privacy::FileRecord>,
    pub provenance_complete: bool,
    pub source_revision: u64,
    pub measured_tokens: Option<u64>,
}

impl WorkIndexMetadata {
    pub fn from_values(messages: &[serde_json::Value], source_revision: u64) -> Self {
        let settings = crate::file_activity::settings();
        let mut provenance = super::provenance::WorkProvenance::default();
        let mut measured_tokens: Option<u64> = None;
        for value in messages {
            let Ok(message) =
                serde_json::from_value::<crate::call_validation::ChatMessage>(value.clone())
            else {
                return Self {
                    records: vec![],
                    provenance_complete: false,
                    source_revision,
                    measured_tokens: None,
                };
            };
            provenance.observe(&message, messages.len(), &settings);
            if let Some(usage) = message.usage {
                measured_tokens = Some(
                    measured_tokens
                        .unwrap_or(0)
                        .saturating_add(usage.total_tokens as u64),
                );
            }
        }
        let (records, provenance_complete) = provenance.snapshot(messages.len(), &settings);
        Self {
            records,
            provenance_complete,
            source_revision,
            measured_tokens,
        }
    }

    #[cfg(test)]
    pub fn from_messages(
        messages: &[crate::call_validation::ChatMessage],
        source_revision: u64,
    ) -> Self {
        let settings = crate::file_activity::settings();
        let provenance = super::provenance::WorkProvenance::from_messages(messages, &settings);
        let (records, provenance_complete) = provenance.snapshot(messages.len(), &settings);
        Self {
            records,
            provenance_complete,
            source_revision,
            measured_tokens: messages
                .iter()
                .filter_map(|message| message.usage.as_ref())
                .map(|usage| usage.total_tokens as u64)
                .reduce(u64::saturating_add),
        }
    }
}

fn timestamp(value: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp_millis().max(0) as u64)
}

pub(super) fn row_from_entry(entry: &TrajectoryIndexEntry, roots: &ProjectRoots) -> WorkRow {
    let (kind, task_id, card_id) = match &entry.source {
        TrajectoryIndexSource::Task {
            task_id,
            role,
            card_id,
            ..
        } => (
            if role == "planner" {
                WorkKindTag::TaskPlanner
            } else {
                WorkKindTag::BoardAgent
            },
            Some(task_id.clone()),
            card_id.clone(),
        ),
        _ => (
            if entry.parent_id.is_some() {
                WorkKindTag::Subagent
            } else {
                WorkKindTag::Chat
            },
            None,
            None,
        ),
    };
    let updated_at_ms = timestamp(&entry.updated_at).unwrap_or(0);
    let mut root = entry.worktree.as_ref().map_or_else(
        || roots.project_root(),
        |meta| {
            roots.get(&meta.id).cloned().unwrap_or_else(|| {
                let mut root = roots::worktree_root_ref(meta);
                root.path = None;
                root
            })
        },
    );
    if root.project != roots.project {
        root.path = None;
    }
    let work = entry.work.as_ref();
    let settings = crate::file_activity::settings();
    let provenance_complete = work.is_some_and(|work| {
        work.provenance_complete
            && work.records.len() <= settings.max_work_provenance_records
            && work.records.iter().fold(0usize, |bytes, record| {
                bytes
                    .saturating_add(record.path.len())
                    .saturating_add(record.zone.len())
            }) <= settings.max_work_provenance_bytes
    });
    WorkRow {
        key: WorkKey::trajectory(&entry.id),
        metadata: WorkMetadata {
            task_id,
            card_id,
            freshness: if updated_at_ms == 0 {
                Freshness::Unknown
            } else if super::service::now_ms().saturating_sub(updated_at_ms)
                <= settings.recent_window_secs.saturating_mul(1000)
            {
                Freshness::Recent
            } else {
                Freshness::Stale
            },
            last_activity_at: timestamp(&entry.updated_at),
            measured_tokens: work.and_then(|work| work.measured_tokens),
            source_revision: work.map_or(0, |work| work.source_revision),
            ..Default::default()
        },
        kind,
        state: WorkStateTag::Unknown,
        attention: false,
        title: Some(entry.title.clone()).filter(|title| !title.is_empty()),
        summary: None,
        root,
        parent_chat_id: entry.parent_id.clone(),
        root_chat_id: entry.root_chat_id.clone(),
        chat_id: Some(entry.id.clone()),
        model: Some(entry.model.clone()).filter(|model| !model.is_empty()),
        current_tool: None,
        declared_files: vec![],
        observed_files: vec![],
        queue_size: 0,
        step_count: 0,
        open_questions: 0,
        updated_at_ms,
        stale: true,
        disclosure: Disclosure::Full,
        source_records: work
            .filter(|_| provenance_complete)
            .map(|work| work.records.clone())
            .unwrap_or_default(),
        provenance_complete,
    }
}

pub(super) async fn indexed_rows(
    app: &AppState,
    roots: &ProjectRoots,
    degraded: &mut Vec<String>,
) -> Vec<WorkRow> {
    let directory = roots.source_workspace_root.join(".refact/trajectories");
    let directory = match tokio::fs::canonicalize(&directory)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
    {
        Ok(directory) if directory.starts_with(&roots.source_workspace_root) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return vec![],
        _ => {
            degraded.push("historical index unavailable".into());
            return vec![];
        }
    };
    let index = match app
        .gcx
        .trajectory_index_coordinator
        .work_snapshot(&directory)
        .await
    {
        Ok(index) => index,
        Err(_) => {
            degraded.push("historical index unavailable".into());
            return vec![];
        }
    };
    index
        .entries
        .iter()
        .filter(|entry| entry.displayable_chat)
        .filter(|entry| {
            crate::chat::trajectory_index::index_entry_file_name_is_valid(&entry.file_name)
        })
        .filter(|entry| {
            entry.worktree.as_ref().is_none_or(|meta| {
                roots::project_identity(Path::new(&meta.source_workspace_root)) == roots.project
            })
        })
        .map(|entry| row_from_entry(entry, roots))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::coordination::privacy_guard;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn one_malformed_index_message_invalidates_all_provenance() {
        let values = vec![
            serde_json::to_value(crate::call_validation::ChatMessage::default()).unwrap(),
            json!(false),
        ];
        let work = WorkIndexMetadata::from_values(&values, 9);
        assert!(!work.provenance_complete);
        assert!(work.records.is_empty());
        assert_eq!(work.measured_tokens, None);
        assert_eq!(work.source_revision, 9);
    }

    #[tokio::test]
    async fn legacy_or_denied_history_never_discloses_to_a_model() {
        let project = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let roots = roots::load_project_roots(gcx.clone(), project.path().into()).await;
        let mut entry: TrajectoryIndexEntry = serde_json::from_value(json!({
            "id":"legacy", "file_name":"legacy.json", "created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-02T00:00:00Z", "title":"Legacy private title", "model":"trusted/model",
            "mode":"AGENT", "message_count":1
        })).unwrap();
        let model = crate::caps::BaseModelRecord {
            id: "other/model".into(),
            ..Default::default()
        };
        assert!(
            privacy_guard::clear_rows(&gcx, vec![row_from_entry(&entry, &roots)], &model)
                .unwrap()
                .is_empty()
        );
        let mut message = crate::call_validation::ChatMessage::new("tool".into(), "private".into());
        message.extra.insert(
            "privacy".into(),
            json!({"files":[{
                "path":"/guarded/file", "zone":"guarded", "attribution":"declared"
            }]}),
        );
        entry.work = Some(WorkIndexMetadata::from_messages(&[message], 1));
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "guarded".into(),
                patterns: vec!["/guarded/**".into()],
                send_to: vec!["trusted".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        assert!(
            privacy_guard::clear_rows(&gcx, vec![row_from_entry(&entry, &roots)], &model)
                .unwrap()
                .is_empty()
        );
    }
}
