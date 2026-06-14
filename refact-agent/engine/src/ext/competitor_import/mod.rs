use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::buddy::types::{BuddyControl, BuddyRuntimeEvent};

pub use refact_ext::competitor_import::converters;
pub use refact_ext::competitor_import::manifest;
pub use refact_ext::competitor_import::markdown;
pub use refact_ext::competitor_import::sources;
pub use refact_ext::competitor_import::tools;
pub use refact_ext::competitor_import::types;
pub use refact_ext::competitor_import::writer;

pub use refact_ext::competitor_import::{
    add_issues, collect_continue_scan, collect_opencode_scan, format_log_issue,
    import_reports_for_runtime_events, issue_matches_outcome, log_import_summary,
    persist_last_report_if_needed, plural_suffix, run_global_import_with_paths,
    run_global_import_with_paths_and_filter, run_global_import_with_paths_and_filter_for_source,
    run_project_import_with_paths, run_project_import_with_paths_and_filter,
    run_project_import_with_paths_and_filter_for_source, runtime_dedupe_key, runtime_scope_label,
    sanitize_log_path, scope_label, status_count_for_scope, unscoped_error_report,
    write_candidates_and_merge,
};

use types::{Competitor, ImportIssue, ImportPrivacyFilter, ImportReport, ImportStatus, ImportSummary};

pub async fn run_global_import(app: AppState) -> ImportSummary {
    let refact_config_dir = app.paths.config_dir.clone();
    let privacy_settings = app.workspace.privacy_settings.clone();
    let home_dir = home::home_dir();
    let filter = privacy_filter_from_settings(privacy_settings);
    let summary =
        run_global_import_with_paths_and_filter(&refact_config_dir, home_dir.as_deref(), &filter)
            .await;
    apply_cache_invalidation(app.clone(), &summary).await;
    log_import_summary("global", &summary);
    emit_buddy_import_events(app, &summary).await;
    summary
}

pub async fn run_global_import_for_source(
    app: AppState,
    source: Option<Competitor>,
) -> ImportSummary {
    let refact_config_dir = app.paths.config_dir.clone();
    let privacy_settings = app.workspace.privacy_settings.clone();
    let home_dir = home::home_dir();
    let filter = privacy_filter_from_settings(privacy_settings);
    let summary = run_global_import_with_paths_and_filter_for_source(
        &refact_config_dir,
        home_dir.as_deref(),
        &filter,
        source,
    )
    .await;
    apply_cache_invalidation(app.clone(), &summary).await;
    log_import_summary("global", &summary);
    emit_buddy_import_events(app, &summary).await;
    summary
}

pub async fn run_project_import_for_source(
    app: AppState,
    source: Option<Competitor>,
) -> ImportSummary {
    let workspace_folders = app.workspace.documents_state.workspace_folders.clone();
    let privacy_settings = app.workspace.privacy_settings.clone();
    let workspace_roots_result = workspace_folders
        .lock()
        .map(|workspace_folders| workspace_folders.clone())
        .map_err(|err| err.to_string());
    let workspace_roots = match workspace_roots_result {
        Ok(workspace_roots) => workspace_roots,
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
            emit_buddy_import_events(app, &summary).await;
            return summary;
        }
    };
    let filter = privacy_filter_from_settings(privacy_settings);
    let summary =
        run_project_import_with_paths_and_filter_for_source(&workspace_roots, &filter, source)
            .await;
    apply_cache_invalidation(app.clone(), &summary).await;
    log_import_summary("project", &summary);
    emit_buddy_import_events(app, &summary).await;
    summary
}

pub async fn run_project_import(app: AppState) -> ImportSummary {
    let workspace_folders = app.workspace.documents_state.workspace_folders.clone();
    let privacy_settings = app.workspace.privacy_settings.clone();
    let workspace_roots_result = workspace_folders
        .lock()
        .map(|workspace_folders| workspace_folders.clone())
        .map_err(|err| err.to_string());
    let workspace_roots = match workspace_roots_result {
        Ok(workspace_roots) => workspace_roots,
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
            emit_buddy_import_events(app, &summary).await;
            return summary;
        }
    };
    let filter = privacy_filter_from_settings(privacy_settings);
    let summary = run_project_import_with_paths_and_filter(&workspace_roots, &filter).await;
    apply_cache_invalidation(app.clone(), &summary).await;
    log_import_summary("project", &summary);
    emit_buddy_import_events(app, &summary).await;
    summary
}

