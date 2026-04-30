use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::sync::RwLock;

use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::global_context::GlobalContext;

use super::{BuddyObserver, ObserverContext};

pub struct WorktreeHygieneObserver;

#[async_trait::async_trait]
impl BuddyObserver for WorktreeHygieneObserver {
    fn id(&self) -> &'static str {
        "worktree_hygiene"
    }

    fn cadence_seconds(&self) -> u64 {
        300
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.git_pressure
    }

    async fn observe(
        &self,
        gcx: Arc<RwLock<GlobalContext>>,
        ctx: &ObserverContext,
    ) -> Vec<BuddyFact> {
        let cache_dir = gcx.read().await.cache_dir.clone();
        let Ok(service) =
            crate::worktrees::service::WorktreeService::new(cache_dir, ctx.project_root.clone())
        else {
            return Vec::new();
        };
        let Ok(inventory) = service.inspect_worktrees().await else {
            return Vec::new();
        };
        detect_worktree_hygiene_facts(inventory, ctx.now)
    }
}

pub fn detect_worktree_hygiene_facts(
    inventory: crate::worktrees::types::WorktreeInventory,
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    if inventory.summary.total == 0 {
        return Vec::new();
    }
    let candidate_ids = inventory.cleanup_candidates.clone();
    vec![BuddyFact {
        kind: BuddyFactKind::WorktreeHygiene,
        key: format!("worktree_hygiene:{}", inventory.project_hash),
        source: "worktree_hygiene",
        payload: json!({
            "project_hash": inventory.project_hash,
            "summary": inventory.summary,
            "cleanup_candidates": candidate_ids,
            "worktrees": inventory.worktrees,
        }),
        seen_at: now,
        confidence: 0.95,
    }]
}
