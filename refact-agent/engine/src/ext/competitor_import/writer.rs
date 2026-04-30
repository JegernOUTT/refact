use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

use chrono::Utc;

use super::manifest::{
    hash_directory, hash_file, manifest_path_for_scope_root, write_string_atomic, ImportManifest,
    ImportManifestEntry, IMPORTER_VERSION,
};
use super::types::{
    ImportArtifact, ImportCandidate, ImportCandidateSummary, ImportIssue, ImportOutcome,
    ImportStatus, ImportSummary,
};

pub async fn write_candidates(scope_root: &Path, candidates: &[ImportCandidate]) -> ImportSummary {
    let mut summary = ImportSummary::default();
    let manifest_path = manifest_path_for_scope_root(scope_root);
    let mut manifest = match ImportManifest::read_from_path(&manifest_path).await {
        Ok(manifest) => manifest,
        Err(err) => {
            summary.add_issue(ImportIssue {
                competitor: None,
                kind: None,
                scope: None,
                path: Some(manifest_path),
                status: ImportStatus::Error,
                message: format!("failed to read import manifest: {err}"),
            });
            return summary;
        }
    };

    for candidate in candidates {
        summary.record_candidate(candidate);
        match write_candidate(scope_root, &mut manifest, candidate).await {
            CandidateWriteResult::Outcome(outcome) => summary.add_outcome(outcome),
            CandidateWriteResult::Error { outcome, issue } => {
                summary.add_outcome(outcome);
                summary.issues.push(issue.clone());
                summary.errors.push(issue);
            }
        }
    }

    manifest.last_report = Some(summary.clone());
    if let Err(err) = manifest.write_to_path(&manifest_path).await {
        summary.add_issue(ImportIssue {
            competitor: None,
            kind: None,
            scope: None,
            path: Some(manifest_path),
            status: ImportStatus::Error,
            message: format!("failed to write import manifest: {err}"),
        });
    }
    summary
}

enum CandidateWriteResult {
    Outcome(ImportOutcome),
    Error {
        outcome: ImportOutcome,
        issue: ImportIssue,
    },
}

async fn write_candidate(
    scope_root: &Path,
    manifest: &mut ImportManifest,
    candidate: &ImportCandidate,
) -> CandidateWriteResult {
    match try_write_candidate(scope_root, manifest, candidate).await {
        Ok(outcome) => CandidateWriteResult::Outcome(outcome),
        Err(err) => {
            let message = err.to_string();
            CandidateWriteResult::Error {
                outcome: outcome(candidate, ImportStatus::Error, message.clone()),
                issue: issue(candidate, ImportStatus::Error, message),
            }
        }
    }
}

async fn try_write_candidate(
    scope_root: &Path,
    manifest: &mut ImportManifest,
    candidate: &ImportCandidate,
) -> Result<ImportOutcome> {
    let dest_path = resolve_destination_path(scope_root, &candidate.destination_path);
    let dest_meta = match tokio::fs::symlink_metadata(&dest_path).await {
        Ok(meta) => Some(meta),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };

    let manifest_entry = manifest.entry_for_dest(&dest_path).cloned();
    if let Some(entry) = manifest_entry {
        if dest_meta.is_some() && !manifest_entry_matches_candidate(&entry, candidate) {
            return Ok(outcome(
                candidate,
                ImportStatus::Conflict,
                "destination is owned by a different import source".to_string(),
            ));
        }
        if dest_meta.is_some() {
            let current_dest_hash = hash_existing_path(&dest_path)?;
            if current_dest_hash != entry.dest_hash {
                return Ok(outcome(
                    candidate,
                    ImportStatus::UserModified,
                    "destination differs from previous generated hash".to_string(),
                ));
            }
            let source_hash = candidate_source_hash(candidate)?;
            if source_hash == entry.source_hash {
                return Ok(outcome(
                    candidate,
                    ImportStatus::Unchanged,
                    "source and destination are unchanged".to_string(),
                ));
            }
            write_artifact(candidate, &dest_path).await?;
            let dest_hash = hash_existing_path(&dest_path)?;
            manifest.upsert_entry(manifest_entry_from_candidate(
                candidate,
                dest_path,
                source_hash,
                dest_hash,
            ));
            return Ok(outcome(
                candidate,
                ImportStatus::Updated,
                "updated generated destination".to_string(),
            ));
        }
    } else if dest_meta.is_some() {
        return Ok(outcome(
            candidate,
            ImportStatus::Conflict,
            "destination exists without import manifest ownership".to_string(),
        ));
    }

    let source_hash = candidate_source_hash(candidate)?;
    write_artifact(candidate, &dest_path).await?;
    let dest_hash = hash_existing_path(&dest_path)?;
    manifest.upsert_entry(manifest_entry_from_candidate(
        candidate,
        dest_path,
        source_hash,
        dest_hash,
    ));
    Ok(outcome(
        candidate,
        ImportStatus::Created,
        "created generated destination".to_string(),
    ))
}

