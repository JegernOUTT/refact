use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::types::{
    Competitor, ConversionContext, ImportIssue, ImportKind, ImportPrivacyFilter, ImportScope,
    ImportSourceRoot, ImportStatus,
};

pub mod claude;
pub mod continue_dev;
pub mod kilo;
pub mod opencode;

pub fn config_root_from_refact_config_dir(refact_config_dir: &Path) -> PathBuf {
    refact_config_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| refact_config_dir.to_path_buf())
}

pub fn discover_global_sources(home_dir: &Path, config_dir: &Path) -> Vec<ImportSourceRoot> {
    vec![
        ImportSourceRoot {
            competitor: Competitor::ClaudeCode,
            scope: ImportScope::Global,
            path: home_dir.join(".claude"),
        },
        ImportSourceRoot {
            competitor: Competitor::OpenCode,
            scope: ImportScope::Global,
            path: config_dir.join("opencode"),
        },
        ImportSourceRoot {
            competitor: Competitor::KiloCode,
            scope: ImportScope::Global,
            path: config_dir.join("kilo"),
        },
        ImportSourceRoot {
            competitor: Competitor::KiloCode,
            scope: ImportScope::Global,
            path: home_dir.join(".kilo"),
        },
        ImportSourceRoot {
            competitor: Competitor::KiloCode,
            scope: ImportScope::Global,
            path: home_dir.join(".kilocode"),
        },
        ImportSourceRoot {
            competitor: Competitor::ContinueDev,
            scope: ImportScope::Global,
            path: home_dir.join(".continue"),
        },
    ]
}

pub fn discover_project_sources(workspace_root: &Path) -> Vec<ImportSourceRoot> {
    let scope = ImportScope::Project {
        root: workspace_root.to_path_buf(),
    };
    vec![
        ImportSourceRoot {
            competitor: Competitor::ClaudeCode,
            scope: scope.clone(),
            path: workspace_root.join(".claude"),
        },
        ImportSourceRoot {
            competitor: Competitor::OpenCode,
            scope: scope.clone(),
            path: workspace_root.join(".opencode"),
        },
        ImportSourceRoot {
            competitor: Competitor::KiloCode,
            scope: scope.clone(),
            path: workspace_root.join(".kilo"),
        },
        ImportSourceRoot {
            competitor: Competitor::KiloCode,
            scope: scope.clone(),
            path: workspace_root.join(".kilocode"),
        },
        ImportSourceRoot {
            competitor: Competitor::ContinueDev,
            scope,
            path: workspace_root.join(".continue"),
        },
    ]
}

pub fn normalize_project_root(root: &Path) -> PathBuf {
    let absolute = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(root))
            .unwrap_or_else(|_| root.to_path_buf())
    };
    lexical_normalize(&absolute)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

pub fn discover_project_scopes(workspace_roots: &[PathBuf]) -> Vec<ImportScope> {
    let mut seen = HashSet::new();
    let mut scopes = Vec::new();
    for root in workspace_roots {
        let normalized_root = normalize_project_root(root);
        let dedup_key = fs::canonicalize(root)
            .map(|path| dunce::simplified(&path).to_path_buf())
            .unwrap_or_else(|_| normalized_root.clone());
        if seen.insert(dedup_key) {
            scopes.push(ImportScope::Project {
                root: normalized_root,
            });
        }
    }
    scopes
}

pub(super) fn regular_dir_exists(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

pub(super) fn regular_file_exists(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

pub(super) fn project_scan_root_allowed(
    path: &Path,
    workspace_root: &Path,
) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("failed to inspect source root: {err}")),
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err("project source root is a symlink and was skipped".to_string());
    }
    if !file_type.is_dir() {
        return Ok(false);
    }
    let canonical_root = fs::canonicalize(workspace_root)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|err| format!("failed to canonicalize workspace root: {err}"))?;
    let canonical_path = fs::canonicalize(path)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|err| format!("failed to canonicalize source root: {err}"))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("project source root resolves outside workspace and was skipped".to_string());
    }
    Ok(true)
}

pub(super) fn scan_root_allowed(context: &ConversionContext, path: &Path) -> Result<bool, String> {
    match &context.scope {
        ImportScope::Project {
            root,
        } => project_scan_root_allowed(path, root),
        ImportScope::Global => Ok(regular_dir_exists(path)),
    }
}

pub(super) fn skipped_root_issue(
    context: &ConversionContext,
    kind: Option<ImportKind>,
    path: &Path,
    message: impl Into<String>,
) -> ImportIssue {
    ImportIssue {
        competitor: Some(context.competitor),
        kind,
        scope: Some(context.scope.clone()),
        path: Some(path.to_path_buf()),
        status: ImportStatus::Unsupported,
        message: message.into(),
    }
}

pub(super) fn check_privacy(
    filter: &ImportPrivacyFilter,
    context: &ConversionContext,
    kind: ImportKind,
    path: &Path,
) -> Result<(), ImportIssue> {
    filter
        .check_path(path)
        .map_err(|message| privacy_skip_issue(context, kind, path, message))
}

pub(super) fn privacy_skip_issue(
    context: &ConversionContext,
    kind: ImportKind,
    path: &Path,
    message: impl Into<String>,
) -> ImportIssue {
    ImportIssue {
        competitor: Some(context.competitor),
        kind: Some(kind),
        scope: Some(context.scope.clone()),
        path: Some(path.to_path_buf()),
        status: ImportStatus::Unsupported,
        message: format!("privacy blocked import: {}", message.into()),
    }
}
