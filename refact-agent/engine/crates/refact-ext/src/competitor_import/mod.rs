use std::path::{Path, PathBuf};

pub mod converters;
pub mod manifest;
pub mod markdown;
pub mod sources;
pub mod tools;
pub mod types;
pub mod writer;

use types::{
    ImportCandidate, ImportIssue, ImportPrivacyFilter, ImportReport, ImportReportScopeKind,
    ImportScope, ImportStatus, ImportSummary,
};

pub async fn run_global_import_with_paths(
    refact_config_dir: &Path,
    home_dir: Option<&Path>,
) -> ImportSummary {
    run_global_import_with_paths_and_filter(
        refact_config_dir,
        home_dir,
        &ImportPrivacyFilter::allow_all(),
    )
    .await
}

pub async fn run_global_import_with_paths_and_filter(
    refact_config_dir: &Path,
    home_dir: Option<&Path>,
    filter: &ImportPrivacyFilter,
) -> ImportSummary {
    let scope = ImportScope::Global;
    let mut summary = ImportSummary::from_scopes(vec![scope.clone()]);
    let Some(home_dir) = home_dir else {
        summary.add_issue(ImportIssue {
            competitor: None,
            kind: None,
            scope: Some(ImportScope::Global),
            path: None,
            status: ImportStatus::Error,
            message: "home directory unavailable".to_string(),
        });
        persist_last_report_if_needed(refact_config_dir, &mut summary).await;
        return summary;
    };
    let config_dir = sources::config_root_from_refact_config_dir(refact_config_dir);
    summary.discovered_sources = sources::discover_global_sources(home_dir, &config_dir);
    let mut candidates = Vec::new();

    let (claude_candidates, claude_issues) =
        sources::claude::collect_global_candidates_with_filter(home_dir, refact_config_dir, filter);
    candidates.extend(claude_candidates);
    add_issues(&mut summary, claude_issues);

    let opencode_scan = sources::opencode::scan_global_root_with_filter(
        &config_dir.join("opencode"),
        refact_config_dir,
        filter,
    );
    collect_opencode_scan(&mut summary, &mut candidates, opencode_scan);

    let kilo_scan = sources::kilo::scan_global_root_with_filter(
        home_dir,
        &config_dir,
        refact_config_dir,
        filter,
    );
    collect_opencode_scan(&mut summary, &mut candidates, kilo_scan);

    let continue_staging_root = refact_config_dir
        .join("imports")
        .join("staging")
        .join("continue");
    let continue_scan = sources::continue_dev::scan_global_root_with_filter(
        home_dir,
        &continue_staging_root,
        filter,
    );
    collect_continue_scan(&mut summary, &mut candidates, continue_scan);

    write_candidates_and_merge(refact_config_dir, &scope, &mut summary, &candidates).await;
    persist_last_report_if_needed(refact_config_dir, &mut summary).await;
    summary
}

pub async fn run_project_import_with_paths(workspace_roots: &[PathBuf]) -> ImportSummary {
    run_project_import_with_paths_and_filter(workspace_roots, &ImportPrivacyFilter::allow_all())
        .await
}

pub async fn run_project_import_with_paths_and_filter(
    workspace_roots: &[PathBuf],
    filter: &ImportPrivacyFilter,
) -> ImportSummary {
    let discovered_scopes = sources::discover_project_scopes(workspace_roots);
    let mut summary = ImportSummary::default();

    for scope in discovered_scopes {
        let ImportScope::Project { root } = scope else {
            continue;
        };
        let scope = ImportScope::Project { root: root.clone() };
        let mut scope_summary = ImportSummary::from_scopes(vec![scope.clone()]);
        scope_summary.discovered_sources = sources::discover_project_sources(&root);
        let mut candidates = Vec::new();

        let (claude_candidates, claude_issues) =
            sources::claude::collect_project_candidates_with_filter(&root, filter);
        candidates.extend(claude_candidates);
        add_issues(&mut scope_summary, claude_issues);

        let opencode_scan = sources::opencode::scan_project_root_with_filter(&root, filter);
        collect_opencode_scan(&mut scope_summary, &mut candidates, opencode_scan);

        let kilo_scan = sources::kilo::scan_project_root_with_filter(&root, filter);
        collect_opencode_scan(&mut scope_summary, &mut candidates, kilo_scan);

        let continue_staging_root = root
            .join(".refact")
            .join("imports")
            .join("staging")
            .join("continue");
        let continue_scan = sources::continue_dev::scan_project_root_with_filter(
            &root,
            &continue_staging_root,
            filter,
        );
        collect_continue_scan(&mut scope_summary, &mut candidates, continue_scan);

        let scope_root = root.join(".refact");
        write_candidates_and_merge(&scope_root, &scope, &mut scope_summary, &candidates).await;
        persist_last_report_if_needed(&scope_root, &mut scope_summary).await;
        summary.merge(scope_summary);
    }

    summary
}

pub fn add_issues(summary: &mut ImportSummary, issues: Vec<ImportIssue>) {
    for issue in issues {
        summary.add_issue(issue);
    }
}

pub fn collect_opencode_scan(
    summary: &mut ImportSummary,
    candidates: &mut Vec<ImportCandidate>,
    mut scan: sources::opencode::OpenCodeScan,
) {
    candidates.append(&mut scan.candidates);
    add_issues(summary, scan.issues);
}

pub fn collect_continue_scan(
    summary: &mut ImportSummary,
    candidates: &mut Vec<ImportCandidate>,
    mut scan: sources::continue_dev::ContinueScanResult,
) {
    candidates.append(&mut scan.candidates);
    add_issues(summary, scan.summary.issues);
}

