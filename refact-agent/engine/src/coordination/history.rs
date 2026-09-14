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
    use crate::chat::trajectory_index::entry_from_trajectory_value;
    use crate::coordination::{catalog, privacy_guard};
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
    async fn historical_rows_follow_the_index_without_per_request_body_reads_and_live_rows_win() {
        let project = tempfile::tempdir().unwrap();
        let dir = project.path().join(".refact/trajectories");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let path = dir.join("historical.json");
        let value = json!({
            "id":"historical", "title":"Indexed history", "model":"trusted/model", "mode":"AGENT",
            "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-02T00:00:00Z",
            "messages":[], "version":7, "tool_use":"agent"
        });
        tokio::fs::write(&path, serde_json::to_vec(&value).unwrap())
            .await
            .unwrap();
        let mut entry = entry_from_trajectory_value(&dir, &path, &value, None).unwrap();
        assert!(entry.work.as_ref().unwrap().provenance_complete);
        app.gcx
            .trajectory_index_coordinator
            .upsert(&dir, entry.clone())
            .await
            .unwrap();
        let first = catalog::build_catalog(&app, None).await.unwrap();
        let row = first.find("historical").unwrap();
        assert_ne!(row.metadata.freshness, Freshness::Live);
        assert_eq!(row.state, WorkStateTag::Unknown);
        assert_eq!(row.metadata.source_revision, 7);
        assert_eq!(row.metadata.measured_tokens, None);
        tokio::fs::write(&path, "not valid JSON").await.unwrap();
        for _ in 0..3 {
            let next = catalog::build_catalog(&app, None).await.unwrap();
            assert_eq!(
                next.find("historical").unwrap().title.as_deref(),
                Some("Indexed history")
            );
        }
        entry.title = "Updated index".into();
        app.gcx
            .trajectory_index_coordinator
            .upsert(&dir, entry)
            .await
            .unwrap();
        assert_eq!(
            catalog::build_catalog(&app, None)
                .await
                .unwrap()
                .find("historical")
                .unwrap()
                .title
                .as_deref(),
            Some("Updated index")
        );
        let mut live = crate::chat::types::ChatSession::new("historical".into());
        live.work_project_root = Some(project.path().into());
        live.thread.title = "Live session".into();
        app.chat
            .sessions
            .write()
            .await
            .insert("historical".into(), Arc::new(tokio::sync::Mutex::new(live)));
        let live = catalog::build_catalog(&app, None).await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(
            live.find("historical").unwrap().metadata.freshness,
            Freshness::Live
        );
        app.chat.sessions.write().await.clear();
        app.gcx
            .trajectory_index_coordinator
            .remove(&dir, ["historical".into()].into())
            .await
            .unwrap();
        assert!(catalog::build_catalog(&app, None)
            .await
            .unwrap()
            .find("historical")
            .is_none());
    }

    #[tokio::test]
    async fn thousands_of_indexed_histories_are_projected_without_any_body_read_per_turn() {
        const ROWS: usize = 2000;
        let project = tempfile::tempdir().unwrap();
        let dir = project.path().join(".refact/trajectories");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let mut paths = Vec::with_capacity(ROWS);
        for index in 0..ROWS {
            let id = format!("history-{index:05}");
            let path = dir.join(format!("{id}.json"));
            let value = json!({
                "id": id, "title": format!("Indexed {index}"), "model":"trusted/model", "mode":"AGENT",
                "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-02T00:00:00Z",
                "messages":[], "version":1, "tool_use":"agent"
            });
            std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            paths.push(path);
        }
        let bootstrap = catalog::build_catalog(&app, None).await.unwrap();
        assert_eq!(bootstrap.len(), ROWS);
        for path in &paths {
            std::fs::write(path, "not valid JSON").unwrap();
        }
        let before = app
            .gcx
            .trajectory_index_coordinator
            .listing_counters_for_test();
        for turn in 0..5 {
            let catalog = catalog::build_catalog(&app, None).await.unwrap();
            assert_eq!(catalog.len(), ROWS, "turn {turn}");
            let row = catalog.find("history-01999").unwrap();
            assert_eq!(row.title.as_deref(), Some("Indexed 1999"));
            assert_ne!(row.metadata.freshness, Freshness::Live);
        }
        let after = app
            .gcx
            .trajectory_index_coordinator
            .listing_counters_for_test();
        assert_eq!(after.index_reads, before.index_reads);
        assert_eq!(after.directory_scans, before.directory_scans);
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