fn resolve_destination_path(scope_root: &Path, destination_path: &Path) -> PathBuf {
    if destination_path.is_absolute() {
        destination_path.to_path_buf()
    } else {
        scope_root.join(destination_path)
    }
}

fn manifest_entry_matches_candidate(
    entry: &ImportManifestEntry,
    candidate: &ImportCandidate,
) -> bool {
    entry.competitor == candidate.competitor
        && entry.kind == candidate.kind
        && entry.source_path == candidate.source_path
}

fn candidate_source_hash(candidate: &ImportCandidate) -> Result<String> {
    match &candidate.artifact {
        ImportArtifact::FileContent { .. } => hash_file(&candidate.source_path),
        ImportArtifact::DirectoryCopy { source_dir } => hash_directory(source_dir),
    }
}

fn hash_existing_path(path: &Path) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("destination is a symlink: {}", path.display()),
        ));
    }
    if file_type.is_dir() {
        hash_directory(path)
    } else if file_type.is_file() {
        hash_file(path)
    } else {
        Err(Error::new(
            ErrorKind::InvalidData,
            format!("unsupported destination file type: {}", path.display()),
        ))
    }
}

async fn write_artifact(candidate: &ImportCandidate, dest_path: &Path) -> Result<()> {
    match &candidate.artifact {
        ImportArtifact::FileContent { content } => write_string_atomic(dest_path, content).await,
        ImportArtifact::DirectoryCopy { source_dir } => {
            copy_directory_atomically(source_dir, dest_path).await
        }
    }
}

async fn copy_directory_atomically(source_dir: &Path, dest_path: &Path) -> Result<()> {
    let source_metadata = tokio::fs::symlink_metadata(source_dir).await?;
    let source_type = source_metadata.file_type();
    if source_type.is_symlink() || !source_type.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "source directory is not a regular directory: {}",
                source_dir.display()
            ),
        ));
    }

    let parent = dest_path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let dest_name = dest_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("import");
    let staging = parent.join(format!(".{}.{}.tmp", dest_name, uuid::Uuid::new_v4()));
    let copy_result = async {
        copy_directory_contents(source_dir, &staging).await?;
        remove_existing_path(dest_path).await?;
        tokio::fs::rename(&staging, dest_path).await
    }
    .await;
    if copy_result.is_err() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    copy_result
}

async fn copy_directory_contents(source_dir: &Path, staging: &Path) -> Result<()> {
    tokio::fs::create_dir_all(staging).await?;
    for entry in walkdir::WalkDir::new(source_dir)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|err| Error::new(ErrorKind::Other, err.to_string()))?;
        let source_path = entry.path();
        if source_path == source_dir {
            continue;
        }
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            continue;
        }
        let relative_path = source_path
            .strip_prefix(source_dir)
            .map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()))?;
        let target_path = staging.join(relative_path);
        if file_type.is_dir() {
            tokio::fs::create_dir_all(&target_path).await?;
        } else if file_type.is_file() {
            if let Some(parent) = target_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let content = tokio::fs::read(source_path).await?;
            tokio::fs::write(&target_path, content).await?;
        }
    }
    Ok(())
}

async fn remove_existing_path(path: &Path) -> Result<()> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let file_type = metadata.file_type();
    if file_type.is_dir() && !file_type.is_symlink() {
        tokio::fs::remove_dir_all(path).await
    } else {
        tokio::fs::remove_file(path).await
    }
}

