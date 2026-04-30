use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use chrono::Utc;
use tokio::sync::RwLock;

use crate::buddy::facts::FactStore;
use crate::buddy::types::{
    BuddyFactKind, BuddyPulse, CompetitorImportPulse, CustomizationPulse, DiagnosticPulse,
    GitPulse, McpPulse, MemoryPulse, ProviderPulse, TaskPulse, TrajectoryPulse, WorktreePulse,
};
use crate::ext::competitor_import::manifest::{manifest_path_for_scope_root, ImportManifest};
use crate::ext::competitor_import::types::{ImportReport, ImportReportCounts, ImportStatus};
use crate::global_context::GlobalContext;

pub async fn build_pulse(
    gcx: Arc<RwLock<GlobalContext>>,
    project_root: &std::path::Path,
    fact_store: &FactStore,
) -> BuddyPulse {
    let mut p = BuddyPulse::default();
    p.generated_at = Some(Utc::now());

    p.tasks = build_tasks_pulse(gcx.clone(), fact_store).await;
    p.trajectories = build_trajectories_pulse(project_root).await;
    p.memory = build_memory_pulse(project_root, fact_store);
    p.providers = build_providers_pulse(gcx.clone()).await;
    p.mcp = build_mcp_pulse(gcx.clone(), fact_store).await;
    p.customization = build_customization_pulse(gcx.clone()).await;
    p.diagnostics = build_diagnostics_pulse(gcx.clone()).await;
    p.git = build_git_pulse(project_root);
    p.worktrees = build_worktree_pulse(gcx.clone(), project_root).await;

    p
}

async fn build_tasks_pulse(gcx: Arc<RwLock<GlobalContext>>, fact_store: &FactStore) -> TaskPulse {
    let mut pulse = TaskPulse::default();
    let stuck = fact_store.recent(BuddyFactKind::TaskStuck, chrono::Duration::hours(1));
    pulse.stuck = stuck.len() as u32;
    let abandoned = fact_store.recent(BuddyFactKind::TaskAbandoned, chrono::Duration::hours(24));
    pulse.abandoned = abandoned.len() as u32;
    if let Ok(tasks) = crate::tasks::storage::list_tasks(gcx).await {
        pulse.total = tasks.len() as u32;
        for task in &tasks {
            let key = serde_json::to_value(&task.status)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            *pulse.by_status.entry(key).or_insert(0) += 1;
        }
    }
    pulse
}

async fn build_trajectories_pulse(project_root: &std::path::Path) -> TrajectoryPulse {
    let traj_dir = project_root.join(".refact").join("trajectories");
    if !traj_dir.exists() {
        return TrajectoryPulse::default();
    }
    let (total, untitled, oldest) =
        crate::buddy::observers::trajectory_clutter::scan_trajectories_dir(&traj_dir).await;
    TrajectoryPulse {
        total,
        untitled,
        oldest_age_days: oldest,
    }
}

fn build_memory_pulse(project_root: &std::path::Path, fact_store: &FactStore) -> MemoryPulse {
    let mut pulse = MemoryPulse::default();
    let orphan_facts = fact_store.recent(BuddyFactKind::MemoryOrphan, chrono::Duration::hours(24));
    pulse.orphan = orphan_facts.len() as u32;
    let stale_facts = fact_store.recent(
        BuddyFactKind::MemoryStaleConflict,
        chrono::Duration::hours(24),
    );
    pulse.stale_conflicts = stale_facts.len() as u32;
    let knowledge_dir = project_root.join(".refact").join("knowledge");
    if knowledge_dir.exists() {
        if let Ok(rd) = std::fs::read_dir(&knowledge_dir) {
            pulse.total = rd.count() as u32;
        }
    }
    pulse
}

async fn build_providers_pulse(gcx: Arc<RwLock<GlobalContext>>) -> ProviderPulse {
    let mut pulse = ProviderPulse::default();
    let gcx_r = gcx.read().await;
    if let Some(caps) = &gcx_r.caps {
        let d = &caps.defaults;
        pulse.defaults_ok = !d.chat_default_model.is_empty()
            && !d.chat_light_model.is_empty()
            && !d.chat_thinking_model.is_empty()
            && !d.chat_buddy_model.is_empty();
        let available: std::collections::HashSet<&str> =
            caps.chat_models.keys().map(|s| s.as_str()).collect();
        let to_check = [
            d.chat_default_model.as_str(),
            d.chat_light_model.as_str(),
            d.chat_buddy_model.as_str(),
            d.chat_thinking_model.as_str(),
        ];
        for model in to_check {
            if !model.is_empty() && !available.contains(model) {
                pulse.broken_refs += 1;
            }
        }
    }
    pulse
}

