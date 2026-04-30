use std::ffi::OsString;
use std::io::{Error, ErrorKind, Result};
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use serde_json::Value;

use super::manifest::{
    hash_directory, hash_file, is_hash_limit_error, manifest_path_for_scope_root,
    write_string_atomic, ImportManifest, ImportManifestEntry, IMPORTER_VERSION,
};
use super::types::{
    ImportArtifact, ImportCandidate, ImportCandidateSummary, ImportIssue, ImportOutcome,
    ImportReport, ImportScope, ImportStatus, ImportSummary,
};

#[cfg(test)]
pub async fn write_candidates(scope_root: &Path, candidates: &[ImportCandidate]) -> ImportSummary {
    write_candidates_for_scope(scope_root, &ImportScope::Global, candidates).await
}

pub async fn write_candidates_for_scope(
    scope_root: &Path,
    scope: &ImportScope,
    candidates: &[ImportCandidate],
) -> ImportSummary {
    let mut summary = ImportSummary::default();
    let manifest_path = manifest_path_for_scope_root(scope_root);
    if candidates.is_empty() && !manifest_path.exists() {
        return summary;
    }
    for candidate in candidates {
        summary.record_candidate(candidate);
    }
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
            summary.mark_completed();
            return summary;
        }
    };

    record_stale_entries(scope_root, scope, &manifest, candidates, &mut summary);
    for candidate in candidates {
        match write_candidate(scope_root, &mut manifest, candidate).await {
            CandidateWriteResult::Outcome(outcome) => summary.add_outcome(outcome),
            CandidateWriteResult::Error { outcome, issue } => {
                summary.add_outcome(outcome);
                summary.issues.push(issue.clone());
                summary.errors.push(issue);
            }
        }
    }

    summary.mark_completed();
    manifest.last_report = Some(ImportReport::from_summary(&summary));
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

fn record_stale_entries(
    scope_root: &Path,
    scope: &ImportScope,
    manifest: &ImportManifest,
    candidates: &[ImportCandidate],
    summary: &mut ImportSummary,
) {
    for entry in &manifest.entries {
        if candidates
            .iter()
            .any(|candidate| manifest_entry_matches_candidate(entry, candidate))
        {
            continue;
        }
        summary.add_outcome(stale_outcome(scope_root, scope, entry));
    }
}

fn stale_outcome(
    scope_root: &Path,
    scope: &ImportScope,
    entry: &ImportManifestEntry,
) -> ImportOutcome {
    let destination_path = entry
        .dest_path
        .strip_prefix(scope_root)
        .unwrap_or(&entry.dest_path)
        .to_path_buf();
    ImportOutcome {
        candidate: ImportCandidateSummary {
            competitor: entry.competitor,
            kind: entry.kind,
            scope: scope.clone(),
            source_root: entry
                .source_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            source_path: entry.source_path.clone(),
            dest_name: dest_name_from_path(&destination_path),
            destination_path,
            metadata: entry.metadata.clone().unwrap_or(Value::Null),
        },
        status: ImportStatus::Stale,
        message: "source no longer exists; generated destination preserved".to_string(),
    }
}

fn dest_name_from_path(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "generated".to_string())
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
    let dest_path = resolve_destination_path(scope_root, &candidate.destination_path)?;
    validate_source_containment(scope_root, candidate)?;
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
            let current_dest_hash = match hash_existing_path(&dest_path) {
                Ok(hash) => hash,
                Err(err) if is_hash_limit_error(&err) => {
                    return Ok(outcome(
                        candidate,
                        ImportStatus::UserModified,
                        "destination too large to verify safely".to_string(),
                    ));
                }
                Err(err) => return Err(err),
            };
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

