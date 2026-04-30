#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;

pub mod converters;
pub mod manifest;
pub mod markdown;
pub mod sources;
pub mod tools;
pub mod types;
pub mod writer;

use types::{ImportCandidate, ImportIssue, ImportScope, ImportStatus, ImportSummary};

pub async fn run_global_import(gcx: Arc<ARwLock<GlobalContext>>) -> ImportSummary {
    let refact_config_dir = {
        let gcx_locked = gcx.read().await;
        gcx_locked.config_dir.clone()
    };
    let home_dir = home::home_dir();
    let summary = run_global_import_with_paths(&refact_config_dir, home_dir.as_deref()).await;
    apply_cache_invalidation(gcx, &summary).await;
    log_import_summary("global", &summary);
    summary
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
        persist_last_report(refact_config_dir, &mut summary).await;
        return summary;
    };
    let config_dir = sources::config_root_from_refact_config_dir(refact_config_dir);
    summary.discovered_sources = sources::discover_global_sources(home_dir, &config_dir);
    let mut candidates = Vec::new();

    let (claude_candidates, claude_issues) =
        sources::claude::collect_global_candidates(home_dir, refact_config_dir);
    candidates.extend(claude_candidates);
    add_issues(&mut summary, claude_issues);

    let opencode_scan =
        sources::opencode::scan_global_root(&config_dir.join("opencode"), refact_config_dir);
    collect_opencode_scan(&mut summary, &mut candidates, opencode_scan);

    let kilo_scan = sources::kilo::scan_global_root(home_dir, &config_dir, refact_config_dir);
    collect_opencode_scan(&mut summary, &mut candidates, kilo_scan);

    let continue_staging_root = refact_config_dir
        .join("imports")
        .join("staging")
        .join("continue");
    let continue_scan = sources::continue_dev::scan_global_root(home_dir, &continue_staging_root);
    collect_continue_scan(&mut summary, &mut candidates, continue_scan);

    write_candidates_and_merge(refact_config_dir, &mut summary, &candidates).await;
    persist_last_report(refact_config_dir, &mut summary).await;
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
            log_import_summary("project", &summary);
            return summary;
        }
    };
    let summary = run_project_import_with_paths(&workspace_roots).await;
    apply_cache_invalidation(gcx, &summary).await;
    log_import_summary("project", &summary);
    summary
}

pub(crate) async fn run_project_import_with_paths(workspace_roots: &[PathBuf]) -> ImportSummary {
    let discovered_scopes = sources::discover_project_scopes(workspace_roots);
    let mut summary = ImportSummary::default();

    for scope in discovered_scopes {
        let ImportScope::Project { root } = scope else {
            continue;
        };
        let scope = ImportScope::Project { root: root.clone() };
        let mut scope_summary = ImportSummary::from_scopes(vec![scope]);
        scope_summary.discovered_sources = sources::discover_project_sources(&root);
        let mut candidates = Vec::new();

        let (claude_candidates, claude_issues) = sources::claude::collect_project_candidates(&root);
        candidates.extend(claude_candidates);
        add_issues(&mut scope_summary, claude_issues);

        let opencode_scan = sources::opencode::scan_project_root(&root);
        collect_opencode_scan(&mut scope_summary, &mut candidates, opencode_scan);

        let kilo_scan = sources::kilo::scan_project_root(&root);
        collect_opencode_scan(&mut scope_summary, &mut candidates, kilo_scan);

        let continue_staging_root = root
            .join(".refact")
            .join("imports")
            .join("staging")
            .join("continue");
        let continue_scan = sources::continue_dev::scan_project_root(&root, &continue_staging_root);
        collect_continue_scan(&mut scope_summary, &mut candidates, continue_scan);

        let scope_root = root.join(".refact");
        write_candidates_and_merge(&scope_root, &mut scope_summary, &candidates).await;
        persist_last_report(&scope_root, &mut scope_summary).await;
        summary.merge(scope_summary);
    }

    summary
}

fn add_issues(summary: &mut ImportSummary, issues: Vec<ImportIssue>) {
    for issue in issues {
        summary.add_issue(issue);
    }
}

fn collect_opencode_scan(
    summary: &mut ImportSummary,
    candidates: &mut Vec<ImportCandidate>,
    mut scan: sources::opencode::OpenCodeScan,
) {
    candidates.append(&mut scan.candidates);
    add_issues(summary, scan.issues);
}

fn collect_continue_scan(
    summary: &mut ImportSummary,
    candidates: &mut Vec<ImportCandidate>,
    mut scan: sources::continue_dev::ContinueScanResult,
) {
    candidates.append(&mut scan.candidates);
    add_issues(summary, scan.summary.issues);
}

async fn write_candidates_and_merge(
    scope_root: &Path,
    summary: &mut ImportSummary,
    candidates: &[ImportCandidate],
) {
    if candidates.is_empty() {
        return;
    }
    summary.merge(writer::write_candidates(scope_root, candidates).await);
}