async fn build_mcp_pulse(gcx: Arc<RwLock<GlobalContext>>, fact_store: &FactStore) -> McpPulse {
    let mut pulse = McpPulse::default();
    pulse.total = gcx.read().await.integration_sessions.len() as u32;
    let failing = fact_store.recent(
        BuddyFactKind::IntegrationFailing,
        chrono::Duration::hours(4),
    );
    pulse.failing = failing.len() as u32;
    let expiring = fact_store.recent(BuddyFactKind::McpAuthExpired, chrono::Duration::hours(24));
    pulse.auth_expiring = expiring.len() as u32;
    pulse
}

async fn build_customization_pulse(gcx: Arc<RwLock<GlobalContext>>) -> CustomizationPulse {
    let mut pulse = CustomizationPulse::default();
    let (config_dir, project_roots) = competitor_import_roots(gcx.clone()).await;
    pulse.competitor_import = build_competitor_import_pulse(&config_dir, &project_roots).await;

    if let Some(reg) =
        crate::yaml_configs::customization_registry::get_project_registry(gcx.clone()).await
    {
        pulse.modes = reg.modes.len() as u32;
        pulse.subagents = reg.subagents.len() as u32;
        pulse.commands = reg.toolbox_commands.len() as u32;
    }

    let ext_dirs = crate::ext::config_dirs::get_ext_dirs(gcx).await;
    let skills = crate::ext::skills::load_skill_indices(&ext_dirs).await;
    pulse.skills = skills.len() as u32;
    let hooks = crate::ext::hooks::load_hooks(&ext_dirs).await;
    pulse.hooks = hooks.len() as u32;

    pulse
}

async fn competitor_import_roots(gcx: Arc<RwLock<GlobalContext>>) -> (PathBuf, Vec<PathBuf>) {
    let config_dir = {
        let gcx_locked = gcx.read().await;
        gcx_locked.config_dir.clone()
    };
    let project_roots = crate::files_correction::get_project_dirs(gcx).await;
    (config_dir, project_roots)
}

pub(crate) async fn build_competitor_import_pulse(
    refact_config_dir: &Path,
    project_roots: &[PathBuf],
) -> CompetitorImportPulse {
    let mut pulse = CompetitorImportPulse::default();
    if let Some(report) = read_import_report(refact_config_dir).await {
        merge_import_report_into_pulse(&mut pulse, &report);
    }
    for project_root in project_roots {
        if let Some(report) = read_import_report(&project_root.join(".refact")).await {
            merge_import_report_into_pulse(&mut pulse, &report);
        }
    }
    pulse.has_attention_items = pulse.attention_items > 0;
    pulse
}

async fn read_import_report(scope_root: &Path) -> Option<ImportReport> {
    ImportManifest::read_from_path(&manifest_path_for_scope_root(scope_root))
        .await
        .ok()
        .and_then(|manifest| manifest.last_report)
}

fn merge_import_report_into_pulse(pulse: &mut CompetitorImportPulse, report: &ImportReport) {
    if let Some(run_at) = report
        .completed_at
        .as_ref()
        .or(report.generated_at.as_ref())
    {
        if pulse
            .last_run_at
            .as_ref()
            .map(|existing| existing < run_at)
            .unwrap_or(true)
        {
            pulse.last_run_at = Some(run_at.clone());
        }
    }
    pulse.discovered_candidates = pulse
        .discovered_candidates
        .saturating_add(saturating_u32(report.discovered_candidates));
    pulse.created = pulse
        .created
        .saturating_add(saturating_u32(report.status_count(&ImportStatus::Created)));
    pulse.updated = pulse
        .updated
        .saturating_add(saturating_u32(report.status_count(&ImportStatus::Updated)));
    pulse.conflicts = pulse
        .conflicts
        .saturating_add(saturating_u32(report.status_count(&ImportStatus::Conflict)));
    pulse.user_modified = pulse.user_modified.saturating_add(saturating_u32(
        report.status_count(&ImportStatus::UserModified),
    ));
    pulse.unsupported = pulse.unsupported.saturating_add(saturating_u32(
        report.status_count(&ImportStatus::Unsupported),
    ));
    pulse.errors = pulse
        .errors
        .saturating_add(saturating_u32(report.status_count(&ImportStatus::Error)));
    pulse.attention_items = pulse
        .attention_items
        .saturating_add(saturating_u32(report.attention_items()));
    for source in actual_sources_seen(report) {
        *pulse.sources_seen.entry(source).or_insert(0) += 1;
    }
}

fn actual_sources_seen(report: &ImportReport) -> BTreeSet<String> {
    report
        .competitor_counts
        .iter()
        .filter(|(_, counts)| report_counts_have_activity(counts))
        .map(|(competitor, _)| competitor.as_str().to_string())
        .collect()
}