fn resolve_destination_path(scope_root: &Path, destination_path: &Path) -> Result<PathBuf> {
    let relative_path = clean_relative_destination_path(destination_path)?;
    let dest_path = scope_root.join(relative_path);
    let lexical_scope = lexical_absolute(scope_root)?;
    let lexical_dest = lexical_absolute(&dest_path)?;
    if !lexical_dest.starts_with(&lexical_scope) {
        return Err(invalid_path_error(format!(
            "destination path escapes import scope: {}",
            destination_path.display()
        )));
    }
    let canonical_scope = canonicalize_existing_prefix(scope_root)?;
    let canonical_dest = canonicalize_existing_prefix(&dest_path)?;
    if !canonical_dest.starts_with(&canonical_scope) {
        return Err(invalid_path_error(format!(
            "destination path escapes import scope through existing path components: {}",
            destination_path.display()
        )));
    }
    Ok(dest_path)
}

fn clean_relative_destination_path(destination_path: &Path) -> Result<PathBuf> {
    if destination_path.is_absolute() {
        return Err(invalid_path_error(format!(
            "destination path must be relative: {}",
            destination_path.display()
        )));
    }
    let mut clean = PathBuf::new();
    for component in destination_path.components() {
        match component {
            Component::Normal(value) => clean.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(invalid_path_error(format!(
                    "destination path contains unsupported components: {}",
                    destination_path.display()
                )));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(invalid_path_error("destination path is empty"));
    }
    Ok(clean)
}

fn validate_source_containment(scope_root: &Path, candidate: &ImportCandidate) -> Result<()> {
    validate_project_source_root(candidate)?;
    validate_source_path_under_root(
        "source path",
        &candidate.source_path,
        &candidate.source_root,
    )?;
    if let ImportArtifact::DirectoryCopy { source_dir } = &candidate.artifact {
        validate_directory_source_path(scope_root, source_dir, &candidate.source_root)?;
    }
    Ok(())
}

fn validate_project_source_root(candidate: &ImportCandidate) -> Result<()> {
    let ImportScope::Project { root } = &candidate.scope else {
        return Ok(());
    };
    match existing_path_is_under_root(&candidate.source_root, root)? {
        None | Some(true) => Ok(()),
        Some(false) => Err(invalid_path_error(format!(
            "source root is outside project scope: {}",
            candidate.source_root.display()
        ))),
    }
}

fn validate_source_path_under_root(label: &str, path: &Path, root: &Path) -> Result<()> {
    match existing_path_is_under_root(path, root)? {
        None | Some(true) => Ok(()),
        Some(false) => Err(invalid_path_error(format!(
            "{label} is outside source root: {}",
            path.display()
        ))),
    }
}

fn validate_directory_source_path(
    scope_root: &Path,
    source_dir: &Path,
    source_root: &Path,
) -> Result<()> {
    match existing_path_is_under_root(source_dir, source_root)? {
        None | Some(true) => return Ok(()),
        Some(false) => {}
    }
    let staging_root = scope_root.join("imports").join("staging");
    match existing_path_is_under_root(source_dir, &staging_root)? {
        Some(true) => Ok(()),
        None | Some(false) => Err(invalid_path_error(format!(
            "directory source is outside source root: {}",
            source_dir.display()
        ))),
    }
}

fn existing_path_is_under_root(path: &Path, root: &Path) -> Result<Option<bool>> {
    let Some(path) = canonical_path_if_exists(path)? else {
        return Ok(None);
    };
    let Some(root) = canonical_path_if_exists(root)? else {
        return Ok(Some(false));
    };
    Ok(Some(path.starts_with(root)))
}

fn canonical_path_if_exists(path: &Path) -> Result<Option<PathBuf>> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(Some(path)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn canonicalize_existing_prefix(path: &Path) -> Result<PathBuf> {
    let absolute = lexical_absolute(path)?;
    let mut probe = absolute.clone();
    let mut suffix = Vec::<OsString>::new();
    loop {
        match std::fs::canonicalize(&probe) {
            Ok(mut canonical) => {
                for component in suffix.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                let Some(name) = probe.file_name().map(|name| name.to_os_string()) else {
                    return Ok(absolute);
                };
                suffix.push(name);
                if !probe.pop() {
                    return Ok(absolute);
                }
            }
            Err(err) => return Err(err),
        }
    }
}

fn lexical_absolute(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(lexical_normalize(&absolute))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

fn invalid_path_error(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
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
    use super::super::manifest::{
        hash_directory, hash_file, manifest_path_for_scope_root, MAX_HASH_FILE_BYTES,
    };
    use super::super::types::{Competitor, ImportKind, ImportScope};

    fn command_destination() -> PathBuf {
        PathBuf::from("commands").join("hello.md")
    }

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
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        let candidate = file_candidate(source_path, dest_rel, "hello");

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
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        let candidate = file_candidate(source_path, dest_rel, "hello");
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
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        write_candidates(
            &scope_root,
            &[file_candidate(source_path.clone(), dest_rel.clone(), "one")],
        )
        .await;

        let summary =
            write_candidates(&scope_root, &[file_candidate(source_path, dest_rel, "two")]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Updated));
        assert_eq!(tokio::fs::read_to_string(&dest_path).await.unwrap(), "two");
    }

    #[tokio::test]
    async fn user_edited_generated_destination_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        write_candidates(
            &scope_root,
            &[file_candidate(source_path.clone(), dest_rel.clone(), "one")],
        )
        .await;
        tokio::fs::write(&dest_path, "user edit").await.unwrap();

        let summary =
            write_candidates(&scope_root, &[file_candidate(source_path, dest_rel, "two")]).await;

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
    async fn oversized_generated_destination_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        write_candidates(
            &scope_root,
            &[file_candidate(source_path.clone(), dest_rel.clone(), "one")],
        )
        .await;
        std::fs::File::create(&dest_path)
            .unwrap()
            .set_len(MAX_HASH_FILE_BYTES + 1)
            .unwrap();

        let summary =
            write_candidates(&scope_root, &[file_candidate(source_path, dest_rel, "two")]).await;

        assert_eq!(
            outcome_status(&summary, 0),
            Some(ImportStatus::UserModified)
        );
        assert!(summary.outcomes[0].message.contains("too large"));
        assert_eq!(
            std::fs::metadata(&dest_path).unwrap().len(),
            MAX_HASH_FILE_BYTES + 1
        );
    }

    #[tokio::test]
    async fn existing_untracked_destination_is_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        tokio::fs::create_dir_all(dest_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&dest_path, "existing").await.unwrap();
        let candidate = file_candidate(source_path, dest_rel, "new");

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
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        let first = file_candidate(
            temp.path().join("claude").join("hello.md"),
            dest_rel.clone(),
            "first",
        );
        let second = file_candidate(
            temp.path().join("opencode").join("hello.md"),
            dest_rel,
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
        let dest_rel = PathBuf::from("skills").join("skill");
        let dest_path = scope_root.join(&dest_rel);
        let candidate = directory_candidate(source_dir.clone(), dest_rel);

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
    async fn absolute_destination_path_is_rejected_without_writing_outside_scope() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let outside = temp.path().join("outside.md");
        let candidate = file_candidate(source_path, outside.clone(), "hello");

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Error));
        assert!(!outside.exists());
    }

    #[tokio::test]
    async fn traversing_destination_path_is_rejected_without_writing_outside_scope() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let outside = temp.path().join("escape.md");
        let candidate = file_candidate(source_path, PathBuf::from("../escape.md"), "hello");

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Error));
        assert!(!outside.exists());
    }

    #[tokio::test]
    async fn directory_copy_destination_traversal_does_not_remove_existing_path() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_dir = temp.path().join("source_skill");
        tokio::fs::create_dir_all(&source_dir).await.unwrap();
        tokio::fs::write(source_dir.join("SKILL.md"), "skill")
            .await
            .unwrap();
        let victim = temp.path().join("victim");
        tokio::fs::create_dir_all(&victim).await.unwrap();
        tokio::fs::write(victim.join("keep.txt"), "keep")
            .await
            .unwrap();
        let candidate = directory_candidate(source_dir, PathBuf::from("../victim"));

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Error));
        assert_eq!(
            tokio::fs::read_to_string(victim.join("keep.txt"))
                .await
                .unwrap(),
            "keep"
        );
    }

    #[tokio::test]
    async fn source_path_outside_source_root_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_root = temp.path().join("source_root");
        let outside_source = temp.path().join("outside").join("hello.md");
        tokio::fs::create_dir_all(&source_root).await.unwrap();
        tokio::fs::create_dir_all(outside_source.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&outside_source, "source").await.unwrap();
        let candidate = ImportCandidate {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Command,
            scope: ImportScope::Global,
            source_root,
            source_path: outside_source,
            dest_name: "hello".to_string(),
            destination_path: command_destination(),
            artifact: ImportArtifact::FileContent {
                content: "generated".to_string(),
            },
            metadata: serde_json::json!({"original_name": "hello"}),
        };

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Error));
        assert!(!scope_root.join(command_destination()).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn project_source_root_symlink_outside_workspace_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let outside_root = temp.path().join("outside_continue");
        let linked_root = workspace.join(".continue");
        let source_path = linked_root.join("prompts").join("hello.md");
        tokio::fs::create_dir_all(outside_root.join("prompts"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::write(outside_root.join("prompts").join("hello.md"), "source")
            .await
            .unwrap();
        std::os::unix::fs::symlink(&outside_root, &linked_root).unwrap();
        let scope_root = workspace.join(".refact");
        let candidate = ImportCandidate {
            competitor: Competitor::ContinueDev,
            kind: ImportKind::Command,
            scope: ImportScope::Project {
                root: workspace.clone(),
            },
            source_root: linked_root,
            source_path,
            dest_name: "hello".to_string(),
            destination_path: command_destination(),
            artifact: ImportArtifact::FileContent {
                content: "generated".to_string(),
            },
            metadata: serde_json::json!({"original_name": "hello"}),
        };

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Error));
        assert!(!scope_root.join(command_destination()).exists());
    }

    #[tokio::test]
    async fn stale_manifest_reports_stale_and_preserves_generated_destination() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        write_candidates(
            &scope_root,
            &[file_candidate(source_path, dest_rel, "hello")],
        )
        .await;
        let before_entries =
            ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
                .await
                .unwrap()
                .entries;

        let summary = write_candidates(&scope_root, &[]).await;
        let after_entries =
            ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
                .await
                .unwrap()
                .entries;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Stale));
        assert_eq!(summary.status_counts.get(&ImportStatus::Stale), Some(&1));
        assert_eq!(
            summary.outcomes[0].message,
            "source no longer exists; generated destination preserved"
        );
        assert_eq!(before_entries, after_entries);
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
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        write_candidates(
            &scope_root,
            &[file_candidate(
                source_path.clone(),
                dest_rel.clone(),
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
            destination_path: dest_rel,
            artifact: ImportArtifact::FileContent {
                content: "changed".to_string(),
            },
            metadata: serde_json::json!({"original_name": "hello"}),
        };

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Error));
        assert_eq!(summary.errors.len(), 1);
        assert!(!summary.errors[0].message.contains("outside source root"));
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn corrupt_manifest_reports_discovered_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_rel = command_destination();
        let manifest_path = manifest_path_for_scope_root(&scope_root);
        tokio::fs::create_dir_all(manifest_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&manifest_path, "not-json").await.unwrap();
        let candidate = file_candidate(source_path, dest_rel, "hello");

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(summary.candidates.len(), 1);
        assert!(summary.outcomes.is_empty());
        assert_eq!(summary.errors.len(), 1);
        assert!(summary.errors[0]
            .message
            .contains("failed to read import manifest"));
        assert!(!scope_root.join(command_destination()).exists());
    }

    #[tokio::test]
    async fn serialized_summary_and_last_report_omit_artifact_content() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("secret.md");
        let dest_rel = PathBuf::from("commands").join("secret.md");
        let candidate = file_candidate(source_path, dest_rel, "sensitive generated body");

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
