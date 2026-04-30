use std::sync::Arc;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::{BuddyControl, BuddySuggestion};

pub struct ProactiveSuggestionsJob;

#[async_trait::async_trait]
impl BuddyJob for ProactiveSuggestionsJob {
    fn id(&self) -> &str {
        "proactive_suggestions"
    }
    fn cooldown_seconds(&self) -> u64 {
        3600
    }
    fn priority(&self) -> u32 {
        6
    }
    fn produces_suggestion(&self) -> bool {
        true
    }

    async fn should_run(
        &self,
        _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        _ctx: &BuddyJobContext,
    ) -> bool {
        true
    }

    async fn execute(
        &self,
        gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        if ctx.job_state.run_count == 0 {
            return BuddyJobResult::default();
        }

        if let Some(suggestion) = worktree_cleanup_suggestion(gcx, &ctx.project_root).await {
            return BuddyJobResult {
                suggestion: Some(suggestion),
                last_result: Some("worktree_cleanup".to_string()),
                ..Default::default()
            };
        }

        let project_root = ctx.project_root.clone();
        let file_count =
            tokio::task::spawn_blocking(move || count_uncommitted_changes(&project_root))
                .await
                .unwrap_or(None);

        let Some(count) = file_count else {
            return BuddyJobResult::default();
        };

        if count >= 10 {
            return BuddyJobResult {
                suggestion: Some(BuddySuggestion {
                    id: format!("git-uncommitted-{}", chrono::Utc::now().timestamp()),
                    suggestion_type: "git_commit".to_string(),
                    title: format!("{} uncommitted files", count),
                    description:
                        "You have many uncommitted changes. Want me to generate a commit message?"
                            .to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    dismissed: false,
                    controls: vec![],
                    quest: None,
                }),
                ..Default::default()
            };
        }

        BuddyJobResult::default()
    }
}

async fn worktree_cleanup_suggestion(
    gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
    project_root: &std::path::Path,
) -> Option<BuddySuggestion> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let service =
        crate::worktrees::service::WorktreeService::new(cache_dir, project_root.to_path_buf())
            .ok()?;
    let inventory = service.inspect_worktrees().await.ok()?;
    worktree_hygiene_suggestion_from_inventory(&inventory)
}

pub fn worktree_hygiene_suggestion_from_inventory(
    inventory: &crate::worktrees::types::WorktreeInventory,
) -> Option<BuddySuggestion> {
    if inventory.summary.abandoned_clean == 0 {
        return None;
    }
    let id = format!("worktree-cleanup-{}", chrono::Utc::now().timestamp());
    let description = format!(
        "I found {} worktrees: {} clean abandoned, {} with changes, {} stale. Want to review cleanup candidates?",
        inventory.summary.total,
        inventory.summary.abandoned_clean,
        inventory.summary.dirty,
        inventory.summary.stale
    );
    Some(BuddySuggestion {
        id: id.clone(),
        suggestion_type: "worktree_cleanup".to_string(),
        title: format!(
            "{} clean abandoned worktrees",
            inventory.summary.abandoned_clean
        ),
        description,
        created_at: chrono::Utc::now().to_rfc3339(),
        dismissed: false,
        controls: vec![
            BuddyControl {
                id: "open-worktrees".to_string(),
                label: "Open Worktrees view".to_string(),
                action: "open_worktrees".to_string(),
                action_param: None,
                style: "primary".to_string(),
            },
            BuddyControl {
                id: "review-worktree-cleanup".to_string(),
                label: "Review cleanup candidates".to_string(),
                action: "review_worktree_cleanup".to_string(),
                action_param: None,
                style: "secondary".to_string(),
            },
            BuddyControl {
                id: "clean-selected-worktrees".to_string(),
                label: "Clean selected clean abandoned worktrees".to_string(),
                action: "open_worktree_cleanup".to_string(),
                action_param: None,
                style: "secondary".to_string(),
            },
            BuddyControl {
                id: "worktree-pulse".to_string(),
                label: "Create Worktrees pulse report".to_string(),
                action: "create_worktrees_pulse".to_string(),
                action_param: None,
                style: "secondary".to_string(),
            },
            BuddyControl {
                id: "dismiss-worktree-cleanup".to_string(),
                label: "Dismiss".to_string(),
                action: "dismiss_suggestion".to_string(),
                action_param: Some(id),
                style: "ghost".to_string(),
            },
        ],
        quest: None,
    })
}

fn count_uncommitted_changes(project_root: &std::path::Path) -> Option<usize> {
    use git2::{Repository, StatusOptions, StatusShow};
    let repo = Repository::open(project_root).ok()?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .show(StatusShow::IndexAndWorkdir);
    let statuses = repo.statuses(Some(&mut opts)).ok()?;
    Some(statuses.iter().filter(|s| !s.status().is_empty()).count())
}