async fn persist_last_report(scope_root: &Path, summary: &mut ImportSummary) {
    if let Err(err) = manifest::write_last_report(scope_root, summary).await {
        summary.add_issue(ImportIssue {
            competitor: None,
            kind: None,
            scope: None,
            path: Some(manifest::manifest_path_for_scope_root(scope_root)),
            status: ImportStatus::Error,
            message: format!("failed to write import report: {err}"),
        });
    }
}

async fn apply_cache_invalidation(gcx: Arc<ARwLock<GlobalContext>>, summary: &ImportSummary) {
    if !summary.has_imported_changes() {
        return;
    }
    let generation = {
        let gcx_locked = gcx.read().await;
        gcx_locked.ext_cache_generation.clone()
    };
    generation.fetch_add(1, Ordering::Relaxed);
    if summary.has_command_or_skill_changes() {
        crate::http::routers::v1::at_commands::invalidate_slash_cache().await;
    }
    if summary.has_subagent_changes() {
        crate::yaml_configs::customization_registry::invalidate_all_registry_caches(gcx).await;
    }
}

fn log_import_summary(label: &str, summary: &ImportSummary) {
    if summary.is_empty() {
        tracing::info!("competitor import {label}: no scopes");
        return;
    }
    for scope in &summary.discovered_scopes {
        tracing::info!(
            "competitor import {label} {}: created={} updated={} unchanged={} conflict={} user_modified={} unsupported={} errors={}",
            scope_label(scope),
            status_count_for_scope(summary, scope, &ImportStatus::Created),
            status_count_for_scope(summary, scope, &ImportStatus::Updated),
            status_count_for_scope(summary, scope, &ImportStatus::Unchanged),
            status_count_for_scope(summary, scope, &ImportStatus::Conflict),
            status_count_for_scope(summary, scope, &ImportStatus::UserModified),
            status_count_for_scope(summary, scope, &ImportStatus::Unsupported),
            status_count_for_scope(summary, scope, &ImportStatus::Error),
        );
    }
    let unscoped_errors = summary
        .errors
        .iter()
        .filter(|issue| issue.scope.is_none())
        .count();
    if unscoped_errors > 0 {
        tracing::info!("competitor import {label}: unscoped_errors={unscoped_errors}");
    }
    for issue in summary.errors.iter().take(5) {
        tracing::warn!(
            "competitor import {label} error: {}{}",
            issue
                .path
                .as_ref()
                .map(|path| format!("{}: ", path.display()))
                .unwrap_or_default(),
            issue.message
        );
    }
}

fn scope_label(scope: &ImportScope) -> String {
    match scope {
        ImportScope::Global => "global".to_string(),
        ImportScope::Project { root } => format!("project:{}", root.display()),
    }
}

fn status_count_for_scope(
    summary: &ImportSummary,
    scope: &ImportScope,
    status: &ImportStatus,
) -> usize {
    let outcome_count = summary
        .outcomes
        .iter()
        .filter(|outcome| &outcome.candidate.scope == scope && &outcome.status == status)
        .count();
    let issue_count = summary
        .issues
        .iter()
        .filter(|issue| issue.scope.as_ref() == Some(scope) && &issue.status == status)
        .filter(|issue| !issue_matches_outcome(summary, issue))
        .count();
    outcome_count + issue_count
}

