//! Competitor customization auto-import v1 is always enabled, non-destructive, and
//! reports unsupported rules without importing them. The first supported source
//! families are Claude Code, OpenCode, Kilo Code, and Continue. This skeleton only
//! discovers global and project scopes; later cards add artifact conversion and writes.

#![allow(dead_code)]

use std::sync::Arc;

use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;

pub mod sources;
pub mod types;

use types::{ImportIssue, ImportScope, ImportStatus, ImportSummary};

pub async fn run_global_import(gcx: Arc<ARwLock<GlobalContext>>) -> ImportSummary {
    let refact_config_dir = {
        let gcx_locked = gcx.read().await;
        gcx_locked.config_dir.clone()
    };
    let mut summary = ImportSummary::from_scopes(vec![ImportScope::Global]);
    let Some(home_dir) = home::home_dir() else {
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
    let config_dir = sources::config_root_from_refact_config_dir(&refact_config_dir);
    summary.discovered_sources = sources::discover_global_sources(&home_dir, &config_dir);
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
    let discovered_scopes = sources::discover_project_scopes(&workspace_roots);
    ImportSummary::from_scopes(discovered_scopes)
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
}