fn report_counts_have_activity(counts: &ImportReportCounts) -> bool {
    counts.discovered > 0
        || counts.created > 0
        || counts.updated > 0
        || counts.unchanged > 0
        || counts.conflicts > 0
        || counts.user_modified > 0
        || counts.unsupported > 0
        || counts.errors > 0
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

async fn build_diagnostics_pulse(gcx: Arc<RwLock<GlobalContext>>) -> DiagnosticPulse {
    let mut pulse = DiagnosticPulse::default();
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    let diagnostics = match lock.as_ref() {
        Some(svc) => svc.recent_diagnostics.clone(),
        None => return pulse,
    };
    drop(lock);

    let hour_ago = Utc::now() - chrono::Duration::hours(1);
    let mut type_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for diag in &diagnostics {
        if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&diag.collected_at) {
            if ts.with_timezone(&Utc) >= hour_ago {
                pulse.last_hour += 1;
                *type_counts.entry(diag.error_type.clone()).or_insert(0) += 1;
            }
        }
    }
    let mut sorted: Vec<(String, u32)> = type_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    pulse.top_error_types = sorted.into_iter().take(3).map(|(t, _)| t).collect();
    pulse
}

async fn build_worktree_pulse(
    gcx: Arc<RwLock<GlobalContext>>,
    project_root: &std::path::Path,
) -> WorktreePulse {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let Ok(service) = crate::worktrees::service::WorktreeService::new(
        cache_dir,
        project_root.to_path_buf(),
    ) else {
        return WorktreePulse::default();
    };
    let Ok(inventory) = service.inspect_worktrees().await else {
        return WorktreePulse::default();
    };
    let summary = inventory.summary;
    WorktreePulse {
        total_registered: saturating_u32(summary.total_registered),
        total_discovered: saturating_u32(summary.total_discovered),
        total: saturating_u32(summary.total),
        clean: saturating_u32(summary.clean),
        dirty: saturating_u32(summary.dirty),
        stale: saturating_u32(summary.stale),
        conflicted: saturating_u32(summary.conflicted),
        shared: saturating_u32(summary.shared),
        abandoned_clean: saturating_u32(summary.abandoned_clean),
        changed_files: saturating_u32(summary.changed_files),
        additions: saturating_u32(summary.additions),
        deletions: saturating_u32(summary.deletions),
        missing_registry_paths: saturating_u32(summary.missing_registry_paths),
        unregistered_cache_dirs: saturating_u32(summary.unregistered_cache_dirs),
        merged_branches: saturating_u32(summary.merged_branches),
        newest_age_hours: summary.newest_age_hours,
        oldest_age_hours: summary.oldest_age_hours,
        disk_usage_bytes: summary.disk_usage_bytes,
    }
}

fn build_git_pulse(project_root: &std::path::Path) -> GitPulse {
    let mut pulse = GitPulse::default();
    let repo = match git2::Repository::open(project_root) {
        Ok(r) => r,
        Err(_) => return pulse,
    };
    if let Ok(statuses) = repo.statuses(None) {
        pulse.uncommitted_files = statuses.len() as u32;
    }
    if let Ok(branches) = repo.branches(None) {
        pulse.branches = branches.count() as u32;
    }
    pulse.diff_lines_4h = compute_diff_lines_4h(&repo).unwrap_or(0);
    pulse
}

