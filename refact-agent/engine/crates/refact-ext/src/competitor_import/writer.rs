use std::ffi::OsString;
use std::io::{Error, ErrorKind, Result};
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::manifest::{
    hash_directory, hash_file, is_hash_limit_error, manifest_path_for_scope_root,
    write_string_atomic, ImportManifest, ImportManifestEntry, IMPORTER_VERSION,
    MAX_HASH_DIRECTORY_BYTES, MAX_HASH_DIRECTORY_DEPTH, MAX_HASH_DIRECTORY_ENTRIES,
    MAX_HASH_DIRECTORY_FILES, MAX_HASH_FILE_BYTES,
};
use super::types::{
    ImportArtifact, ImportCandidate, ImportCandidateSummary, ImportIssue, ImportOutcome,
    ImportReport, ImportScope, ImportStatus, ImportSummary,
};

#[cfg(test)]
pub async fn write_candidates(scope_root: &Path, candidates: &[ImportCandidate]) -> ImportSummary {
    write_candidates_for_scope(scope_root, &ImportScope::Global, candidates).await
}

#[cfg(test)]
pub async fn write_candidates_for_scope(
    scope_root: &Path,
    scope: &ImportScope,
    candidates: &[ImportCandidate],
) -> ImportSummary {
    write_candidates_for_scope_with_issues(scope_root, scope, candidates, &[]).await
}

pub(crate) async fn write_candidates_for_scope_with_issues(
    scope_root: &Path,
    scope: &ImportScope,
    candidates: &[ImportCandidate],
    existing_issues: &[ImportIssue],
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

    record_stale_entries(
        scope_root,
        scope,
        &manifest,
        candidates,
        existing_issues,
        &mut summary,
    );
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
    existing_issues: &[ImportIssue],
    summary: &mut ImportSummary,
) {
    for entry in &manifest.entries {
        if existing_issues
            .iter()
            .any(|issue| issue_matches_stale_entry(scope, entry, issue))
        {
            continue;
        }
        let source_matches = candidates
            .iter()
            .filter(|candidate| manifest_entry_matches_candidate(entry, candidate))
            .collect::<Vec<_>>();
        if source_matches
            .iter()
            .any(|candidate| candidate_matches_ownership(scope_root, entry, candidate))
        {
            continue;
        }
        if source_matches
            .iter()
            .any(|candidate| candidate_destination_differs(scope_root, entry, candidate))
        {
            summary.add_outcome(stale_outcome(
                scope_root,
                scope,
                entry,
                "source now maps to a different destination; generated destination preserved",
            ));
        } else if !source_path_exists(&entry.source_path) {
            summary.add_outcome(stale_outcome(
                scope_root,
                scope,
                entry,
                "source no longer exists; generated destination preserved",
            ));
        }
    }
}

fn issue_matches_stale_entry(
    scope: &ImportScope,
    entry: &ImportManifestEntry,
    issue: &ImportIssue,
) -> bool {
    issue
        .competitor
        .map(|competitor| competitor == entry.competitor)
        .unwrap_or(true)
        && issue.kind == Some(entry.kind)
        && issue.scope.as_ref() == Some(scope)
        && issue
            .path
            .as_ref()
            .is_some_and(|path| paths_equivalent(path, &entry.source_path))
}

fn candidate_matches_ownership(
    scope_root: &Path,
    entry: &ImportManifestEntry,
    candidate: &ImportCandidate,
) -> bool {
    if !manifest_entry_matches_candidate(entry, candidate) {
        return false;
    }
    resolve_destination_path(scope_root, &candidate.destination_path)
        .map(|dest_path| paths_equivalent(&dest_path, &entry.dest_path))
        .unwrap_or(false)
}

fn candidate_destination_differs(
    scope_root: &Path,
    entry: &ImportManifestEntry,
    candidate: &ImportCandidate,
) -> bool {
    resolve_destination_path(scope_root, &candidate.destination_path)
        .map(|dest_path| !paths_equivalent(&dest_path, &entry.dest_path))
        .unwrap_or(false)
}

