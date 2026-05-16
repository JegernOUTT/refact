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
        ImportScope::Project { root } => project_scan_root_allowed(path, root),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_discovery_uses_explicit_home_and_config_paths() {
        let home = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();

        let sources = discover_global_sources(home.path(), config.path());
        let paths = sources
            .iter()
            .map(|source| source.path.clone())
            .collect::<Vec<_>>();

        assert_eq!(sources.len(), 6);
        assert!(paths.contains(&home.path().join(".claude")));
        assert!(paths.contains(&config.path().join("opencode")));
        assert!(paths.contains(&config.path().join("kilo")));
        assert!(paths.contains(&home.path().join(".kilo")));
        assert!(paths.contains(&home.path().join(".kilocode")));
        assert!(paths.contains(&home.path().join(".continue")));
        assert!(sources
            .iter()
            .all(|source| source.scope == ImportScope::Global));
    }

    #[test]
    fn project_discovery_returns_no_scopes_without_workspaces() {
        assert!(discover_project_scopes(&[]).is_empty());
    }

    #[test]
    fn project_discovery_returns_one_scope_per_workspace() {
        let root_a = PathBuf::from("/workspace/a");
        let root_b = PathBuf::from("/workspace/b");
        let canonical_a = normalize_project_root(&root_a);
        let canonical_b = normalize_project_root(&root_b);
        let scopes = discover_project_scopes(&[root_a.clone(), root_b.clone(), root_a]);

        assert_eq!(
            scopes,
            vec![
                ImportScope::Project { root: canonical_a },
                ImportScope::Project { root: canonical_b }
            ]
        );
    }

    #[test]
    fn project_discovery_normalizes_equivalent_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let mut roots = vec![
            root.clone(),
            root.join("."),
            PathBuf::from(format!("{}/", root.display())),
        ];
        #[cfg(unix)]
        {
            let link = temp.path().join("repo-link");
            std::os::unix::fs::symlink(&root, &link).unwrap();
            roots.push(link);
        }

        let scopes = discover_project_scopes(&roots);

        assert_eq!(
            scopes,
            vec![ImportScope::Project {
                root: normalize_project_root(&root)
            }]
        );
    }

    #[test]
    fn project_source_discovery_lists_supported_roots() {
        let root = PathBuf::from("/workspace/project");

        let sources = discover_project_sources(&root);

        assert_eq!(sources.len(), 5);
        assert!(sources
            .iter()
            .any(|source| source.path == root.join(".claude")));
        assert!(sources
            .iter()
            .any(|source| source.path == root.join(".opencode")));
        assert!(sources
            .iter()
            .any(|source| source.path == root.join(".kilo")));
        assert!(sources
            .iter()
            .any(|source| source.path == root.join(".kilocode")));
        assert!(sources
            .iter()
            .any(|source| source.path == root.join(".continue")));
    }

    #[test]
    fn config_root_uses_parent_of_refact_config_dir() {
        let config_root = PathBuf::from("/home/user/.config");
        let refact_config = config_root.join("refact");

        assert_eq!(
            config_root_from_refact_config_dir(&refact_config),
            config_root
        );
    }
}
