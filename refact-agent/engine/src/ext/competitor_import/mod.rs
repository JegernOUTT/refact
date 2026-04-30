#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;

pub mod converters;
pub mod manifest;
pub mod markdown;
pub mod sources;
pub mod tools;
pub mod types;
pub mod writer;

use types::{Competitor, ImportIssue, ImportScope, ImportSourceRoot, ImportStatus, ImportSummary};

pub async fn run_global_import(gcx: Arc<ARwLock<GlobalContext>>) -> ImportSummary {
    let refact_config_dir = {
        let gcx_locked = gcx.read().await;
        gcx_locked.config_dir.clone()
    };
    let home_dir = home::home_dir();
    run_global_import_with_paths(&refact_config_dir, home_dir.as_deref()).await
}

pub(crate) async fn run_global_import_with_paths(
    refact_config_dir: &Path,
    home_dir: Option<&Path>,
) -> ImportSummary {
    let mut summary = ImportSummary::from_scopes(vec![ImportScope::Global]);
    let Some(home_dir) = home_dir else {
        summary.add_issue(ImportIssue {
            competitor: None,
            kind: None,
            scope: Some(ImportScope::Global),
            path: None,
            status: ImportStatus::Error,
            message: "home directory unavailable".to_string(),
        });
        return summary;
    };
    let config_dir = sources::config_root_from_refact_config_dir(refact_config_dir);
    summary.discovered_sources = sources::discover_global_sources(home_dir, &config_dir);
    let (candidates, issues) =
        sources::claude::collect_global_candidates(home_dir, refact_config_dir);
    for issue in issues {
        summary.add_issue(issue);
    }
    if !candidates.is_empty() {
        summary.merge(writer::write_candidates(refact_config_dir, &candidates).await);
    }
    summary
}

pub async fn run_project_import(gcx: Arc<ARwLock<GlobalContext>>) -> ImportSummary {
    let workspace_folders = {
        let gcx_locked = gcx.read().await;
        gcx_locked.documents_state.workspace_folders.clone()
    };
    let workspace_roots = match workspace_folders.lock() {
        Ok(workspace_folders) => workspace_folders.clone(),
        Err(err) => {
            let mut summary = ImportSummary::default();
            summary.add_issue(ImportIssue {
                competitor: None,
                kind: None,
                scope: None,
                path: None,
                status: ImportStatus::Error,
                message: format!("workspace folders unavailable: {err}"),
            });
            return summary;
        }
    };
    run_project_import_with_paths(&workspace_roots).await
}

pub(crate) async fn run_project_import_with_paths(workspace_roots: &[PathBuf]) -> ImportSummary {
    let discovered_scopes = sources::discover_project_scopes(workspace_roots);
    let mut summary = ImportSummary::from_scopes(discovered_scopes.clone());

    for scope in discovered_scopes {
        let ImportScope::Project { root } = scope else {
            continue;
        };
        let scope = ImportScope::Project { root: root.clone() };
        summary.discovered_sources.push(ImportSourceRoot {
            competitor: Competitor::ClaudeCode,
            scope: scope.clone(),
            path: root.join(".claude"),
        });
        let (candidates, issues) = sources::claude::collect_project_candidates(&root);
        for issue in issues {
            summary.add_issue(issue);
        }
        if !candidates.is_empty() {
            summary.merge(writer::write_candidates(&root.join(".refact"), &candidates).await);
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn project_import_without_workspaces_is_empty_noop() {
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let summary = run_project_import(gcx).await;

        assert!(summary.is_empty());
    }

    #[tokio::test]
    async fn global_import_helper_uses_injected_home_and_config_paths() {
        let home = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let refact_config = config.path().join("refact");

        let summary = run_global_import_with_paths(&refact_config, Some(home.path())).await;

        assert_eq!(summary.discovered_scopes, vec![ImportScope::Global]);
        assert_eq!(summary.discovered_sources.len(), 6);
        assert!(summary
            .discovered_sources
            .iter()
            .any(|source| source.path == home.path().join(".claude")));
        assert!(summary
            .discovered_sources
            .iter()
            .any(|source| source.path == config.path().join("opencode")));
    }

    #[tokio::test]
    async fn global_import_helper_reports_missing_home_without_mutating_paths() {
        let config = tempfile::tempdir().unwrap();

        let summary = run_global_import_with_paths(&config.path().join("refact"), None).await;

        assert_eq!(summary.errors.len(), 1);
        assert!(summary.discovered_sources.is_empty());
    }
}
