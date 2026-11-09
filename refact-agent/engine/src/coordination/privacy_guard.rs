use std::path::PathBuf;
use std::sync::Arc;

use refact_privacy::{FileRecord, PrivacyAuditError, PrivacyAudited};
use crate::caps::BaseModelRecord;
use crate::global_context::GlobalContext;
use crate::privacy::destinations::DestinationExt;
use crate::privacy::records::BatchDeclaredRecords;
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
    let policy = crate::privacy::classification_cache::policy_fingerprint(gcx)
        .map_err(|_| super::error::WorkError::policy_unavailable())?;
    Ok(format!("{}:{}", model.id, policy))
}

pub fn clear_rows(
    gcx: &Arc<GlobalContext>,
    rows: Vec<WorkRow>,
    model: &BaseModelRecord,
) -> Result<Vec<WorkRow>, super::error::WorkError> {
    let before = policy_key(gcx, model)?;
    let mut batch = BatchDeclaredRecords::new(gcx)
        .map_err(|_| super::error::WorkError::policy_unavailable())?;
    if model.id.trim().is_empty() {
        return Err(super::error::WorkError::policy_unavailable());
    }
    let visible = clear_rows_with_batch(gcx, rows, model, &mut batch);
    batch
        .finish(gcx)
        .map_err(|_| super::error::WorkError::policy_unavailable())?;
    if policy_key(gcx, model)? != before {
        return Err(super::error::WorkError::policy_unavailable());
    }
    Ok(visible)
}

fn clear_rows_with_batch(
    gcx: &Arc<GlobalContext>,
    rows: Vec<WorkRow>,
    model: &BaseModelRecord,
    batch: &mut BatchDeclaredRecords,
) -> Vec<WorkRow> {
    let destination = refact_privacy::Destination::from_model_record(model);
    rows.into_iter()
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
            let mut records = batch.classify_declared_paths(gcx, paths);
            records.extend(row.source_records.iter().cloned());
            let audit = WorkAudit {
                records: records.iter().cloned().enumerate().collect(),
            };
            refact_privacy::clear(audit, &destination, &batch.compiled).ok()?;
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
        .collect()
}

pub fn clear_roots(
    gcx: &Arc<GlobalContext>,
    roots: Vec<super::types::WorkRootRef>,
    model: &BaseModelRecord,
) -> Result<Vec<super::types::WorkRootRef>, super::error::WorkError> {
    let before = policy_key(gcx, model)?;
    let mut batch = BatchDeclaredRecords::new(gcx)
        .map_err(|_| super::error::WorkError::policy_unavailable())?;
    let roots = clear_roots_with_batch(gcx, roots, model, &mut batch);
    batch
        .finish(gcx)
        .map_err(|_| super::error::WorkError::policy_unavailable())?;
    if policy_key(gcx, model)? != before {
        return Err(super::error::WorkError::policy_unavailable());
    }
    Ok(roots)
}

fn clear_roots_with_batch(
    gcx: &Arc<GlobalContext>,
    roots: Vec<super::types::WorkRootRef>,
    model: &BaseModelRecord,
    batch: &mut BatchDeclaredRecords,
) -> Vec<super::types::WorkRootRef> {
    let destination = refact_privacy::Destination::from_model_record(model);
    roots
        .into_iter()
        .filter(|root| {
            let paths = root
                .path
                .iter()
                .chain(root.source_path.iter())
                .map(PathBuf::from);
            let records = batch.classify_declared_paths(gcx, paths);
            refact_privacy::clear(
                WorkAudit {
                    records: records.into_iter().enumerate().collect(),
                },
                &destination,
                &batch.compiled,
            )
            .is_ok()
        })
        .collect()
}

pub(super) struct ScopedClearance<'a> {
    gcx: &'a Arc<GlobalContext>,
    model: &'a BaseModelRecord,
    #[cfg(not(windows))]
    batch: BatchDeclaredRecords,
}

impl ScopedClearance<'_> {
    pub(super) fn clear_rows(
        &mut self,
        rows: Vec<WorkRow>,
    ) -> Result<Vec<WorkRow>, super::error::WorkError> {
        #[cfg(windows)]
        {
            clear_rows(self.gcx, rows, self.model)
        }
        #[cfg(not(windows))]
        {
            Ok(clear_rows_with_batch(
                self.gcx,
                rows,
                self.model,
                &mut self.batch,
            ))
        }
    }

    pub(super) fn clear_roots(
        &mut self,
        roots: Vec<super::types::WorkRootRef>,
    ) -> Result<Vec<super::types::WorkRootRef>, super::error::WorkError> {
        #[cfg(windows)]
        {
            clear_roots(self.gcx, roots, self.model)
        }
        #[cfg(not(windows))]
        {
            Ok(clear_roots_with_batch(
                self.gcx,
                roots,
                self.model,
                &mut self.batch,
            ))
        }
    }
}