fn compute_diff_lines_4h(repo: &git2::Repository) -> Option<u32> {
    let cutoff = (Utc::now() - chrono::Duration::hours(4)).timestamp();
    let mut revwalk = repo.revwalk().ok()?;
    revwalk
        .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .ok()?;
    revwalk.push_head().ok()?;
    let mut lines = 0u32;
    for oid_result in revwalk.take(crate::buddy::observers::git_pressure::MAX_DIFF_COMMITS) {
        let oid = oid_result.ok()?;
        let commit = repo.find_commit(oid).ok()?;
        if commit.time().seconds() < cutoff {
            break;
        }
        let tree = commit.tree().ok()?;
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
        let diff = repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .ok()?;
        let stats = diff.stats().ok()?;
        lines = lines.saturating_add((stats.insertions() + stats.deletions()) as u32);
    }
    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::competitor_import::manifest::write_last_report;
    use crate::ext::competitor_import::types::{
        Competitor, ImportCandidateSummary, ImportKind, ImportOutcome, ImportScope,
        ImportSourceRoot, ImportSummary,
    };

    fn candidate(
        competitor: Competitor,
        kind: ImportKind,
        scope: ImportScope,
        name: &str,
    ) -> ImportCandidateSummary {
        ImportCandidateSummary {
            competitor,
            kind,
            scope,
            source_root: PathBuf::from("/source"),
            source_path: PathBuf::from(format!("/source/{name}.md")),
            dest_name: name.to_string(),
            destination_path: PathBuf::from(format!("commands/{name}.md")),
            metadata: serde_json::Value::Null,
        }
    }

    async fn write_summary_report(scope_root: &Path, mut summary: ImportSummary) {
        summary.mark_completed();
        write_last_report(scope_root, &summary).await.unwrap();
    }

    #[tokio::test]
    async fn competitor_import_pulse_merges_global_and_project_reports() {
        let temp = tempfile::tempdir().unwrap();
        let refact_config = temp.path().join("config").join("refact");
        let workspace = temp.path().join("workspace");
        let mut global = ImportSummary::from_scopes(vec![ImportScope::Global]);
        global.discovered_sources.push(ImportSourceRoot {
            competitor: Competitor::ClaudeCode,
            scope: ImportScope::Global,
            path: temp.path().join(".claude"),
        });
        let global_candidate = candidate(
            Competitor::ClaudeCode,
            ImportKind::Command,
            ImportScope::Global,
            "global",
        );
        global.candidates.push(global_candidate.clone());
        global.add_outcome(ImportOutcome {
            candidate: global_candidate,
            status: ImportStatus::Created,
            message: "created".to_string(),
        });
        write_summary_report(&refact_config, global).await;

        let project_scope = ImportScope::Project {
            root: workspace.clone(),
        };
        let mut project = ImportSummary::from_scopes(vec![project_scope.clone()]);
        project.discovered_sources.push(ImportSourceRoot {
            competitor: Competitor::OpenCode,
            scope: project_scope.clone(),
            path: workspace.join(".opencode"),
        });
        let project_candidate = candidate(
            Competitor::OpenCode,
            ImportKind::Command,
            project_scope.clone(),
            "project",
        );
        project.candidates.push(project_candidate.clone());
        project.add_outcome(ImportOutcome {
            candidate: project_candidate,
            status: ImportStatus::Conflict,
            message: "conflict".to_string(),
        });
        project.add_issue(crate::ext::competitor_import::types::ImportIssue {
            competitor: Some(Competitor::OpenCode),
            kind: Some(ImportKind::Command),
            scope: Some(project_scope),
            path: Some(PathBuf::from("commands/project.md")),
            status: ImportStatus::Error,
            message: "error".to_string(),
        });
        write_summary_report(&workspace.join(".refact"), project).await;

        let pulse = build_competitor_import_pulse(&refact_config, &[workspace]).await;

        assert!(pulse.last_run_at.is_some());
        assert_eq!(pulse.discovered_candidates, 2);
        assert_eq!(pulse.created, 1);
        assert_eq!(pulse.conflicts, 1);
        assert_eq!(pulse.errors, 1);
        assert_eq!(pulse.attention_items, 2);
        assert!(pulse.has_attention_items);
        assert_eq!(pulse.sources_seen.get("claude_code"), Some(&1));
        assert_eq!(pulse.sources_seen.get("opencode"), Some(&1));
    }

    #[tokio::test]
    async fn competitor_import_pulse_sources_seen_uses_actual_activity() {
        let temp = tempfile::tempdir().unwrap();
        let refact_config = temp.path().join("config").join("refact");
        let workspace = temp.path().join("workspace");
        let command_path = workspace.join(".claude").join("commands").join("only.md");
        std::fs::create_dir_all(command_path.parent().unwrap()).unwrap();
        std::fs::write(&command_path, "Only Claude is present.").unwrap();

        crate::ext::competitor_import::run_project_import_with_paths(&[workspace.clone()]).await;
        let pulse = build_competitor_import_pulse(&refact_config, &[workspace]).await;

        assert_eq!(pulse.sources_seen.get("claude_code"), Some(&1));
        assert_eq!(pulse.sources_seen.len(), 1);
        assert!(!pulse.sources_seen.contains_key("opencode"));
        assert!(!pulse.sources_seen.contains_key("kilo_code"));
        assert!(!pulse.sources_seen.contains_key("continue_dev"));
    }

    #[tokio::test]
    async fn missing_competitor_import_manifests_return_default_pulse() {
        let temp = tempfile::tempdir().unwrap();
        let pulse = build_competitor_import_pulse(
            &temp.path().join("config").join("refact"),
            &[temp.path().join("workspace")],
        )
        .await;

        assert!(pulse.last_run_at.is_none());
        assert_eq!(pulse.discovered_candidates, 0);
        assert_eq!(pulse.attention_items, 0);
        assert!(!pulse.has_attention_items);
        assert!(pulse.sources_seen.is_empty());
    }

    #[test]
    fn customization_pulse_deserializes_without_competitor_import_field() {
        let json = r#"{"modes":1,"skills":2,"commands":3,"subagents":4,"hooks":5}"#;

        let pulse: CustomizationPulse = serde_json::from_str(json).unwrap();

        assert_eq!(pulse.modes, 1);
        assert_eq!(pulse.competitor_import, CompetitorImportPulse::default());
    }
}