fn privacy_filter_from_settings(
    settings: Arc<crate::privacy::PrivacySettings>,
) -> ImportPrivacyFilter {
    ImportPrivacyFilter::with_check(Arc::new(move |path| {
        crate::privacy::check_file_privacy(
            settings.clone(),
            path,
            &crate::privacy::FilePrivacyLevel::AllowToSendAnywhere,
        )
    }))
}

async fn apply_cache_invalidation(app: AppState, summary: &ImportSummary) {
    if !summary.has_imported_changes() {
        return;
    }
    app.integrations
        .ext_cache_generation
        .fetch_add(1, Ordering::Relaxed);
    if summary.has_command_or_skill_changes() {
        crate::http::routers::v1::at_commands::invalidate_slash_cache().await;
    }
    if summary.has_subagent_changes() {
        crate::yaml_configs::customization_registry::invalidate_all_registry_caches(app.gcx).await;
    }
}

async fn emit_buddy_import_events(app: AppState, summary: &ImportSummary) {
    let reports = import_reports_for_runtime_events(summary);
    if reports.is_empty() {
        return;
    }
    let buddy_arc = app.buddy.buddy.clone();
    let mut buddy = buddy_arc.lock().await;
    let Some(service) = buddy.as_mut() else {
        return;
    };
    for report in reports {
        service.enqueue_runtime_event(buddy_runtime_event_for_import_report(&report));
    }
}

