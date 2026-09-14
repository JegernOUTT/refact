use std::path::PathBuf;
use std::sync::Arc;

use refact_privacy::{FileRecord, PrivacyAuditError, PrivacyAudited};
use crate::caps::BaseModelRecord;
use crate::global_context::GlobalContext;
use crate::privacy::destinations::clear_for_model;
use super::types::WorkRow;

struct WorkAudit {
    records: Vec<(usize, FileRecord)>,
}

impl PrivacyAudited for WorkAudit {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        Ok(self.records.clone())
    }
}

pub fn policy_key(
    gcx: &Arc<GlobalContext>,
    model: &BaseModelRecord,
) -> Result<String, super::error::WorkError> {
    if model.id.trim().is_empty() {
        return Err(super::error::WorkError::policy_unavailable());
    }
    let load = gcx
        .privacy_policy_load
        .read()
        .map_err(|_| super::error::WorkError::policy_unavailable())?;
    if load.error.is_some() || load.policy.compile().is_err() {
        return Err(super::error::WorkError::policy_unavailable());
    }
    let policy = serde_json::to_string(load.policy.as_ref())
        .map_err(|_| super::error::WorkError::policy_unavailable())?;
    Ok(format!(
        "{}:{}",
        model.id,
        crate::file_activity::digest_of(&policy)
    ))
}

pub fn clear_rows(
    gcx: &Arc<GlobalContext>,
    rows: Vec<WorkRow>,
    model: &BaseModelRecord,
) -> Result<Vec<WorkRow>, super::error::WorkError> {
    let before = policy_key(gcx, model)?;
    if model.id.trim().is_empty() {
        return Err(super::error::WorkError::policy_unavailable());
    }
    let visible = rows
        .into_iter()
        .filter_map(|mut row| {
            if !row.provenance_complete {
                return None;
            }
            let paths = row.disclosed_paths().into_iter().map(|path| {
                let path = PathBuf::from(path);
                if path.is_absolute() {
                    path
                } else {
                    row.root
                        .path
                        .as_ref()
                        .map_or(path.clone(), |root| PathBuf::from(root).join(path))
                }
            });
            let mut records = crate::privacy::records::declared_file_records(gcx, paths).ok()?;
            records.extend(row.source_records.iter().cloned());
            let audit = WorkAudit {
                records: records.iter().cloned().enumerate().collect(),
            };
            clear_for_model(gcx, audit, model).ok()?;
            // Carry all disclosed path records into future tool/capsule
            // messages, so a later model switch cannot re-send old metadata.
            row.source_records = records;
            row.current_tool = row.current_tool.and_then(|tool| {
                let name = tool.split([':', '(', ' ']).next().unwrap_or("").trim();
                (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                    .then(|| name.to_string())
            });
            Some(row)
        })
        .collect();
    if policy_key(gcx, model)? != before {
        return Err(super::error::WorkError::policy_unavailable());
    }
    Ok(visible)
}

pub fn clear_roots(
    gcx: &Arc<GlobalContext>,
    roots: Vec<super::types::WorkRootRef>,
    model: &BaseModelRecord,
) -> Result<Vec<super::types::WorkRootRef>, super::error::WorkError> {
    let before = policy_key(gcx, model)?;
    let roots = roots
        .into_iter()
        .filter(|root| {
            let paths = root
                .path
                .iter()
                .chain(root.source_path.iter())
                .map(PathBuf::from);
            let Ok(records) = crate::privacy::records::declared_file_records(gcx, paths) else {
                return false;
            };
            clear_for_model(
                gcx,
                WorkAudit {
                    records: records.into_iter().enumerate().collect(),
                },
                model,
            )
            .is_ok()
        })
        .collect();
    if policy_key(gcx, model)? != before {
        return Err(super::error::WorkError::policy_unavailable());
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::types::{Disclosure, WorkKindTag, WorkRootKind, WorkRootRef, WorkStateTag};
    use refact_privacy::{Attribution, PrivacyPolicy, ShellBehavior, Zone};
    use refact_work_core::WorkKey;

    fn row() -> WorkRow {
        WorkRow {
            key: WorkKey::trajectory("chat"),
            metadata: Default::default(),
            kind: WorkKindTag::Chat,
            state: WorkStateTag::Running,
            attention: false,
            title: Some("sensitive title".into()),
            summary: Some("sensitive summary".into()),
            root: WorkRootRef {
                id: "project".into(),
                kind: WorkRootKind::Project,
                path: None,
                branch: None,
                project: "project".into(),
                source_path: None,
            },
            parent_chat_id: None,
            root_chat_id: None,
            chat_id: Some("chat".into()),
            model: None,
            current_tool: Some("shell: token=private".into()),
            declared_files: vec![],
            observed_files: vec![],
            queue_size: 0,
            step_count: 0,
            open_questions: 0,
            updated_at_ms: 1,
            stale: false,
            disclosure: Disclosure::Full,
            source_records: vec![],
            provenance_complete: true,
        }
    }

    #[tokio::test]
    async fn unavailable_provenance_and_destination_reveal_no_row_or_count() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut missing = row();
        missing.provenance_complete = false;
        let model = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        assert!(clear_rows(&gcx, vec![missing], &model).unwrap().is_empty());
        assert_eq!(
            clear_rows(&gcx, vec![row()], &BaseModelRecord::default())
                .unwrap_err()
                .code,
            "policy_unavailable"
        );
    }

    #[tokio::test]
    async fn inherited_source_records_guard_free_text_even_without_paths() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
            zones: vec![Zone {
                name: "guarded".into(),
                patterns: vec!["**/secret/**".into()],
                send_to: vec!["trusted".into()],
                on_shell_read: ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        let mut guarded = row();
        guarded.source_records.push(FileRecord {
            path: "/secret/a".into(),
            zone: "guarded".into(),
            attribution: Attribution::Declared,
        });
        let model = BaseModelRecord {
            id: "other/model".into(),
            ..Default::default()
        };
        let rows = clear_rows(&gcx, vec![guarded], &model).unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn tool_argument_previews_are_not_exposed() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let model = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        let rows = clear_rows(&gcx, vec![row()], &model).unwrap();
        assert_eq!(rows[0].current_tool.as_deref(), Some("shell"));
    }

    #[tokio::test]
    async fn relative_paths_carry_the_same_absolute_provenance_that_was_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = dir.path().join("guarded.rs");
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
            zones: vec![Zone {
                name: "guarded".into(),
                patterns: vec![path.to_string_lossy().into_owned()],
                send_to: vec!["trusted".into()],
                on_shell_read: ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        let mut source = row();
        source.root.path = Some(dir.path().to_string_lossy().into_owned());
        source.declared_files.push("guarded.rs".into());
        let model = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        let rows = clear_rows(&gcx, vec![source.clone()], &model).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0]
            .source_records
            .iter()
            .any(|r| r.path == path.to_string_lossy() && r.zone == "guarded"));
        let other = BaseModelRecord {
            id: "other/model".into(),
            ..Default::default()
        };
        assert!(clear_rows(&gcx, vec![source], &other).unwrap().is_empty());
        let audit = WorkAudit {
            records: rows[0].source_records.iter().cloned().enumerate().collect(),
        };
        assert!(clear_for_model(&gcx, audit, &other).is_err());
    }
}
