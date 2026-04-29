use std::sync::Arc;
use chrono::Utc;
use tokio::sync::RwLock;

use crate::buddy::facts::FactStore;
use crate::buddy::types::{
    BuddyFactKind, BuddyPulse, CustomizationPulse, DiagnosticPulse, GitPulse, McpPulse,
    MemoryPulse, ProviderPulse, TaskPulse, TrajectoryPulse,
};
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
            && !d.chat_thinking_model.is_empty()
            && !d.chat_buddy_model.is_empty();
        let available: std::collections::HashSet<&str> =
            caps.chat_models.keys().map(|s| s.as_str()).collect();
        let to_check = [
            d.chat_default_model.as_str(),
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
    let reg = match crate::yaml_configs::customization_registry::get_project_registry(gcx.clone())
        .await
    {
        Some(r) => r,
        None => return pulse,
    };
    pulse.modes = reg.modes.len() as u32;
    pulse.subagents = reg.subagents.len() as u32;
    pulse.commands = reg.toolbox_commands.len() as u32;

    let ext_dirs = crate::ext::config_dirs::get_ext_dirs(gcx).await;
    let skills = crate::ext::skills::load_skill_indices(&ext_dirs).await;
    pulse.skills = skills.len() as u32;
    let hooks = crate::ext::hooks::load_hooks(&ext_dirs).await;
    pulse.hooks = hooks.len() as u32;

    pulse
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