fn source_path_exists(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(err) if err.kind() == ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

fn stale_outcome(
    scope_root: &Path,
    scope: &ImportScope,
    entry: &ImportManifestEntry,
    message: &str,
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
        message: message.to_string(),
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
            let source_hash = candidate.source_hash.clone();
            let desired_dest_hash = candidate.artifact_hash.clone();
            if current_dest_hash != entry.dest_hash && current_dest_hash != desired_dest_hash {
                return Ok(outcome(
                    candidate,
                    ImportStatus::UserModified,
                    "destination differs from previous generated hash".to_string(),
                ));
            }
            if current_dest_hash == desired_dest_hash {
                if entry.source_hash != source_hash
                    || entry.dest_hash != current_dest_hash
                    || entry.importer_version != IMPORTER_VERSION
                    || !manifest_entry_metadata_matches_candidate(&entry, candidate)
                {
                    manifest.upsert_entry(manifest_entry_from_candidate(
                        candidate,
                        dest_path,
                        source_hash,
                        current_dest_hash,
                    ));
                    return Ok(outcome(
                        candidate,
                        ImportStatus::Unchanged,
                        "generated destination is unchanged; refreshed import metadata".to_string(),
                    ));
                }
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

    let source_hash = candidate.source_hash.clone();
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
    validate_staged_directory_source(scope_root, &staging_root, source_dir)
}

fn validate_staged_directory_source(
    scope_root: &Path,
    staging_root: &Path,
    source_dir: &Path,
) -> Result<()> {
    let canonical_scope = canonical_existing_directory(scope_root)?;
    let canonical_staging = canonical_existing_directory(staging_root)?;
    if !canonical_staging.starts_with(&canonical_scope) {
        return Err(invalid_path_error(format!(
            "staging root escapes import scope: {}",
            staging_root.display()
        )));
    }
    let source_metadata = std::fs::symlink_metadata(source_dir)?;
    if source_metadata.file_type().is_symlink() || !source_metadata.file_type().is_dir() {
        return Err(invalid_path_error(format!(
            "directory source is not a regular directory: {}",
            source_dir.display()
        )));
    }
    let canonical_source =
        std::fs::canonicalize(source_dir).map(|path| dunce::simplified(&path).to_path_buf())?;
    if canonical_source.starts_with(&canonical_staging) {
        Ok(())
    } else {
        Err(invalid_path_error(format!(
            "directory source is outside source root: {}",
            source_dir.display()
        )))
    }
}

fn canonical_existing_directory(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(invalid_path_error(format!(
            "path is not a directory: {}",
            path.display()
        )));
    }
    std::fs::canonicalize(path).map(|path| dunce::simplified(&path).to_path_buf())
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
        Ok(path) => Ok(Some(dunce::simplified(&path).to_path_buf())),
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
                canonical = dunce::simplified(&canonical).to_path_buf();
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
        && paths_equivalent(&entry.source_path, &candidate.source_path)
}

fn paths_equivalent(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

fn path_key(path: &Path) -> PathBuf {
    canonical_path_if_exists(path)
        .ok()
        .flatten()
        .unwrap_or_else(|| dunce::simplified(&lexical_normalize(path)).to_path_buf())
}

fn manifest_entry_metadata_matches_candidate(
    entry: &ImportManifestEntry,
    candidate: &ImportCandidate,
) -> bool {
    entry.metadata == candidate_manifest_metadata(candidate)
}

fn candidate_manifest_metadata(candidate: &ImportCandidate) -> Option<Value> {
    if candidate.metadata.is_null() {
        None
    } else {
        Some(candidate.metadata.clone())
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
        replace_directory_staging(&staging, dest_path).await
    }
    .await;
    if copy_result.is_err() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    copy_result
}

async fn replace_directory_staging(staging: &Path, dest_path: &Path) -> Result<()> {
    replace_directory_staging_inner(staging, dest_path, || Ok(())).await
}

async fn replace_directory_staging_inner<F>(
    staging: &Path,
    dest_path: &Path,
    after_backup: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let existing = match tokio::fs::symlink_metadata(dest_path).await {
        Ok(metadata) => Some(metadata),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };
    if existing.is_none() {
        return tokio::fs::rename(staging, dest_path).await;
    }

    let parent = dest_path.parent().unwrap_or_else(|| Path::new("."));
    let dest_name = dest_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("import");
    let backup = parent.join(format!(".{}.{}.bak", dest_name, uuid::Uuid::new_v4()));
    tokio::fs::rename(dest_path, &backup).await?;

    let replace_result = match after_backup() {
        Ok(()) => tokio::fs::rename(staging, dest_path).await,
        Err(err) => Err(err),
    };
    if let Err(err) = replace_result {
        let restore_result = restore_directory_backup(dest_path, &backup).await;
        let _ = remove_existing_path(staging).await;
        if let Err(restore_err) = restore_result {
            return Err(Error::new(
                err.kind(),
                format!(
                    "failed to replace directory: {err}; failed to restore backup: {restore_err}"
                ),
            ));
        }
        return Err(err);
    }

    remove_existing_path(&backup).await
}

async fn restore_directory_backup(dest_path: &Path, backup: &Path) -> Result<()> {
    match tokio::fs::symlink_metadata(dest_path).await {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => tokio::fs::rename(backup, dest_path).await,
        Err(err) => Err(err),
    }
}

#[cfg(test)]
async fn replace_directory_staging_failing_after_backup(
    staging: &Path,
    dest_path: &Path,
) -> Result<()> {
    replace_directory_staging_inner(staging, dest_path, || {
        Err(Error::new(
            ErrorKind::Other,
            "injected directory replacement failure",
        ))
    })
    .await
}

async fn copy_directory_contents(source_dir: &Path, staging: &Path) -> Result<()> {
    tokio::fs::create_dir_all(staging).await?;
    let mut entry_count = 0usize;
    let mut file_count = 0usize;
    let mut total_bytes = 0u64;
    let mut entries = walkdir::WalkDir::new(source_dir)
        .follow_links(false)
        .sort_by_file_name()
        .max_depth(MAX_HASH_DIRECTORY_DEPTH + 1)
        .into_iter();
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(|err| Error::new(ErrorKind::Other, err.to_string()))?;
        let source_path = entry.path();
        if source_path == source_dir {
            continue;
        }
        entry_count += 1;
        if entry_count > MAX_HASH_DIRECTORY_ENTRIES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("directory copy exceeds {MAX_HASH_DIRECTORY_ENTRIES} entry limit"),
            ));
        }
        if entry.depth() > MAX_HASH_DIRECTORY_DEPTH {
            if entry.file_type().is_dir() {
                entries.skip_current_dir();
            }
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("directory copy exceeds {MAX_HASH_DIRECTORY_DEPTH} depth limit"),
            ));
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
            file_count += 1;
            if file_count > MAX_HASH_DIRECTORY_FILES {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("directory copy exceeds {MAX_HASH_DIRECTORY_FILES} file limit"),
                ));
            }
            if let Some(parent) = target_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let remaining = MAX_HASH_DIRECTORY_BYTES
                .checked_sub(total_bytes)
                .ok_or_else(|| {
                    Error::new(ErrorKind::InvalidData, "directory copy byte count overflow")
                })?;
            let copied =
                copy_file_limited(source_path, &target_path, MAX_HASH_FILE_BYTES, remaining)
                    .await?;
            total_bytes = total_bytes.checked_add(copied).ok_or_else(|| {
                Error::new(ErrorKind::InvalidData, "directory copy byte count overflow")
            })?;
            if total_bytes > MAX_HASH_DIRECTORY_BYTES {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("directory copy exceeds {MAX_HASH_DIRECTORY_BYTES} byte limit"),
                ));
            }
        }
    }
    Ok(())
}