fn issue_matches_outcome(summary: &ImportSummary, issue: &ImportIssue) -> bool {
    summary.outcomes.iter().any(|outcome| {
        issue.status == outcome.status
            && issue.kind == Some(outcome.candidate.kind)
            && issue.scope.as_ref() == Some(&outcome.candidate.scope)
            && issue.path.as_ref() == Some(&outcome.candidate.destination_path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::ext::competitor_import::manifest::{manifest_path_for_scope_root, ImportManifest};
    use crate::ext::competitor_import::types::{
        Competitor, ImportCandidateSummary, ImportKind, ImportOutcome,
    };

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    async fn read_manifest(scope_root: &Path) -> ImportManifest {
        ImportManifest::read_from_path(&manifest_path_for_scope_root(scope_root))
            .await
            .unwrap()
    }

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

    #[tokio::test]
    async fn global_import_writes_to_injected_refact_config_dir() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let config = temp.path().join("config");
        let refact_config = config.join("refact");
        write(
            &home.join(".claude").join("commands").join("global.md"),
            "Run globally.",
        );
        write(
            &config.join("opencode").join("commands").join("open.md"),
            "Open globally.",
        );

        let summary = run_global_import_with_paths(&refact_config, Some(&home)).await;

        assert_eq!(summary.status_counts.get(&ImportStatus::Created), Some(&2));
        assert_eq!(
            fs::read_to_string(refact_config.join("commands").join("global.md")).unwrap(),
            "Run globally."
        );
        assert_eq!(
            fs::read_to_string(refact_config.join("commands").join("open.md")).unwrap(),
            "Open globally."
        );
        let manifest = read_manifest(&refact_config).await;
        let report = manifest.last_report.unwrap();
        assert_eq!(report.discovered_sources.len(), 6);
        assert_eq!(report.status_counts.get(&ImportStatus::Created), Some(&2));
    }

    #[tokio::test]
    async fn project_import_writes_to_workspace_refact_dir() {
        let workspace = tempfile::tempdir().unwrap();
        write(
            &workspace
                .path()
                .join(".opencode")
                .join("commands")
                .join("review.md"),
            "Review project.",
        );

        let summary = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;

        assert_eq!(summary.status_counts.get(&ImportStatus::Created), Some(&1));
        assert_eq!(
            fs::read_to_string(workspace.path().join(".refact/commands/review.md")).unwrap(),
            "Review project."
        );
        let manifest = read_manifest(&workspace.path().join(".refact")).await;
        let report = manifest.last_report.unwrap();
        assert_eq!(report.discovered_sources.len(), 5);
        assert_eq!(report.status_counts.get(&ImportStatus::Created), Some(&1));
    }

    #[tokio::test]
    async fn project_imports_multiple_workspaces_independently() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        write(
            &first
                .path()
                .join(".claude")
                .join("commands")
                .join("review.md"),
            "Review first.",
        );
        write(
            &second
                .path()
                .join(".continue")
                .join("prompts")
                .join("deploy.md"),
            "---\nname: Deploy\ndescription: Deploy\ninvokable: true\n---\nDeploy second.",
        );

        let summary = run_project_import_with_paths(&[
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ])
        .await;

        assert_eq!(summary.discovered_scopes.len(), 2);
        assert_eq!(summary.status_counts.get(&ImportStatus::Created), Some(&2));
        assert_eq!(
            fs::read_to_string(first.path().join(".refact/commands/review.md")).unwrap(),
            "Review first."
        );
        assert_eq!(
            fs::read_to_string(second.path().join(".refact/commands/deploy.md")).unwrap(),
            "---\ndescription: Deploy\n---\nDeploy second."
        );
    }

    #[tokio::test]
    async fn repeated_project_import_is_idempotent() {
        let workspace = tempfile::tempdir().unwrap();
        write(
            &workspace
                .path()
                .join(".kilo")
                .join("commands")
                .join("review.md"),
            "Review once.",
        );

        let first = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;
        let second = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;

        assert_eq!(first.status_counts.get(&ImportStatus::Created), Some(&1));
        assert_eq!(second.status_counts.get(&ImportStatus::Unchanged), Some(&1));
        assert!(!second.has_imported_changes());
    }

    #[tokio::test]
    async fn import_changes_drive_cache_invalidation_flags() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        write(
            &workspace
                .path()
                .join(".claude")
                .join("commands")
                .join("review.md"),
            "Review.",
        );
        write(
            &workspace
                .path()
                .join(".claude")
                .join("agents")
                .join("reviewer.md"),
            "---\nname: Reviewer\ndescription: Reviews code\n---\nReview code.",
        );
        {
            let gcx_locked = gcx.read().await;
            *gcx_locked.documents_state.workspace_folders.lock().unwrap() =
                vec![workspace.path().to_path_buf()];
        }

        let summary = run_project_import(gcx.clone()).await;
        let generation_after_first = gcx
            .read()
            .await
            .ext_cache_generation
            .load(Ordering::Relaxed);
        let repeated = run_project_import(gcx.clone()).await;
        let generation_after_second = gcx
            .read()
            .await
            .ext_cache_generation
            .load(Ordering::Relaxed);

        assert!(summary.has_command_or_skill_changes());
        assert!(summary.has_subagent_changes());
        assert_eq!(generation_after_first, 1);
        assert!(!repeated.has_imported_changes());
        assert_eq!(generation_after_second, 1);
    }

    #[tokio::test]
    async fn cache_invalidation_ignores_unchanged_outcomes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut summary = ImportSummary::default();
        summary.add_outcome(ImportOutcome {
            candidate: ImportCandidateSummary {
                competitor: Competitor::ClaudeCode,
                kind: ImportKind::Command,
                scope: ImportScope::Global,
                source_root: PathBuf::from("/source"),
                source_path: PathBuf::from("/source/review.md"),
                dest_name: "review".to_string(),
                destination_path: PathBuf::from("/dest/review.md"),
                metadata: serde_json::Value::Null,
            },
            status: ImportStatus::Unchanged,
            message: "unchanged".to_string(),
        });

        apply_cache_invalidation(gcx.clone(), &summary).await;

        assert_eq!(
            gcx.read()
                .await
                .ext_cache_generation
                .load(Ordering::Relaxed),
            0
        );
    }
}
