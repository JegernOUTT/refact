use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::file_activity::{begin_writes, WriteIntent, WriteKind};
use crate::global_context::GlobalContext;
use super::tool_apply_patch::ApplyPatchResult;

pub(super) async fn apply(gcx: Arc<GlobalContext>, plan: &ApplyPatchResult) -> Result<(), String> {
    let mut initial: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    for result in &plan.file_results {
        match result.action {
            "add" => {
                initial.entry(result.path.clone()).or_insert(None);
            }
            "rename" => {
                let source = result
                    .chunks
                    .first()
                    .ok_or("patch move has no source")?
                    .file_name
                    .clone();
                initial
                    .entry(source.into())
                    .or_insert_with(|| Some(result.before.clone()));
                initial.entry(result.path.clone()).or_insert(None);
            }
            _ => {
                initial
                    .entry(result.path.clone())
                    .or_insert_with(|| Some(result.before.clone()));
            }
        }
    }
    if initial.is_empty() {
        return Ok(());
    }
    let actor = crate::file_activity::current_actor();
    let paths: Vec<_> = initial
        .iter()
        .map(|(path, before)| {
            (
                path.clone(),
                if before.is_some() {
                    WriteIntent::Contextual
                } else {
                    WriteIntent::Create
                },
            )
        })
        .collect();
    let guards = begin_writes(actor.as_deref(), &paths, "apply_patch").await?;
    let mut guards: BTreeMap<_, _> = paths
        .into_iter()
        .map(|(path, _)| path)
        .zip(guards)
        .collect();
    let mut identities = std::collections::HashSet::new();
    for guard in guards.values() {
        if !identities.insert(guard.key()) {
            return Err(
                "patch paths alias the same physical file; combine edits under one path".into(),
            );
        }
    }
    for (path, expected) in &initial {
        let current = match tokio::fs::read_to_string(path).await {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        if &current != expected {
            return Err(format!(
                "patch preimage changed at {}; reread before retrying",
                path.display()
            ));
        }
        if let Some(current) = current {
            if crate::files_in_workspace::get_file_text_from_memory_or_disk(gcx.clone(), path)
                .await?
                != current
            {
                return Err(format!(
                    "patch buffer differs from disk at {}",
                    path.display()
                ));
            }
        }
    }
    let mut applied = 0usize;
    for result in &plan.file_results {
        let operation: Result<(), String> = async {
            let source = if result.action == "rename" {
                Some(PathBuf::from(
                    &result
                        .chunks
                        .first()
                        .ok_or("patch move has no source")?
                        .file_name,
                ))
            } else {
                None
            };
            if result.action == "delete" {
                tokio::fs::remove_file(&result.path)
                    .await
                    .map_err(|error| error.to_string())?;
                guards
                    .get_mut(&result.path)
                    .ok_or("missing patch guard")?
                    .record_bytes(WriteKind::Deleted, None);
            } else {
                if let Some(parent) = result.path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|error| error.to_string())?;
                }
                let mode = if result.action == "add" || source.is_some() {
                    crate::file_activity::ReplaceMode::CreateNew
                } else {
                    crate::file_activity::ReplaceMode::Replace
                };
                crate::file_activity::write_file_durably(
                    &result.path,
                    result.after.as_bytes(),
                    mode,
                )
                .await?;
                guards
                    .get_mut(&result.path)
                    .ok_or("missing patch guard")?
                    .record_bytes(
                        if result.action == "update" {
                            WriteKind::Modified
                        } else {
                            WriteKind::Created
                        },
                        Some(result.after.as_bytes()),
                    );
                super::undo_history::record_edit(
                    &result.path,
                    (result.action == "update").then_some(result.before.as_str()),
                    Some(&result.after),
                );
                if let Some(source) = source {
                    let metadata = tokio::fs::metadata(&source)
                        .await
                        .map_err(|error| error.to_string())?;
                    tokio::fs::set_permissions(&result.path, metadata.permissions())
                        .await
                        .map_err(|error| error.to_string())?;
                    tokio::fs::remove_file(&source)
                        .await
                        .map_err(|error| error.to_string())?;
                    guards
                        .get_mut(&source)
                        .ok_or("missing patch source guard")?
                        .record_bytes(WriteKind::Deleted, None);
                    crate::files_in_workspace::remove_memory_document_for_path(
                        gcx.clone(),
                        &source,
                    )
                    .await;
                }
            }
            crate::files_in_workspace::remove_memory_document_for_path(gcx.clone(), &result.path)
                .await;
            Ok(())
        }
        .await;
        if let Err(error) = operation {
            let mut changed = Vec::new();
            let mut unchanged = Vec::new();
            let mut unknown = Vec::new();
            for (path, before) in &initial {
                let current = match tokio::fs::read(path).await {
                    Ok(bytes) => Some(bytes),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(_) => {
                        unknown.push(path.display().to_string());
                        continue;
                    }
                };
                if current.as_deref() == before.as_deref().map(str::as_bytes) {
                    unchanged.push(path.display().to_string());
                } else {
                    changed.push(path.display().to_string());
                }
            }
            return Err(format!(
                "patch is not atomic and stopped at {} after {applied} of {} operations: {error}. Changed: [{}]. Unchanged: [{}]. Unknown: [{}]. Lists reflect observed disk content, not successful completion. The failed operation may have changed a file before reporting an error; durability is unconfirmed. Inspect changed and unknown paths before retrying.",
                result.path.display(),
                plan.file_results.len(),
                changed.join(", "),
                unchanged.join(", "),
                unknown.join(", "),
            ));
        }
        applied += 1;
    }
    super::auxiliary::fast_enqueue_for_edit(gcx, &initial.into_keys().collect::<Vec<_>>()).await
}