pub async fn write_candidates_and_merge(
    scope_root: &Path,
    scope: &ImportScope,
    summary: &mut ImportSummary,
    candidates: &[ImportCandidate],
) {
    let existing_issues = summary.issues.clone();
    let writer_summary = writer::write_candidates_for_scope_with_issues(
        scope_root,
        scope,
        candidates,
        &existing_issues,
    )
    .await;
    summary.merge(writer_summary);
}

pub async fn persist_last_report_if_needed(scope_root: &Path, summary: &mut ImportSummary) {
    if !should_persist_last_report(scope_root, summary).await {
        return;
    }
    persist_last_report(scope_root, summary).await;
}

async fn should_persist_last_report(scope_root: &Path, summary: &ImportSummary) -> bool {
    has_report_activity(summary)
        || tokio::fs::try_exists(manifest::manifest_path_for_scope_root(scope_root))
            .await
            .unwrap_or(false)
}

fn has_report_activity(summary: &ImportSummary) -> bool {
    !summary.candidates.is_empty()
        || !summary.outcomes.is_empty()
        || !summary.issues.is_empty()
        || !summary.errors.is_empty()
        || !summary.status_counts.is_empty()
}

async fn persist_last_report(scope_root: &Path, summary: &mut ImportSummary) {
    summary.mark_completed();
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

pub fn import_reports_for_runtime_events(summary: &ImportSummary) -> Vec<ImportReport> {
    if summary.discovered_scopes.is_empty() {
        return vec![ImportReport::from_summary(summary)];
    }
    let mut reports = summary
        .discovered_scopes
        .iter()
        .map(|scope| ImportReport::from_summary_for_scope(summary, scope))
        .collect::<Vec<_>>();
    if let Some(report) = unscoped_error_report(summary) {
        reports.push(report);
    }
    reports
}

pub fn unscoped_error_report(summary: &ImportSummary) -> Option<ImportReport> {
    let mut aggregate = ImportSummary {
        generated_at: summary.generated_at.clone(),
        completed_at: summary.completed_at.clone(),
        ..ImportSummary::default()
    };
    for issue in summary
        .issues
        .iter()
        .filter(|issue| issue.scope.is_none() && issue.status == ImportStatus::Error)
    {
        aggregate.add_issue(issue.clone());
    }
    if aggregate.issues.is_empty() {
        None
    } else {
        Some(ImportReport::from_summary(&aggregate))
    }
}

pub fn runtime_scope_label(report: &ImportReport) -> &'static str {
    if let Some(scope) = report.discovered_scopes.first() {
        return match scope {
            ImportScope::Global => "global settings",
            ImportScope::Project { .. } => "project workspace",
        };
    }
    match report.reported_scopes.first().map(|scope| scope.scope_kind) {
        Some(ImportReportScopeKind::Global) => "global settings",
        Some(ImportReportScopeKind::Project) => "project workspace",
        None => "workspace",
    }
}

pub fn runtime_dedupe_key(report: &ImportReport) -> String {
    if let Some(scope) = report.discovered_scopes.first() {
        return match scope {
            ImportScope::Global => "competitor_import:global".to_string(),
            ImportScope::Project { root } => {
                let hash = manifest::hash_string(&root.to_string_lossy());
                format!("competitor_import:project:{}", &hash[..16])
            }
        };
    }
    match report.reported_scopes.first() {
        Some(scope) if scope.scope_kind == ImportReportScopeKind::Global => {
            "competitor_import:global".to_string()
        }
        Some(scope) if scope.scope_kind == ImportReportScopeKind::Project => {
            match scope.scope_id.as_deref() {
                Some(scope_id) => format!("competitor_import:project:{scope_id}"),
                None => "competitor_import:project".to_string(),
            }
        }
        _ => "competitor_import:workspace".to_string(),
    }
}

pub fn plural_suffix(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

pub fn log_import_summary(label: &str, summary: &ImportSummary) {
    if summary.is_empty() {
        tracing::info!("competitor import {label}: no scopes");
        return;
    }
    for scope in &summary.discovered_scopes {
        tracing::info!(
            "competitor import {label} {}: created={} updated={} unchanged={} stale={} conflict={} user_modified={} unsupported={} errors={}",
            scope_label(scope),
            status_count_for_scope(summary, scope, &ImportStatus::Created),
            status_count_for_scope(summary, scope, &ImportStatus::Updated),
            status_count_for_scope(summary, scope, &ImportStatus::Unchanged),
            status_count_for_scope(summary, scope, &ImportStatus::Stale),
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
            "competitor import {label} error: {}",
            format_log_issue(issue)
        );
    }
}

pub fn format_log_issue(issue: &ImportIssue) -> String {
    let mut formatted = String::new();
    if let Some(path) = &issue.path {
        formatted.push_str(&sanitize_log_path(path));
        formatted.push_str(": ");
    }
    formatted.push_str(&types::sanitize_report_message(&issue.message));
    formatted
}

pub fn sanitize_log_path(path: &Path) -> String {
    types::sanitize_report_path_value(&path.to_string_lossy())
}

pub fn scope_label(scope: &ImportScope) -> String {
    match scope {
        ImportScope::Global => "global".to_string(),
        ImportScope::Project { .. } => "project:<redacted>".to_string(),
    }
}

pub fn status_count_for_scope(
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

pub fn issue_matches_outcome(summary: &ImportSummary, issue: &ImportIssue) -> bool {
    summary.outcomes.iter().any(|outcome| {
        issue.status == outcome.status
            && issue.kind == Some(outcome.candidate.kind)
            && issue.scope.as_ref() == Some(&outcome.candidate.scope)
            && issue.path.as_ref() == Some(&outcome.candidate.destination_path)
    })
}