fn manifest_entry_from_candidate(
    candidate: &ImportCandidate,
    dest_path: PathBuf,
    source_hash: String,
    dest_hash: String,
) -> ImportManifestEntry {
    ImportManifestEntry {
        competitor: candidate.competitor,
        kind: candidate.kind,
        source_path: candidate.source_path.clone(),
        source_hash,
        dest_path,
        dest_hash,
        importer_version: IMPORTER_VERSION.to_string(),
        last_imported_at: Utc::now(),
        metadata: if candidate.metadata.is_null() {
            None
        } else {
            Some(candidate.metadata.clone())
        },
    }
}

fn outcome(candidate: &ImportCandidate, status: ImportStatus, message: String) -> ImportOutcome {
    ImportOutcome {
        candidate: ImportCandidateSummary::from(candidate),
        status,
        message,
    }
}

fn issue(candidate: &ImportCandidate, status: ImportStatus, message: String) -> ImportIssue {
    ImportIssue {
        competitor: Some(candidate.competitor),
        kind: Some(candidate.kind),
        scope: Some(candidate.scope.clone()),
        path: Some(candidate.destination_path.clone()),
        status,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::manifest::{hash_directory, hash_file, manifest_path_for_scope_root};
    use super::super::types::{Competitor, ImportKind, ImportScope};

    fn file_candidate(source_path: PathBuf, dest_path: PathBuf, content: &str) -> ImportCandidate {
        if let Some(parent) = source_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&source_path, content).unwrap();
        ImportCandidate {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Command,
            scope: ImportScope::Global,
            source_root: source_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(PathBuf::new),
            source_path,
            dest_name: "hello".to_string(),
            destination_path: dest_path,
            artifact: ImportArtifact::FileContent {
                content: content.to_string(),
            },
            metadata: serde_json::json!({"original_name": "hello"}),
        }
    }

    fn directory_candidate(source_dir: PathBuf, dest_path: PathBuf) -> ImportCandidate {
        ImportCandidate {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Skill,
            scope: ImportScope::Global,
            source_root: source_dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(PathBuf::new),
            source_path: source_dir.clone(),
            dest_name: "skill".to_string(),
            destination_path: dest_path,
            artifact: ImportArtifact::DirectoryCopy { source_dir },
            metadata: serde_json::json!({"original_name": "skill"}),
        }
    }

    fn outcome_status(summary: &ImportSummary, index: usize) -> Option<ImportStatus> {
        summary
            .outcomes
            .get(index)
            .map(|outcome| outcome.status.clone())
    }

    #[tokio::test]
    async fn first_file_import_creates_destination_and_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_path = scope_root.join("commands").join("hello.md");
        let candidate = file_candidate(source_path, dest_path.clone(), "hello");

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Created));
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "hello"
        );
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].dest_path, dest_path);
    }

    #[tokio::test]
    async fn second_unchanged_import_reports_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_path = scope_root.join("commands").join("hello.md");
        let candidate = file_candidate(source_path, dest_path.clone(), "hello");
        write_candidates(&scope_root, &[candidate.clone()]).await;
        let first_hash = hash_file(&dest_path).unwrap();

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Unchanged));
        assert_eq!(hash_file(&dest_path).unwrap(), first_hash);
    }

    #[tokio::test]
    async fn changed_source_updates_generated_destination() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_path = scope_root.join("commands").join("hello.md");
        write_candidates(
            &scope_root,
            &[file_candidate(
                source_path.clone(),
                dest_path.clone(),
                "one",
            )],
        )
        .await;

        let summary = write_candidates(
            &scope_root,
            &[file_candidate(source_path, dest_path.clone(), "two")],
        )
        .await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Updated));
        assert_eq!(tokio::fs::read_to_string(&dest_path).await.unwrap(), "two");
    }

    #[tokio::test]
    async fn user_edited_generated_destination_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_path = scope_root.join("commands").join("hello.md");
        write_candidates(
            &scope_root,
            &[file_candidate(
                source_path.clone(),
                dest_path.clone(),
                "one",
            )],
        )
        .await;
        tokio::fs::write(&dest_path, "user edit").await.unwrap();

        let summary = write_candidates(
            &scope_root,
            &[file_candidate(source_path, dest_path.clone(), "two")],
        )
        .await;

        assert_eq!(
            outcome_status(&summary, 0),
            Some(ImportStatus::UserModified)
        );
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "user edit"
        );
    }

    #[tokio::test]
    async fn existing_untracked_destination_is_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_path = scope_root.join("commands").join("hello.md");
        tokio::fs::create_dir_all(dest_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&dest_path, "existing").await.unwrap();
        let candidate = file_candidate(source_path, dest_path.clone(), "new");

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Conflict));
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "existing"
        );
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();
        assert!(manifest.entries.is_empty());
    }

    #[tokio::test]
    async fn generated_destination_owned_by_other_source_is_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let dest_path = scope_root.join("commands").join("hello.md");
        let first = file_candidate(
            temp.path().join("claude").join("hello.md"),
            dest_path.clone(),
            "first",
        );
        let second = file_candidate(
            temp.path().join("opencode").join("hello.md"),
            dest_path.clone(),
            "second",
        );

        let summary = write_candidates(&scope_root, &[first, second]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Created));
        assert_eq!(outcome_status(&summary, 1), Some(ImportStatus::Conflict));
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "first"
        );
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert!(manifest.entries[0].source_path.ends_with("claude/hello.md"));
    }

    #[tokio::test]
    async fn directory_artifact_copies_regular_files_and_skips_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_dir = temp.path().join("source_skill");
        tokio::fs::create_dir_all(source_dir.join("nested"))
            .await
            .unwrap();
        tokio::fs::write(source_dir.join("SKILL.md"), "skill")
            .await
            .unwrap();
        tokio::fs::write(source_dir.join("nested").join("note.txt"), "note")
            .await
            .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(source_dir.join("SKILL.md"), source_dir.join("link.md"))
            .unwrap();
        let dest_path = scope_root.join("skills").join("skill");
        let candidate = directory_candidate(source_dir.clone(), dest_path.clone());

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Created));
        assert_eq!(
            tokio::fs::read_to_string(dest_path.join("nested").join("note.txt"))
                .await
                .unwrap(),
            "note"
        );
        #[cfg(unix)]
        assert!(!dest_path.join("link.md").exists());
        assert_eq!(
            hash_directory(&source_dir).unwrap(),
            hash_directory(&dest_path).unwrap()
        );
    }

    #[tokio::test]
    async fn stale_manifest_does_not_delete_generated_destination() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_path = scope_root.join("commands").join("hello.md");
        write_candidates(
            &scope_root,
            &[file_candidate(source_path, dest_path.clone(), "hello")],
        )
        .await;

        let summary = write_candidates(&scope_root, &[]).await;

        assert!(summary.is_empty());
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn missing_source_reports_error_and_preserves_destination() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_path = scope_root.join("commands").join("hello.md");
        write_candidates(
            &scope_root,
            &[file_candidate(
                source_path.clone(),
                dest_path.clone(),
                "hello",
            )],
        )
        .await;
        tokio::fs::remove_file(&source_path).await.unwrap();
        let candidate = ImportCandidate {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Command,
            scope: ImportScope::Global,
            source_root: source_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            source_path,
            dest_name: "hello".to_string(),
            destination_path: dest_path.clone(),
            artifact: ImportArtifact::FileContent {
                content: "changed".to_string(),
            },
            metadata: serde_json::json!({"original_name": "hello"}),
        };

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Error));
        assert_eq!(summary.errors.len(), 1);
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn serialized_summary_and_last_report_omit_artifact_content() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("secret.md");
        let dest_path = scope_root.join("commands").join("secret.md");
        let candidate = file_candidate(source_path, dest_path, "sensitive generated body");

        let summary = write_candidates(&scope_root, &[candidate]).await;
        let summary_json = serde_json::to_string(&summary).unwrap();
        let manifest_json = tokio::fs::read_to_string(manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();

        assert!(!summary_json.contains("sensitive generated body"));
        assert!(!manifest_json.contains("sensitive generated body"));
        assert!(manifest_json.contains("last_report"));
    }
}