async fn copy_file_limited(
    source_path: &Path,
    target_path: &Path,
    per_file_limit: u64,
    total_remaining: u64,
) -> Result<u64> {
    let metadata = tokio::fs::symlink_metadata(source_path).await?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "copy source is not a regular file: {}",
                source_path.display()
            ),
        ));
    }
    if metadata.len() > per_file_limit {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("directory copy file exceeds {per_file_limit} byte limit"),
        ));
    }
    if metadata.len() > total_remaining {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("directory copy exceeds {MAX_HASH_DIRECTORY_BYTES} byte limit"),
        ));
    }
    let mut source = tokio::fs::File::open(source_path).await?;
    let mut target = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target_path)
        .await?;
    let mut buffer = vec![0u8; 16 * 1024];
    let mut copied = 0u64;
    loop {
        let read = source.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        copied = copied.checked_add(read as u64).ok_or_else(|| {
            Error::new(ErrorKind::InvalidData, "directory copy byte count overflow")
        })?;
        if copied > per_file_limit {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("directory copy file exceeds {per_file_limit} byte limit while reading"),
            ));
        }
        if copied > total_remaining {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "directory copy exceeds {MAX_HASH_DIRECTORY_BYTES} byte limit while reading"
                ),
            ));
        }
        target.write_all(&buffer[..read]).await?;
    }
    target.flush().await?;
    if copied != metadata.len() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "directory copy source changed while reading: {} expected {} bytes, copied {copied} bytes",
                source_path.display(),
                metadata.len()
            ),
        ));
    }
    Ok(copied)
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
        metadata: candidate_manifest_metadata(candidate),
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
    use super::super::converters::convert_skill_package;
    use super::super::manifest::{
        hash_directory, hash_file, hash_string, manifest_path_for_scope_root,
        MAX_HASH_DIRECTORY_BYTES, MAX_HASH_FILE_BYTES,
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
            source_hash: hash_string(content),
            artifact_hash: hash_string(content),
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
            source_hash: hash_directory(&source_dir).unwrap(),
            artifact_hash: hash_directory(&source_dir).unwrap(),
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

    fn backup_paths(parent: &Path, dest_name: &str) -> Vec<PathBuf> {
        let prefix = format!(".{dest_name}.");
        std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".bak"))
            })
            .collect()
    }

    async fn write_manifest_entry(scope_root: &Path, entry: ImportManifestEntry) {
        let mut manifest = ImportManifest::default();
        manifest.entries.push(entry);
        manifest
            .write_to_path(&manifest_path_for_scope_root(scope_root))
            .await
            .unwrap();
    }

    fn subagent_candidate(
        source_path: PathBuf,
        dest_path: PathBuf,
        content: &str,
    ) -> ImportCandidate {
        if let Some(parent) = source_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&source_path, "subagent source").unwrap();
        ImportCandidate {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Subagent,
            scope: ImportScope::Global,
            source_root: source_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(PathBuf::new),
            source_path,
            dest_name: "reviewer".to_string(),
            destination_path: dest_path,
            artifact: ImportArtifact::FileContent {
                content: content.to_string(),
            },
            source_hash: hash_string("subagent source"),
            artifact_hash: hash_string(content),
            metadata: serde_json::json!({"original_name": "reviewer"}),
        }
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
    async fn old_importer_version_regenerates_unmodified_subagent_destination() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("reviewer.md");
        let dest_rel = PathBuf::from("subagents").join("reviewer.yaml");
        let dest_path = scope_root.join(&dest_rel);
        let old_yaml = "schema_version: 1\nid: reviewer\n";
        let new_yaml = "schema_version: 2\nid: reviewer\n";
        let candidate = subagent_candidate(source_path.clone(), dest_rel, new_yaml);
        tokio::fs::create_dir_all(dest_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&dest_path, old_yaml).await.unwrap();
        write_manifest_entry(
            &scope_root,
            ImportManifestEntry {
                competitor: candidate.competitor,
                kind: candidate.kind,
                source_path: source_path.clone(),
                source_hash: candidate.source_hash.clone(),
                dest_path: dest_path.clone(),
                dest_hash: hash_file(&dest_path).unwrap(),
                importer_version: "competitor_import_v1".to_string(),
                last_imported_at: Utc::now(),
                metadata: Some(candidate.metadata.clone()),
            },
        )
        .await;

        let summary = write_candidates(&scope_root, &[candidate]).await;
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Updated));
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            new_yaml
        );
        assert_eq!(manifest.entries[0].importer_version, IMPORTER_VERSION);
    }

    #[tokio::test]
    async fn old_importer_version_preserves_user_modified_subagent_destination() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("reviewer.md");
        let dest_rel = PathBuf::from("subagents").join("reviewer.yaml");
        let dest_path = scope_root.join(&dest_rel);
        let old_yaml = "schema_version: 1\nid: reviewer\n";
        let new_yaml = "schema_version: 2\nid: reviewer\n";
        let candidate = subagent_candidate(source_path.clone(), dest_rel, new_yaml);
        tokio::fs::create_dir_all(dest_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&dest_path, old_yaml).await.unwrap();
        let old_dest_hash = hash_file(&dest_path).unwrap();
        write_manifest_entry(
            &scope_root,
            ImportManifestEntry {
                competitor: candidate.competitor,
                kind: candidate.kind,
                source_path: source_path.clone(),
                source_hash: candidate.source_hash.clone(),
                dest_path: dest_path.clone(),
                dest_hash: old_dest_hash,
                importer_version: "competitor_import_v1".to_string(),
                last_imported_at: Utc::now(),
                metadata: Some(candidate.metadata.clone()),
            },
        )
        .await;
        tokio::fs::write(&dest_path, "user edit").await.unwrap();

        let summary = write_candidates(&scope_root, &[candidate]).await;
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();

        assert_eq!(
            outcome_status(&summary, 0),
            Some(ImportStatus::UserModified)
        );
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "user edit"
        );
        assert_eq!(manifest.entries[0].importer_version, "competitor_import_v1");
    }

    #[tokio::test]
    async fn matching_generated_destination_refreshes_stale_manifest_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("reviewer.md");
        let dest_rel = PathBuf::from("subagents").join("reviewer.yaml");
        let dest_path = scope_root.join(&dest_rel);
        let current_yaml = "schema_version: 2\nid: reviewer\n";
        let candidate = subagent_candidate(source_path.clone(), dest_rel, current_yaml);
        let candidate_source_hash = candidate.source_hash.clone();
        tokio::fs::create_dir_all(dest_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&dest_path, current_yaml).await.unwrap();
        write_manifest_entry(
            &scope_root,
            ImportManifestEntry {
                competitor: candidate.competitor,
                kind: candidate.kind,
                source_path: source_path.clone(),
                source_hash: hash_string("old source"),
                dest_path: dest_path.clone(),
                dest_hash: hash_string("schema_version: 1\nid: reviewer\n"),
                importer_version: "competitor_import_v1".to_string(),
                last_imported_at: Utc::now(),
                metadata: None,
            },
        )
        .await;

        let summary = write_candidates(&scope_root, &[candidate]).await;
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Unchanged));
        assert!(!summary.has_imported_changes());
        assert_eq!(manifest.entries[0].importer_version, IMPORTER_VERSION);
        assert_eq!(manifest.entries[0].source_hash, candidate_source_hash);
        assert_eq!(
            manifest.entries[0].dest_hash,
            hash_file(&dest_path).unwrap()
        );
        assert!(manifest.entries[0].metadata.is_some());
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
    async fn mutated_file_source_after_candidate_creation_keeps_snapshot_hash() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_rel = command_destination();
        let dest_path = scope_root.join(&dest_rel);
        let candidate = file_candidate(source_path.clone(), dest_rel, "original");
        tokio::fs::write(&source_path, "mutated").await.unwrap();

        let summary = write_candidates(&scope_root, &[candidate]).await;
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Created));
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "original"
        );
        assert_eq!(manifest.entries[0].source_hash, hash_string("original"));
        assert_ne!(
            manifest.entries[0].source_hash,
            hash_file(&source_path).unwrap()
        );
    }

    #[tokio::test]
    async fn mutated_skill_source_after_candidate_creation_keeps_staged_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let scope_root = workspace.join(".refact");
        let skill_dir = workspace.join(".claude").join("skills").join("foo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Foo Skill\n---\n# Foo\nUse foo.",
        )
        .unwrap();
        std::fs::write(skill_dir.join("notes.txt"), "original notes").unwrap();
        let context = super::super::types::ConversionContext {
            competitor: Competitor::ClaudeCode,
            scope: ImportScope::Project {
                root: workspace.clone(),
            },
            source_root: workspace.join(".claude"),
        };
        let candidate = convert_skill_package(
            &context,
            &skill_dir,
            &scope_root.join("imports").join("staging").join("claude"),
        )
        .unwrap();
        std::fs::write(skill_dir.join("notes.txt"), "mutated notes").unwrap();

        let summary = write_candidates_for_scope(
            &scope_root,
            &ImportScope::Project { root: workspace },
            &[candidate.clone()],
        )
        .await;
        let dest_path = scope_root.join(&candidate.destination_path);
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Created));
        assert_eq!(
            tokio::fs::read_to_string(dest_path.join("notes.txt"))
                .await
                .unwrap(),
            "original notes"
        );
        assert_eq!(manifest.entries[0].source_hash, candidate.source_hash);
        assert_eq!(manifest.entries[0].dest_hash, candidate.artifact_hash);
    }

    #[test]
    fn staged_directory_source_under_import_staging_is_allowed() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let staging_root = scope_root.join("imports").join("staging");
        let source_dir = staging_root.join("claude").join("skill");
        std::fs::create_dir_all(&source_dir).unwrap();

        validate_staged_directory_source(&scope_root, &staging_root, &source_dir).unwrap();
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
    async fn directory_replacement_success_removes_backup() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path();
        let dest_path = parent.join("skill");
        let staging = parent.join(".skill.test.tmp");
        tokio::fs::create_dir_all(&dest_path).await.unwrap();
        tokio::fs::write(dest_path.join("old.txt"), "old")
            .await
            .unwrap();
        tokio::fs::create_dir_all(&staging).await.unwrap();
        tokio::fs::write(staging.join("new.txt"), "new")
            .await
            .unwrap();

        replace_directory_staging(&staging, &dest_path)
            .await
            .unwrap();

        assert!(!staging.exists());
        assert!(!dest_path.join("old.txt").exists());
        assert_eq!(
            tokio::fs::read_to_string(dest_path.join("new.txt"))
                .await
                .unwrap(),
            "new"
        );
        assert!(backup_paths(parent, "skill").is_empty());
    }

    #[tokio::test]
    async fn directory_replacement_failure_restores_backup() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path();
        let dest_path = parent.join("skill");
        let staging = parent.join(".skill.test.tmp");
        tokio::fs::create_dir_all(&dest_path).await.unwrap();
        tokio::fs::write(dest_path.join("old.txt"), "old")
            .await
            .unwrap();
        tokio::fs::create_dir_all(&staging).await.unwrap();
        tokio::fs::write(staging.join("new.txt"), "new")
            .await
            .unwrap();

        let err = replace_directory_staging_failing_after_backup(&staging, &dest_path)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("injected"));
        assert!(!staging.exists());
        assert_eq!(
            tokio::fs::read_to_string(dest_path.join("old.txt"))
                .await
                .unwrap(),
            "old"
        );
        assert!(!dest_path.join("new.txt").exists());
        assert!(backup_paths(parent, "skill").is_empty());
    }

    #[tokio::test]
    async fn changed_directory_source_updates_generated_destination() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_dir = temp.path().join("source_skill");
        tokio::fs::create_dir_all(&source_dir).await.unwrap();
        tokio::fs::write(source_dir.join("SKILL.md"), "skill")
            .await
            .unwrap();
        tokio::fs::write(source_dir.join("note.txt"), "one")
            .await
            .unwrap();
        let dest_rel = PathBuf::from("skills").join("skill");
        let dest_path = scope_root.join(&dest_rel);
        write_candidates(
            &scope_root,
            &[directory_candidate(source_dir.clone(), dest_rel.clone())],
        )
        .await;
        tokio::fs::write(source_dir.join("note.txt"), "two")
            .await
            .unwrap();

        let summary = write_candidates(
            &scope_root,
            &[directory_candidate(source_dir.clone(), dest_rel)],
        )
        .await;
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Updated));
        assert_eq!(
            tokio::fs::read_to_string(dest_path.join("note.txt"))
                .await
                .unwrap(),
            "two"
        );
        assert!(backup_paths(dest_path.parent().unwrap(), "skill").is_empty());
        assert_eq!(
            manifest.entries[0].dest_hash,
            hash_directory(&dest_path).unwrap()
        );
    }

    #[tokio::test]
    async fn oversized_directory_copy_fails_without_writing_partial_destination() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_dir = temp.path().join("source_skill");
        tokio::fs::create_dir_all(&source_dir).await.unwrap();
        tokio::fs::write(source_dir.join("SKILL.md"), "skill")
            .await
            .unwrap();
        std::fs::File::create(source_dir.join("huge.bin"))
            .unwrap()
            .set_len(MAX_HASH_DIRECTORY_BYTES + 1)
            .unwrap();
        let dest_rel = PathBuf::from("skills").join("skill");
        let dest_path = scope_root.join(&dest_rel);
        let candidate = ImportCandidate {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Skill,
            scope: ImportScope::Global,
            source_root: source_dir.parent().unwrap().to_path_buf(),
            source_path: source_dir.clone(),
            dest_name: "skill".to_string(),
            destination_path: dest_rel,
            source_hash: hash_string("source"),
            artifact_hash: hash_string("artifact"),
            artifact: ImportArtifact::DirectoryCopy { source_dir },
            metadata: serde_json::json!({"original_name": "skill"}),
        };

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Error));
        assert!(!dest_path.exists());
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

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_staging_root_directory_source_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let outside = temp.path().join("outside-staging");
        let source_dir = outside.join("claude").join("skill");
        tokio::fs::create_dir_all(&source_dir).await.unwrap();
        tokio::fs::write(source_dir.join("SKILL.md"), "skill")
            .await
            .unwrap();
        tokio::fs::create_dir_all(scope_root.join("imports"))
            .await
            .unwrap();
        std::os::unix::fs::symlink(&outside, scope_root.join("imports").join("staging")).unwrap();
        let mut candidate = directory_candidate(source_dir, PathBuf::from("skills").join("skill"));
        candidate.source_root = temp.path().join("source-root");
        candidate.source_path = candidate.source_root.join("skill");
        std::fs::create_dir_all(&candidate.source_root).unwrap();

        let summary = write_candidates(&scope_root, &[candidate]).await;

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Error));
        assert!(!scope_root.join("skills").join("skill").exists());
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
            source_hash: hash_string("source"),
            artifact_hash: hash_string("generated"),
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
            source_hash: hash_string("source"),
            artifact_hash: hash_string("generated"),
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
            &[file_candidate(source_path.clone(), dest_rel, "hello")],
        )
        .await;
        let before_entries =
            ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
                .await
                .unwrap()
                .entries;
        tokio::fs::remove_file(&source_path).await.unwrap();

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
    async fn absent_candidate_with_existing_source_is_not_reported_stale() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let dest_rel = command_destination();
        write_candidates(
            &scope_root,
            &[file_candidate(source_path.clone(), dest_rel, "hello")],
        )
        .await;

        let summary = write_candidates(&scope_root, &[]).await;

        assert!(source_path.exists());
        assert!(summary.outcomes.is_empty());
        assert_eq!(summary.status_counts.get(&ImportStatus::Stale), None);
    }

    #[tokio::test]
    async fn remapped_destination_reports_old_entry_stale_and_creates_new_destination() {
        let temp = tempfile::tempdir().unwrap();
        let scope_root = temp.path().join("refact");
        let source_path = temp.path().join("source").join("hello.md");
        let old_dest_rel = PathBuf::from("commands").join("old.md");
        let new_dest_rel = PathBuf::from("commands").join("new.md");
        let old_dest_path = scope_root.join(&old_dest_rel);
        let new_dest_path = scope_root.join(&new_dest_rel);
        write_candidates(
            &scope_root,
            &[file_candidate(
                source_path.clone(),
                old_dest_rel.clone(),
                "hello",
            )],
        )
        .await;

        let summary = write_candidates(
            &scope_root,
            &[file_candidate(source_path, new_dest_rel, "hello")],
        )
        .await;

        assert_eq!(summary.status_counts.get(&ImportStatus::Stale), Some(&1));
        assert_eq!(summary.status_counts.get(&ImportStatus::Created), Some(&1));
        assert!(summary.outcomes.iter().any(|outcome| {
            outcome.status == ImportStatus::Stale
                && outcome.candidate.destination_path == old_dest_rel
                && outcome.message
                    == "source now maps to a different destination; generated destination preserved"
        }));
        assert_eq!(
            tokio::fs::read_to_string(old_dest_path).await.unwrap(),
            "hello"
        );
        assert_eq!(
            tokio::fs::read_to_string(new_dest_path).await.unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn missing_source_candidate_uses_snapshot_and_updates_destination() {
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
            source_hash: hash_string("changed source"),
            artifact_hash: hash_string("changed"),
            metadata: serde_json::json!({"original_name": "hello"}),
        };

        let summary = write_candidates(&scope_root, &[candidate]).await;
        let manifest = ImportManifest::read_from_path(&manifest_path_for_scope_root(&scope_root))
            .await
            .unwrap();

        assert_eq!(outcome_status(&summary, 0), Some(ImportStatus::Updated));
        assert!(summary.errors.is_empty());
        assert_eq!(
            tokio::fs::read_to_string(&dest_path).await.unwrap(),
            "changed"
        );
        assert_eq!(
            manifest.entries[0].source_hash,
            hash_string("changed source")
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