pub(super) fn with_clearance<T>(
    gcx: &Arc<GlobalContext>,
    model: &BaseModelRecord,
    work: impl FnOnce(&mut ScopedClearance<'_>) -> Result<T, super::error::WorkError>,
) -> Result<T, super::error::WorkError> {
    #[cfg(windows)]
    {
        work(&mut ScopedClearance {
            gcx,
            model,
        })
    }
    #[cfg(not(windows))]
    {
        let before = policy_key(gcx, model)?;
        let batch = BatchDeclaredRecords::new(gcx)
            .map_err(|_| super::error::WorkError::policy_unavailable())?;
        let mut clearance = ScopedClearance {
            gcx,
            model,
            batch,
        };
        let result = work(&mut clearance);
        let finished = clearance
            .batch
            .finish(gcx)
            .map_err(|_| super::error::WorkError::policy_unavailable());
        finished?;
        if policy_key(gcx, model)? != before {
            return Err(super::error::WorkError::policy_unavailable());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::destinations::clear_for_model;
    use crate::coordination::types::{
        Disclosure, RootPresence, WorkKindTag, WorkRootKind, WorkRootRef, WorkStateTag,
    };
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
                presence: RootPresence::Unknown,
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
        assert!(rows[0].source_records.iter().any(|r| r.path
            == refact_core::chat_types::normalize_file_name(path.to_string_lossy().into_owned())
            && r.zone == "guarded"));
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

    #[cfg(not(windows))]
    #[tokio::test]
    async fn scoped_clearance_revokes_equal_policy_identity_swap_and_load_error() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let model = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        let error = with_clearance(&gcx, &model, |clearance| {
            assert_eq!(clearance.clear_rows(vec![row()])?.len(), 1);
            let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
            gcx.privacy_policy_load.write().unwrap().policy = Arc::new((*policy).clone());
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "policy_unavailable");
        let error = with_clearance(&gcx, &model, |clearance| {
            assert!(clearance.clear_roots(Vec::new())?.is_empty());
            gcx.privacy_policy_load.write().unwrap().error = Some("fixture load failure".into());
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "policy_unavailable");
        assert_eq!(
            with_clearance(&gcx, &model, |_| Ok(())).unwrap_err().code,
            "policy_unavailable"
        );
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn scoped_clearance_finish_failure_precedes_closure_error() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let model = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        let stable = with_clearance(&gcx, &model, |_| {
            Err::<(), _>(super::super::error::WorkError::cursor_expired())
        })
        .unwrap_err();
        assert_eq!(
            stable.code,
            super::super::error::WorkError::cursor_expired().code
        );
        for load_error in [false, true] {
            let error = with_clearance(&gcx, &model, |clearance| {
                clearance.clear_rows(Vec::new())?;
                if load_error {
                    gcx.privacy_policy_load.write().unwrap().error =
                        Some("fixture load failure".into());
                } else {
                    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
                    gcx.privacy_policy_load.write().unwrap().policy = Arc::new((*policy).clone());
                }
                Err::<(), _>(super::super::error::WorkError::cursor_expired())
            })
            .unwrap_err();
            assert_eq!(error.code, "policy_unavailable");
        }
    }

    #[tokio::test]
    async fn scoped_clearance_matches_wrappers_and_preserves_inherited_denial() {
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
        let mut inherited = row();
        inherited.source_records.push(FileRecord {
            path: "/secret/a".into(),
            zone: "guarded".into(),
            attribution: Attribution::Declared,
        });
        let mut root = row().root;
        root.path = Some("/secret/a".into());
        for id in ["trusted/model", "other/model"] {
            let model = BaseModelRecord {
                id: id.into(),
                ..Default::default()
            };
            let expected_rows = clear_rows(&gcx, vec![inherited.clone()], &model).unwrap();
            let expected_roots = clear_roots(&gcx, vec![root.clone()], &model).unwrap();
            let (rows, roots) = with_clearance(&gcx, &model, |clearance| {
                Ok((
                    clearance.clear_rows(vec![inherited.clone()])?,
                    clearance.clear_roots(vec![root.clone()])?,
                ))
            })
            .unwrap();
            assert_eq!(
                serde_json::to_value(&rows).unwrap(),
                serde_json::to_value(&expected_rows).unwrap()
            );
            assert_eq!(
                rows.iter()
                    .map(|row| &row.source_records)
                    .collect::<Vec<_>>(),
                expected_rows
                    .iter()
                    .map(|row| &row.source_records)
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                serde_json::to_value(&roots).unwrap(),
                serde_json::to_value(&expected_roots).unwrap()
            );
            assert_eq!(rows.len(), usize::from(id == "trusted/model"));
            assert_eq!(roots.len(), usize::from(id == "trusted/model"));
        }
        let model = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
            blocked: vec!["[".into()],
            ..Default::default()
        });
        assert_eq!(
            with_clearance(&gcx, &model, |clearance| {
                clearance.clear_rows(Vec::new())?;
                clearance.clear_roots(Vec::new())?;
                Ok(())
            })
            .unwrap_err()
            .code,
            "policy_unavailable"
        );
    }
}