pub(crate) fn buddy_runtime_event_for_import_report(report: &ImportReport) -> BuddyRuntimeEvent {
    let created = report.status_count(&ImportStatus::Created);
    let updated = report.status_count(&ImportStatus::Updated);
    let unchanged = report.status_count(&ImportStatus::Unchanged);
    let stale = report.status_count(&ImportStatus::Stale);
    let conflicts = report.status_count(&ImportStatus::Conflict);
    let user_modified = report.status_count(&ImportStatus::UserModified);
    let unsupported = report.status_count(&ImportStatus::Unsupported);
    let errors = report.status_count(&ImportStatus::Error);
    let attention = conflicts + user_modified + errors;
    let status = if errors > 0 {
        "error"
    } else if stale + conflicts + user_modified + unsupported > 0 {
        "warning"
    } else {
        "completed"
    };
    let priority = if attention > 0 {
        "high"
    } else if created + updated > 0 || stale > 0 || unsupported > 0 {
        "normal"
    } else {
        "low"
    };
    let title = if attention > 0 {
        format!("Competitor import needs attention ({attention})")
    } else if created + updated > 0 {
        format!(
            "Competitor import added {} customization{}",
            created + updated,
            plural_suffix(created + updated)
        )
    } else if stale > 0 {
        format!(
            "Competitor import found {} stale generated customization{}",
            stale,
            plural_suffix(stale)
        )
    } else {
        "Competitor import checked customizations".to_string()
    };
    let description = format!(
        "{}: discovered {}, created {}, updated {}, unchanged {}, stale {}, conflicts {}, user-modified {}, unsupported {}, errors {}.",
        runtime_scope_label(report),
        report.discovered_candidates,
        created,
        updated,
        unchanged,
        stale,
        conflicts,
        user_modified,
        unsupported,
        errors
    );
    let mut event = crate::buddy::actor::make_runtime_event(
        "competitor_import",
        &title,
        "competitor_import",
        &runtime_dedupe_key(report),
        status,
        Some(priority),
    );
    event.description = Some(description);
    event.failure_category = None;
    event.failure_summary = None;
    event.persistent = attention > 0;
    event.ttl_ms = if attention > 0 {
        None
    } else if priority == "low" {
        Some(6000)
    } else {
        Some(12000)
    };
    event.speech_text = if attention > 0 { Some(title) } else { None };
    event.controls = vec![
        BuddyControl {
            id: "open-buddy".to_string(),
            label: "Open Buddy".to_string(),
            action: "open_buddy".to_string(),
            action_param: None,
            style: "primary".to_string(),
        },
        BuddyControl {
            id: "dismiss".to_string(),
            label: "Dismiss".to_string(),
            action: "dismiss".to_string(),
            action_param: None,
            style: "secondary".to_string(),
        },
    ];
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;

    use crate::ext::competitor_import::manifest::{manifest_path_for_scope_root, ImportManifest};
    use crate::ext::competitor_import::types::{
        Competitor, ImportCandidateSummary, ImportKind, ImportOutcome, ImportScope,
    };
    use crate::privacy::{FilePrivacySettings, PrivacySettings};
    use crate::yaml_configs::customization_types::SubagentConfig;

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

    fn status_count(summary: &ImportSummary, status: ImportStatus) -> usize {
        summary.status_counts.get(&status).copied().unwrap_or(0)
    }

    fn read_subagent_config(scope_root: &Path, id: &str) -> SubagentConfig {
        let content =
            fs::read_to_string(scope_root.join("subagents").join(format!("{id}.yaml"))).unwrap();
        serde_yaml::from_str(&content).unwrap()
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn privacy_filter(blocked: &[PathBuf]) -> ImportPrivacyFilter {
        let settings = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked: blocked
                    .iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect(),
            },
            loaded_ts: 0,
        });
        ImportPrivacyFilter::with_check(Arc::new(move |path| {
            crate::privacy::check_file_privacy(
                settings.clone(),
                path,
                &crate::privacy::FilePrivacyLevel::AllowToSendAnywhere,
            )
        }))
    }
    #[test]
    fn log_issue_format_redacts_absolute_paths() {
        let issue = ImportIssue {
            competitor: Some(Competitor::ClaudeCode),
            kind: Some(ImportKind::Command),
            scope: Some(ImportScope::Project {
                root: PathBuf::from("/home/user/private-project"),
            }),
            path: Some(PathBuf::from(
                "/home/user/private-project/.claude/commands/secret.md",
            )),
            status: ImportStatus::Error,
            message:
                "failed reading /home/user/private-project/.claude/commands/secret.md and C:\\Users\\me\\secret.md"
                    .to_string(),
        };

        let formatted = format_log_issue(&issue);

        assert!(formatted.contains("<redacted>/.../secret.md"));
        assert!(formatted.contains("<redacted-path>"));
        assert!(!formatted.contains("/home/user"));
        assert!(!formatted.contains("private-project"));
        assert!(!formatted.contains("C:\\Users"));
    }

    #[test]
    fn log_issue_format_keeps_relative_generated_path_readable() {
        let issue = ImportIssue {
            competitor: Some(Competitor::ClaudeCode),
            kind: Some(ImportKind::Command),
            scope: Some(ImportScope::Global),
            path: Some(PathBuf::from("commands/foo.md")),
            status: ImportStatus::Conflict,
            message: "destination exists without import manifest ownership".to_string(),
        };

        let formatted = format_log_issue(&issue);

        assert!(formatted.starts_with("commands/foo.md: "));
        assert!(formatted.contains("destination exists"));
    }

    #[test]
    fn log_scope_label_redacts_project_root() {
        let label = scope_label(&ImportScope::Project {
            root: PathBuf::from("/home/user/private-project"),
        });

        assert_eq!(label, "project:<redacted>");
        assert!(!label.contains("/home/user"));
        assert!(!label.contains("private-project"));
    }

    async fn set_allow_all_privacy(gcx: crate::global_context::SharedGlobalContext) -> AppState {
        let app = AppState::from_gcx(gcx.clone()).await;
        let config_dir = app.paths.config_dir.clone();
        tokio::fs::write(
            config_dir.join("privacy.yaml"),
            "privacy_rules:\n  only_send_to_servers_I_control: []\n  blocked: []\n",
        )
        .await
        .unwrap();
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        AppState::from_gcx(gcx).await
    }

    #[tokio::test]
    async fn project_import_without_workspaces_is_empty_noop() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;

        let summary = run_project_import(app).await;

        assert!(summary.is_empty());
    }

    #[tokio::test]
    async fn empty_project_import_does_not_create_manifest() {
        let workspace = tempfile::tempdir().unwrap();
        let scope_root = workspace.path().join(".refact");

        let summary = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;

        assert_eq!(summary.discovered_scopes.len(), 1);
        assert!(summary.candidates.is_empty());
        assert!(summary.issues.is_empty());
        assert!(summary.outcomes.is_empty());
        assert!(!manifest_path_for_scope_root(&scope_root).exists());
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
        assert_eq!(report.reported_sources.len(), 6);
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
        assert_eq!(report.reported_sources.len(), 5);
        assert_eq!(report.status_counts.get(&ImportStatus::Created), Some(&1));
    }

    #[tokio::test]
    async fn privacy_blocked_claude_command_is_skipped_without_blocking_public_command() {
        let workspace = tempfile::tempdir().unwrap();
        let private_path = workspace.path().join(".claude/commands/private.md");
        let public_path = workspace.path().join(".claude/commands/public.md");
        write(&private_path, "Private command body must not leak.");
        write(&public_path, "Public command body.");
        let filter = privacy_filter(&[private_path.clone()]);

        let summary =
            run_project_import_with_paths_and_filter(&[workspace.path().to_path_buf()], &filter)
                .await;

        assert_eq!(status_count(&summary, ImportStatus::Created), 1);
        assert_eq!(status_count(&summary, ImportStatus::Unsupported), 1);
        assert!(!workspace
            .path()
            .join(".refact/commands/private.md")
            .exists());
        assert_eq!(
            fs::read_to_string(workspace.path().join(".refact/commands/public.md")).unwrap(),
            "Public command body."
        );
        assert!(summary.issues.iter().any(|issue| {
            issue.status == ImportStatus::Unsupported
                && issue.kind == Some(ImportKind::Command)
                && issue.path.as_deref() == Some(private_path.as_path())
        }));
        let report = ImportReport::from_summary(&summary);
        let report_json = serde_json::to_string(&report).unwrap();
        let manifest_json = fs::read_to_string(manifest_path_for_scope_root(
            &workspace.path().join(".refact"),
        ))
        .unwrap();
        assert!(!report_json.contains("Private command body must not leak"));
        assert!(!manifest_json.contains("Private command body must not leak"));
        assert!(!manifest_json.contains(&private_path.to_string_lossy().to_string()));
    }

    #[tokio::test]
    async fn privacy_blocked_existing_import_is_not_reported_stale() {
        let workspace = tempfile::tempdir().unwrap();
        let source_path = workspace.path().join(".claude/commands/private.md");
        let dest_path = workspace.path().join(".refact/commands/private.md");
        write(&source_path, "Private command body.");

        let first = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;
        let filter = privacy_filter(&[source_path.clone()]);
        let blocked =
            run_project_import_with_paths_and_filter(&[workspace.path().to_path_buf()], &filter)
                .await;

        assert_eq!(status_count(&first, ImportStatus::Created), 1);
        assert_eq!(status_count(&blocked, ImportStatus::Unsupported), 1);
        assert_eq!(status_count(&blocked, ImportStatus::Stale), 0);
        assert!(source_path.exists());
        assert_eq!(
            fs::read_to_string(dest_path).unwrap(),
            "Private command body."
        );
    }

    #[tokio::test]
    async fn privacy_blocked_continue_check_is_skipped_without_blocking_public_check() {
        let workspace = tempfile::tempdir().unwrap();
        let blocked_path = workspace.path().join(".continue/checks/security.md");
        let public_path = workspace.path().join(".continue/checks/style.md");
        write(
            &blocked_path,
            "---\nname: Security\ndescription: Security\n---\nBlocked check body.",
        );
        write(
            &public_path,
            "---\nname: Style\ndescription: Style\n---\nPublic check body.",
        );
        let filter = privacy_filter(&[blocked_path.clone()]);

        let summary =
            run_project_import_with_paths_and_filter(&[workspace.path().to_path_buf()], &filter)
                .await;

        assert_eq!(status_count(&summary, ImportStatus::Created), 1);
        assert_eq!(status_count(&summary, ImportStatus::Unsupported), 1);
        assert!(!workspace
            .path()
            .join(".refact/subagents/security.yaml")
            .exists());
        assert!(workspace
            .path()
            .join(".refact/subagents/style.yaml")
            .exists());
        assert!(summary.issues.iter().any(|issue| {
            issue.status == ImportStatus::Unsupported
                && issue.kind == Some(ImportKind::Subagent)
                && issue.path.as_deref() == Some(blocked_path.as_path())
        }));
    }

    #[tokio::test]
    async fn privacy_blocked_skill_support_file_skips_whole_package_without_staging() {
        let workspace = tempfile::tempdir().unwrap();
        let skill_dir = workspace.path().join(".claude/skills/private-skill");
        let blocked_support = skill_dir.join("secret.txt");
        write(
            &skill_dir.join("SKILL.md"),
            "---\nname: Private Skill\n---\nUse private skill.",
        );
        write(&blocked_support, "Blocked supporting file body.");
        write(
            &workspace.path().join(".claude/commands/public.md"),
            "Public command body.",
        );
        let filter = privacy_filter(&[blocked_support]);

        let summary =
            run_project_import_with_paths_and_filter(&[workspace.path().to_path_buf()], &filter)
                .await;

        assert_eq!(status_count(&summary, ImportStatus::Created), 1);
        assert_eq!(status_count(&summary, ImportStatus::Unsupported), 1);
        assert!(!workspace
            .path()
            .join(".refact/skills/private-skill")
            .exists());
        assert!(workspace.path().join(".refact/commands/public.md").exists());
        assert!(!workspace
            .path()
            .join(".refact/imports/staging/claude")
            .exists());
        let report_json = serde_json::to_string(&ImportReport::from_summary(&summary)).unwrap();
        assert!(!report_json.contains("Blocked supporting file body"));
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
    async fn cross_source_same_command_name_first_write_wins_and_conflict_is_reported() {
        let workspace = tempfile::tempdir().unwrap();
        write(
            &workspace.path().join(".claude/commands/review.md"),
            "Claude review.",
        );
        write(
            &workspace.path().join(".opencode/commands/review.md"),
            "OpenCode review.",
        );

        let summary = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;

        assert_eq!(status_count(&summary, ImportStatus::Created), 1);
        assert_eq!(status_count(&summary, ImportStatus::Conflict), 1);
        assert_eq!(
            fs::read_to_string(workspace.path().join(".refact/commands/review.md")).unwrap(),
            "Claude review."
        );
        let review_outcomes = summary
            .outcomes
            .iter()
            .filter(|outcome| outcome.candidate.dest_name == "review")
            .map(|outcome| outcome.status.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            review_outcomes,
            vec![ImportStatus::Created, ImportStatus::Conflict]
        );
        let manifest = read_manifest(&workspace.path().join(".refact")).await;
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].competitor, Competitor::ClaudeCode);
        let summary_json = serde_json::to_string(&summary).unwrap();
        let manifest_json = fs::read_to_string(manifest_path_for_scope_root(
            &workspace.path().join(".refact"),
        ))
        .unwrap();
        assert!(!summary_json.contains("Claude review."));
        assert!(!summary_json.contains("OpenCode review."));
        assert!(!manifest_json.contains("Claude review."));
        assert!(!manifest_json.contains("OpenCode review."));
    }

    #[tokio::test]
    async fn source_change_updates_only_unedited_generated_destination() {
        let workspace = tempfile::tempdir().unwrap();
        let source_path = workspace.path().join(".claude/commands/update.md");
        let dest_path = workspace.path().join(".refact/commands/update.md");
        write(&source_path, "one");

        let first = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;
        write(&source_path, "two");
        let second = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;
        fs::write(&dest_path, "user edit").unwrap();
        write(&source_path, "three");
        let third = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;

        assert_eq!(status_count(&first, ImportStatus::Created), 1);
        assert_eq!(status_count(&second, ImportStatus::Updated), 1);
        assert_eq!(status_count(&third, ImportStatus::UserModified), 1);
        assert_eq!(fs::read_to_string(dest_path).unwrap(), "user edit");
    }

    #[tokio::test]
    async fn global_and_project_imports_with_same_names_do_not_interfere() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let config = temp.path().join("config");
        let refact_config = config.join("refact");
        let workspace = temp.path().join("workspace");
        write(
            &home.join(".claude/commands/shared.md"),
            "Global shared command.",
        );
        write(
            &workspace.join(".claude/commands/shared.md"),
            "Project shared command.",
        );

        let global_summary = run_global_import_with_paths(&refact_config, Some(&home)).await;
        let project_summary = run_project_import_with_paths(&[workspace.clone()]).await;
        let global_repeated = run_global_import_with_paths(&refact_config, Some(&home)).await;

        assert_eq!(status_count(&global_summary, ImportStatus::Created), 1);
        assert_eq!(status_count(&project_summary, ImportStatus::Created), 1);
        assert_eq!(status_count(&global_repeated, ImportStatus::Unchanged), 1);
        assert_eq!(
            fs::read_to_string(refact_config.join("commands/shared.md")).unwrap(),
            "Global shared command."
        );
        assert_eq!(
            fs::read_to_string(workspace.join(".refact/commands/shared.md")).unwrap(),
            "Project shared command."
        );
        assert_eq!(read_manifest(&refact_config).await.entries.len(), 1);
        assert_eq!(
            read_manifest(&workspace.join(".refact"))
                .await
                .entries
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn continue_rules_are_skipped_while_checks_import() {
        let workspace = tempfile::tempdir().unwrap();
        write(
            &workspace.path().join(".continue/rules/security.md"),
            "# Security rules",
        );
        write(
            &workspace.path().join(".continue/checks/security.md"),
            "---\nname: Security Check\ndescription: Review security\n---\nFind issues.",
        );

        let summary = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;

        assert_eq!(status_count(&summary, ImportStatus::Created), 1);
        assert_eq!(status_count(&summary, ImportStatus::Unsupported), 1);
        assert!(workspace
            .path()
            .join(".refact/subagents/security-check.yaml")
            .exists());
        assert!(!workspace.path().join(".refact/rules/security.md").exists());
        assert!(summary.issues.iter().any(|issue| {
            issue.kind == Some(ImportKind::UnsupportedRules)
                && issue
                    .path
                    .as_ref()
                    .is_some_and(|path| path.ends_with(".continue/rules/security.md"))
        }));
    }

    #[tokio::test]
    async fn kilo_legacy_workflows_import_as_commands() {
        let workspace = tempfile::tempdir().unwrap();
        write(
            &workspace.path().join(".kilocode/workflows/deploy.md"),
            "Deploy legacy workflow.",
        );

        let summary = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;

        assert_eq!(status_count(&summary, ImportStatus::Created), 1);
        assert_eq!(
            fs::read_to_string(workspace.path().join(".refact/commands/deploy.md")).unwrap(),
            "Deploy legacy workflow."
        );
        assert!(summary.candidates.iter().any(|candidate| {
            candidate.competitor == Competitor::KiloCode
                && candidate.kind == ImportKind::Command
                && candidate.dest_name == "deploy"
        }));
    }

    #[tokio::test]
    async fn malformed_source_reports_error_without_blocking_other_sources() {
        let workspace = tempfile::tempdir().unwrap();
        write(&workspace.path().join("opencode.jsonc"), "{ command: [ }");
        write(
            &workspace.path().join(".claude/commands/good.md"),
            "Good command.",
        );

        let summary = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;

        assert_eq!(status_count(&summary, ImportStatus::Created), 1);
        assert_eq!(status_count(&summary, ImportStatus::Error), 1);
        assert_eq!(summary.errors.len(), 1);
        assert_eq!(
            fs::read_to_string(workspace.path().join(".refact/commands/good.md")).unwrap(),
            "Good command."
        );
    }

    #[tokio::test]
    async fn generated_subagent_yaml_from_every_importer_parses_and_uses_conservative_tools() {
        let workspace = tempfile::tempdir().unwrap();
        write(
            &workspace.path().join(".claude/agents/claude-agent.md"),
            "---\nname: Claude Agent\ndescription: Claude helper\ntools:\n  - Read\n  - Edit\n  - Bash\n  - UnknownDanger\ndenied-tools:\n  - Bash\nmaxTurns: 4\n---\nHelp from Claude.",
        );
        write(
            &workspace.path().join(".opencode/agents/open-agent.md"),
            "---\ndescription: Open helper\ntools:\n  - read\n  - grep\n  - bash\n  - unknownDanger\npermission:\n  bash: deny\nsteps: 5\n---\nHelp from OpenCode.",
        );
        write(
            &workspace.path().join(".kilo/agents/kilo-agent.md"),
            "---\ndescription: Kilo helper\npermission:\n  edit: allow\n  bash: deny\nsteps: 6\n---\nHelp from Kilo.",
        );
        write(
            &workspace.path().join(".continue/checks/continue-check.md"),
            "---\nname: Continue Check\ndescription: Continue helper\n---\nHelp from Continue.",
        );

        let summary = run_project_import_with_paths(&[workspace.path().to_path_buf()]).await;

        assert_eq!(status_count(&summary, ImportStatus::Created), 4);
        let scope_root = workspace.path().join(".refact");
        let claude = read_subagent_config(&scope_root, "claude-agent");
        let opencode = read_subagent_config(&scope_root, "open-agent");
        let kilo = read_subagent_config(&scope_root, "kilo-agent");
        let continue_check = read_subagent_config(&scope_root, "continue-check");
        assert_eq!(claude.tools, strings(&["cat", "apply_patch"]));
        assert_eq!(opencode.tools, strings(&["cat", "search_pattern"]));
        assert_eq!(kilo.tools, strings(&["apply_patch"]));
        assert_eq!(
            continue_check.tools,
            strings(&["tree", "cat", "search_pattern"])
        );
        for config in [&claude, &opencode, &kilo, &continue_check] {
            assert_eq!(config.schema_version, 2);
            assert!(!config.tools.contains(&"shell".to_string()));
            assert!(!config.tools.contains(&"unknownDanger".to_string()));
            assert!(config.expose_as_tool);
        }
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
        let app = set_allow_all_privacy(gcx.clone()).await;
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
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![workspace.path().to_path_buf()];

        let summary = run_project_import(app.clone()).await;
        let generation_after_first = app
            .integrations
            .ext_cache_generation
            .load(Ordering::Relaxed);
        let repeated = run_project_import(app.clone()).await;
        let generation_after_second = app
            .integrations
            .ext_cache_generation
            .load(Ordering::Relaxed);

        assert!(summary.has_command_or_skill_changes());
        assert!(summary.has_subagent_changes());
        assert_eq!(generation_after_first, 1);
        assert!(!repeated.has_imported_changes());
        assert_eq!(generation_after_second, 1);
    }

    #[tokio::test]
    async fn stale_project_import_does_not_drive_cache_invalidation() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = set_allow_all_privacy(gcx.clone()).await;
        let workspace = tempfile::tempdir().unwrap();
        let source_path = workspace.path().join(".claude/commands/review.md");
        let dest_path = workspace.path().join(".refact/commands/review.md");
        write(&source_path, "Review.");
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![workspace.path().to_path_buf()];

        let first = run_project_import(app.clone()).await;
        fs::remove_file(&source_path).unwrap();
        let stale = run_project_import(app.clone()).await;
        let generation = app
            .integrations
            .ext_cache_generation
            .load(Ordering::Relaxed);

        assert_eq!(status_count(&first, ImportStatus::Created), 1);
        assert_eq!(status_count(&stale, ImportStatus::Stale), 1);
        assert!(!stale.has_imported_changes());
        assert_eq!(generation, 1);
        assert_eq!(fs::read_to_string(dest_path).unwrap(), "Review.");
    }

    fn runtime_event_summary(status: ImportStatus, scope: ImportScope) -> ImportSummary {
        let mut summary = ImportSummary::from_scopes(vec![scope.clone()]);
        let candidate = ImportCandidateSummary {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Command,
            scope,
            source_root: PathBuf::from("/source"),
            source_path: PathBuf::from("/source/secret.md"),
            dest_name: "secret".to_string(),
            destination_path: PathBuf::from("commands/secret.md"),
            metadata: serde_json::json!({"artifact_body": "secret body"}),
        };
        summary.add_outcome(ImportOutcome {
            candidate,
            status,
            message: "generated output changed".to_string(),
        });
        summary.mark_completed();
        summary
    }

    #[test]
    fn runtime_event_for_unchanged_import_is_low_transient() {
        let summary = runtime_event_summary(ImportStatus::Unchanged, ImportScope::Global);
        let report = ImportReport::from_summary(&summary);

        let event = buddy_runtime_event_for_import_report(&report);

        assert_eq!(event.signal_type, "competitor_import");
        assert_eq!(event.source, "competitor_import");
        assert_eq!(event.status, "completed");
        assert_eq!(event.priority, "low");
        assert!(!event.persistent);
        assert_eq!(event.ttl_ms, Some(6000));
        assert_eq!(
            event.dedupe_key.as_deref(),
            Some("competitor_import:global")
        );
    }

    #[test]
    fn runtime_event_for_created_import_is_normal_completed() {
        let summary = runtime_event_summary(ImportStatus::Created, ImportScope::Global);
        let report = ImportReport::from_summary(&summary);

        let event = buddy_runtime_event_for_import_report(&report);

        assert_eq!(event.status, "completed");
        assert_eq!(event.priority, "normal");
        assert!(!event.persistent);
        assert!(event.title.contains("added 1"));
        assert_eq!(event.controls[0].action, "open_buddy");
    }

    #[test]
    fn runtime_event_for_conflicts_and_errors_is_sanitized_attention() {
        let scope = ImportScope::Project {
            root: PathBuf::from("/home/user/private-project"),
        };
        let mut summary = runtime_event_summary(ImportStatus::Conflict, scope);
        summary.add_issue(ImportIssue {
            competitor: Some(Competitor::OpenCode),
            kind: Some(ImportKind::Command),
            scope: summary.discovered_scopes.first().cloned(),
            path: Some(PathBuf::from("commands/other.md")),
            status: ImportStatus::Error,
            message: "failed without leaking body".to_string(),
        });
        summary.mark_completed();
        let report = ImportReport::from_summary(&summary);

        let event = buddy_runtime_event_for_import_report(&report);
        let event_json = serde_json::to_string(&event).unwrap();

        assert_eq!(event.status, "error");
        assert_eq!(event.priority, "high");
        assert!(event.persistent);
        assert!(event.ttl_ms.is_none());
        assert!(event.speech_text.is_some());
        assert!(!event_json.contains("secret body"));
        assert!(!event_json.contains("private-project"));
        assert!(event
            .dedupe_key
            .as_deref()
            .is_some_and(|key| key.starts_with("competitor_import:project:")));
    }

    #[test]
    fn runtime_reports_include_aggregate_unscoped_errors_with_scopes() {
        let scope = ImportScope::Project {
            root: PathBuf::from("/home/user/private-project"),
        };
        let mut summary = ImportSummary::from_scopes(vec![scope]);
        summary.add_issue(ImportIssue {
            competitor: None,
            kind: None,
            scope: None,
            path: None,
            status: ImportStatus::Error,
            message: "workspace folders unavailable".to_string(),
        });
        summary.mark_completed();

        let reports = import_reports_for_runtime_events(&summary);
        let events = reports
            .iter()
            .map(buddy_runtime_event_for_import_report)
            .collect::<Vec<_>>();

        assert_eq!(reports.len(), 2);
        assert!(events.iter().any(|event| {
            event.status == "error"
                && event.dedupe_key.as_deref() == Some("competitor_import:workspace")
        }));
    }

    #[tokio::test]
    async fn runtime_event_emit_is_noop_without_buddy_service() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let summary = runtime_event_summary(ImportStatus::Created, ImportScope::Global);

        emit_buddy_import_events(app.clone(), &summary).await;

        assert!(app.buddy.buddy.lock().await.is_none());
    }

    #[tokio::test]
    async fn runtime_event_emit_enqueues_when_buddy_service_exists() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let service = crate::buddy::actor::BuddyService::new(
            std::env::temp_dir().join(format!("buddy-import-test-{}", uuid::Uuid::new_v4())),
            crate::buddy::state::default_buddy_state(),
            crate::buddy::settings::BuddySettings::default(),
            Vec::new(),
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.buddy.buddy.lock().await = Some(service);
        let summary = runtime_event_summary(ImportStatus::Created, ImportScope::Global);

        emit_buddy_import_events(app.clone(), &summary).await;

        let buddy_arc = app.buddy.buddy.clone();
        let lock = buddy_arc.lock().await;
        let service = lock.as_ref().unwrap();
        assert_eq!(service.runtime_queue.items.len(), 1);
        assert_eq!(
            service.runtime_queue.items[0].signal_type,
            "competitor_import"
        );
    }

    #[tokio::test]
    async fn cache_invalidation_ignores_unchanged_outcomes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
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

        apply_cache_invalidation(app.clone(), &summary).await;

        assert_eq!(
            app.integrations
                .ext_cache_generation
                .load(Ordering::Relaxed),
            0
        );
    }
}
