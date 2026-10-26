use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::fs;

use super::receipts::BuddyReceipt;
use super::types::{
    BuddyOpportunity, BuddyPriority, BuddyPulse, BuddyState, DailyLlmSpend, OpportunityStatus,
};

pub const BRIEFINGS_KEEP: usize = 14;
pub const BRIEFING_TOP_CARDS: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingJobRun {
    pub workflow_id: String,
    pub run_count: u64,
    pub outputs: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub last_outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingCard {
    pub id: String,
    pub summary: String,
    pub priority: BuddyPriority,
    pub confidence: f32,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingPulseDelta {
    pub tasks_total: u32,
    pub memory_pending_ops: u32,
    pub diagnostics_last_hour: u32,
    pub git_uncommitted_files: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyBriefing {
    pub date: String,
    pub generated_at: String,
    pub job_runs: Vec<BriefingJobRun>,
    pub receipts: Vec<BuddyReceipt>,
    pub top_cards: Vec<BriefingCard>,
    pub pulse: BriefingPulseDelta,
    pub spend: DailyLlmSpend,
}

pub fn briefings_dir(project_root: &Path) -> PathBuf {
    project_root.join(".refact").join("buddy").join("briefings")
}

fn briefing_path(project_root: &Path, date: &str) -> PathBuf {
    briefings_dir(project_root).join(format!("{}.json", date))
}

fn priority_weight(priority: BuddyPriority) -> f32 {
    match priority {
        BuddyPriority::Critical => 4.0,
        BuddyPriority::High => 3.0,
        BuddyPriority::Normal => 2.0,
        BuddyPriority::Low => 1.0,
    }
}

pub fn assemble_briefing(
    state: &BuddyState,
    pulse: &BuddyPulse,
    opportunities: &[BuddyOpportunity],
    receipts: Vec<BuddyReceipt>,
    date: String,
) -> BuddyBriefing {
    let now = Utc::now();
    let mut job_runs: Vec<BriefingJobRun> = state
        .workflow_summaries
        .iter()
        .filter(|summary| summary.run_count > 0 || summary.outputs > 0)
        .map(|summary| BriefingJobRun {
            workflow_id: summary.workflow_id.clone(),
            run_count: summary.run_count,
            outputs: summary.outputs,
            tokens_in: summary.tokens_in,
            tokens_out: summary.tokens_out,
            last_outcome: summary.last_outcome.clone(),
        })
        .collect();
    job_runs.sort_by(|a, b| {
        (b.tokens_in + b.tokens_out)
            .cmp(&(a.tokens_in + a.tokens_out))
            .then_with(|| a.workflow_id.cmp(&b.workflow_id))
    });
    job_runs.truncate(12);

    let mut scored: Vec<(&BuddyOpportunity, f32)> = opportunities
        .iter()
        .filter(|opp| {
            matches!(
                opp.status,
                OpportunityStatus::New | OpportunityStatus::Shown
            )
        })
        .map(|opp| {
            let age_hours = now.signed_duration_since(opp.created_at).num_hours().max(0) as f32;
            let freshness = 1.0 / (1.0 + age_hours / 24.0);
            (
                opp,
                priority_weight(opp.priority) * opp.confidence.max(0.05) * freshness,
            )
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_cards = scored
        .into_iter()
        .take(BRIEFING_TOP_CARDS)
        .map(|(opp, _)| BriefingCard {
            id: opp.id.clone(),
            summary: opp.summary.clone(),
            priority: opp.priority,
            confidence: opp.confidence,
            kind: serde_json::to_value(opp.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
        })
        .collect();

    let recent_receipts = receipts
        .into_iter()
        .rev()
        .take(10)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    BuddyBriefing {
        date,
        generated_at: now.to_rfc3339(),
        job_runs,
        receipts: recent_receipts,
        top_cards,
        pulse: BriefingPulseDelta {
            tasks_total: pulse.tasks.total,
            memory_pending_ops: pulse.memory.pending_ops,
            diagnostics_last_hour: pulse.diagnostics.last_hour,
            git_uncommitted_files: pulse.git.uncommitted_files,
        },
        spend: state.llm_spend.clone(),
    }
}

/// Enabled-storage briefings are ephemeral projections. The caller must not save
/// them into legacy briefing files or feed legacy queue cards into this branch.
pub async fn assemble_operator_briefing(
    runtime: &super::outcome_runtime::OutcomeRuntime,
    state: &BuddyState,
    pulse: &BuddyPulse,
    date: String,
) -> Result<BuddyBriefing, String> {
    let (opportunities, receipts) = runtime
        .briefing_inputs()
        .await
        .map_err(|error| error.to_string())?;
    // Receipt JSON is historical compatibility data, not current execution truth.
    Ok(assemble_briefing(
        state,
        pulse,
        &opportunities,
        receipts,
        date,
    ))
}

pub async fn save_briefing(project_root: &Path, briefing: &BuddyBriefing) -> Result<(), String> {
    let dir = briefings_dir(project_root);
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("failed to create {:?}: {}", dir, e))?;
    super::storage::atomic_write_json(&briefing_path(project_root, &briefing.date), briefing)
        .await?;
    prune_briefings(&dir).await;
    Ok(())
}

async fn prune_briefings(dir: &Path) {
    let Ok(mut entries) = fs::read_dir(dir).await else {
        return;
    };
    let mut files: Vec<PathBuf> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            files.push(path);
        }
    }
    files.sort();
    let overflow = files.len().saturating_sub(BRIEFINGS_KEEP);
    for path in files.into_iter().take(overflow) {
        let _ = fs::remove_file(path).await;
    }
}

pub async fn load_briefing(project_root: &Path, date: Option<&str>) -> Option<BuddyBriefing> {
    let path = match date {
        Some(date) => briefing_path(project_root, date),
        None => {
            let dir = briefings_dir(project_root);
            let mut entries = fs::read_dir(&dir).await.ok()?;
            let mut files: Vec<PathBuf> = Vec::new();
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    files.push(path);
                }
            }
            files.sort();
            files.pop()?
        }
    };
    let content = fs::read_to_string(&path).await.ok()?;
    serde_json::from_str(&content).ok()
}

pub async fn briefing_exists(project_root: &Path, date: &str) -> bool {
    fs::try_exists(briefing_path(project_root, date))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::buddy::types::{BuddyAction, BuddyOpportunityKind, BuddyOpportunityLinks};

    fn opp(id: &str, priority: BuddyPriority, confidence: f32) -> BuddyOpportunity {
        let now = Utc::now();
        BuddyOpportunity {
            id: id.to_string(),
            kind: BuddyOpportunityKind::MemoryOpsBatch,
            summary: format!("card {}", id),
            priority,
            confidence,
            fact_keys: vec![],
            cooldown_key: format!("k:{}", id),
            cooldown_secs: 60,
            status: OpportunityStatus::New,
            proposed_actions: vec![BuddyAction::Dismiss],
            humor: None,
            humor_allowed: false,
            related: BuddyOpportunityLinks::default(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(24),
            resolved_at: None,
        }
    }
}
