use chrono::Utc;
use refact_core::knowledge_index::KnowledgeCard;

use crate::app_state::AppState;
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::buddy::types::{
    BuddyAction, BuddyOpportunity, BuddyOpportunityKind, BuddyOpportunityLinks, BuddyPriority,
    OpportunityStatus,
};

pub struct BuddyHandbookDistillerJob;

const COOLDOWN_SECONDS: u64 = 24 * 60 * 60;
const PRIORITY: u32 = 28;
const MIN_CLUSTER_DOCS: usize = 3;
const MAX_SOURCE_DOCS: usize = 12;
const HANDBOOK_FRESH_DAYS: u64 = 7;

pub(crate) fn handbook_slug(tag: &str) -> String {
    let slug: String = tag
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    slug.trim_matches('-').to_string()
}

pub(crate) fn render_handbook_doc(tag: &str, cards: &[KnowledgeCard]) -> String {
    let mut out = vec![
        format!("# Handbook: {}", tag),
        String::new(),
        format!(
            "Distilled from {} recurring insight(s) tagged `{}`. Generated {}.",
            cards.len(),
            tag,
            Utc::now().format("%Y-%m-%d")
        ),
        String::new(),
    ];
    for card in cards.iter().take(MAX_SOURCE_DOCS) {
        out.push(format!("## {}", card.title));
        if let Some(summary) = card
            .summary
            .as_deref()
            .or(card.description.as_deref())
            .filter(|text| !text.trim().is_empty())
        {
            out.push(summary.trim().to_string());
        }
        out.push(format!("- source: `{}`", card.file_path.display()));
        out.push(String::new());
    }
    out.join("\n")
}

fn handbook_target_path(tag: &str) -> String {
    format!(".refact/knowledge/handbook/{}.md", handbook_slug(tag))
}

async fn handbook_doc_is_fresh(project_root: &std::path::Path, tag: &str) -> bool {
    let path = project_root.join(handbook_target_path(tag));
    let Ok(metadata) = tokio::fs::metadata(&path).await else {
        return false;
    };
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .map(|age| age.as_secs() < HANDBOOK_FRESH_DAYS * 24 * 60 * 60)
        .unwrap_or(true)
}

#[async_trait::async_trait]
impl BuddyJob for BuddyHandbookDistillerJob {
    fn id(&self) -> &str {
        "buddy_handbook_distiller"
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        ctx.settings.housekeeping_enabled
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let clusters = {
            let idx = gcx.gcx.knowledge_index.lock().await;
            idx.tag_clusters(MIN_CLUSTER_DOCS)
        };
        let mut chosen: Option<(String, Vec<KnowledgeCard>)> = None;
        for (tag, cards) in clusters.into_iter().take(8) {
            if !handbook_doc_is_fresh(&ctx.project_root, &tag).await {
                chosen = Some((tag, cards));
                break;
            }
        }
        let Some((tag, cards)) = chosen else {
            return BuddyJobResult::default();
        };
        let content = render_handbook_doc(&tag, &cards);
        let target_path = handbook_target_path(&tag);
        let draft = {
            let buddy_arc = gcx.buddy.buddy.clone();
            let mut lock = buddy_arc.lock().await;
            let Some(svc) = lock.as_mut() else {
                return BuddyJobResult::default();
            };
            match svc.create_draft(
                crate::buddy::types::DraftKind::PulseReport,
                format!("Handbook: {}", tag),
                content,
                format!(
                    "Distilled {} recurring '{}' insights into one handbook page.",
                    cards.len(),
                    tag
                ),
            ) {
                Ok(draft) => draft,
                Err(err) => {
                    tracing::warn!("buddy: handbook draft failed: {}", err);
                    return BuddyJobResult::default();
                }
            }
        };
        let now = Utc::now();
        let mut opp = BuddyOpportunity {
            id: uuid::Uuid::new_v4().to_string(),
            kind: BuddyOpportunityKind::JobFinding,
            summary: format!(
                "Distill {} '{}' insights into the handbook",
                cards.len(),
                tag
            ),
            priority: BuddyPriority::Normal,
            confidence: 0.8,
            fact_keys: vec![],
            cooldown_key: format!("handbook:{}", handbook_slug(&tag)),
            cooldown_secs: COOLDOWN_SECONDS,
            status: OpportunityStatus::New,
            proposed_actions: vec![
                BuddyAction::ApplyConfigPatch {
                    draft_id: draft.id.clone(),
                    target_path: target_path.clone(),
                },
                BuddyAction::Dismiss,
            ],
            humor: None,
            humor_allowed: false,
            related: BuddyOpportunityLinks::default(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(48),
            resolved_at: None,
        };
        opp.related.config_paths = vec![target_path];
        BuddyJobResult {
            opportunities: vec![(opp, COOLDOWN_SECONDS)],
            last_result: Some(format!("proposed:{}", tag)),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn card(title: &str, path: &str) -> KnowledgeCard {
        KnowledgeCard {
            id: title.to_string(),
            title: title.to_string(),
            summary: Some(format!("{} summary", title)),
            description: None,
            tags: vec!["testing".to_string()],
            filenames: vec![],
            entities: vec![],
            related_files: vec![],
            related_entities: vec![],
            kind: Some("insight".to_string()),
            created: Some("2026-07-01".to_string()),
            created_at: None,
            updated: None,
            file_path: PathBuf::from(path),
        }
    }

    #[test]
    fn slug_normalizes_tags() {
        assert_eq!(handbook_slug("Memory Garden!"), "memory-garden");
        assert_eq!(handbook_slug("diag:cluster"), "diag-cluster");
    }

    #[test]
    fn render_includes_sources_and_titles() {
        let cards = vec![card("First insight", "/k/a.md"), card("Second", "/k/b.md")];
        let doc = render_handbook_doc("testing", &cards);
        assert!(doc.contains("# Handbook: testing"));
        assert!(doc.contains("## First insight"));
        assert!(doc.contains("source: `/k/a.md`"));
        assert!(doc.contains("2 recurring insight(s)"));
    }
}
