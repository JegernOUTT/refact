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

fn has_live_handbook_draft(
    drafts: &[crate::buddy::types::BuddyDraft],
    slug: &str,
    now: chrono::DateTime<Utc>,
) -> bool {
    let prefix = "Handbook: ";
    drafts.iter().any(|draft| {
        draft.kind == crate::buddy::types::DraftKind::PulseReport
            && draft.expires_at > now
            && draft
                .title
                .strip_prefix(prefix)
                .is_some_and(|tag| handbook_slug(tag) == slug)
    })
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
        let slug = handbook_slug(&tag);
        let buddy_arc = gcx.buddy.buddy.clone();
        let pending_handbook = {
            let buddy = buddy_arc.lock().await;
            buddy.as_ref().is_some_and(|service| {
                has_live_handbook_draft(&service.draft_store.snapshot(), &slug, Utc::now())
            })
        };
        if pending_handbook {
            tracing::debug!(slug, "buddy: handbook draft already pending");
            return BuddyJobResult::default();
        }
        let content = render_handbook_doc(&tag, &cards);
        let target_path = handbook_target_path(&tag);
        let draft = {
            match crate::buddy::actor::BuddyService::create_draft_durable(
                &buddy_arc,
                crate::buddy::types::DraftKind::PulseReport,
                format!("Handbook: {}", tag),
                content,
                format!(
                    "Distilled {} recurring '{}' insights into one handbook page.",
                    cards.len(),
                    tag
                ),
            )
            .await
            {
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
    use crate::app_state::AppState;
    use crate::buddy::actor::BuddyService;
    use crate::buddy::outcome_runtime::OutcomeRuntime;
    use crate::buddy::runtime_queue::RuntimeQueue;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::state::default_buddy_state;
    use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};
    use std::path::PathBuf;
    use tokio::sync::broadcast;

    fn card(title: &str, path: &str, tag: &str) -> KnowledgeCard {
        KnowledgeCard {
            id: title.to_string(),
            title: title.to_string(),
            summary: Some(format!("{} summary", title)),
            description: None,
            tags: vec![tag.to_string()],
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

    async fn setup_draft_context(app: &AppState, root: &std::path::Path) -> OutcomeRuntime {
        let project = ProjectIdentity {
            project_id: "handbook-test".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        };
        let worker = WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        };
        let runtime = OutcomeRuntime::open(root, project, worker).unwrap();
        let (events, _) = broadcast::channel(16);
        let mut service = BuddyService::new(
            root.to_path_buf(),
            default_buddy_state(),
            BuddySettings::default(),
            vec![],
            RuntimeQueue::new(),
            events,
            None,
        );
        service.operator_features.storage = true;
        service.outcome_runtime = Some(runtime.clone());
        *app.buddy.buddy.lock().await = Some(service);
        runtime
    }

    async fn run_distillation(app: &AppState, root: &std::path::Path) -> BuddyJobResult {
        BuddyHandbookDistillerJob
            .execute(
                app.clone(),
                BuddyJobContext {
                    identity_name: "Pixel".into(),
                    personality: Default::default(),
                    onboarding: Default::default(),
                    recent_diagnostics: vec![],
                    project_root: root.to_path_buf(),
                    job_state: Default::default(),
                    workflow_summaries: vec![],
                    total_workflow_runs: 0,
                    suggestion_state: vec![],
                    pet: Default::default(),
                    active_quest: None,
                    settings: BuddySettings::default(),
                    pulse: Default::default(),
                    facts: vec![],
                    recent_activities: vec![],
                },
            )
            .await
    }

    async fn seed_cards(app: &AppState, tags: &[&str]) {
        let mut index = app.gcx.knowledge_index.lock().await;
        for tag in tags {
            for idx in 0..3 {
                index.add_card(card(
                    &format!("{tag} insight {idx}"),
                    &format!("/knowledge/{tag}-{idx}.md"),
                    tag,
                ));
            }
        }
    }

    #[test]
    fn slug_normalizes_tags() {
        assert_eq!(handbook_slug("Memory Garden!"), "memory-garden");
        assert_eq!(handbook_slug("diag:cluster"), "diag-cluster");
    }

    #[test]
    fn render_includes_sources_and_titles() {
        let cards = vec![
            card("First insight", "/k/a.md", "testing"),
            card("Second", "/k/b.md", "testing"),
        ];
        let doc = render_handbook_doc("testing", &cards);
        assert!(doc.contains("# Handbook: testing"));
        assert!(doc.contains("## First insight"));
        assert!(doc.contains("source: `/k/a.md`"));
        assert!(doc.contains("2 recurring insight(s)"));
    }

    #[test]
    fn pending_handbook_draft_is_scoped_by_slug_and_lifetime() {
        let now = Utc::now();
        let draft = crate::buddy::types::BuddyDraft {
            id: "draft-1".into(),
            kind: crate::buddy::types::DraftKind::PulseReport,
            title: "Handbook: Memory Garden!".into(),
            yaml_or_json: "# handbook".into(),
            explanation: "proposal".into(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
        };

        assert!(has_live_handbook_draft(
            &[draft.clone()],
            "memory-garden",
            now
        ));
        assert!(!has_live_handbook_draft(&[draft.clone()], "testing", now));

        let mut expired = draft;
        expired.expires_at = now;
        assert!(!has_live_handbook_draft(&[expired], "memory-garden", now));
    }

    #[tokio::test]
    async fn same_slug_pending_handbook_draft_is_skipped_until_consumed() {
        let root = tempfile::tempdir().unwrap();
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let runtime = setup_draft_context(&app, root.path()).await;
        seed_cards(&app, &["memory"]).await;

        let first = run_distillation(&app, root.path()).await;
        assert_eq!(first.opportunities.len(), 1);
        let draft_id = match &first.opportunities[0].0.proposed_actions[0] {
            BuddyAction::ApplyConfigPatch { draft_id, .. } => draft_id.clone(),
            _ => panic!("expected handbook draft action"),
        };
        let second = run_distillation(&app, root.path()).await;
        assert!(second.opportunities.is_empty());
        assert_eq!(
            app.buddy
                .buddy
                .lock()
                .await
                .as_ref()
                .unwrap()
                .draft_store
                .snapshot()
                .len(),
            1
        );
        assert_eq!(
            runtime
                .list_work_items(None, 10)
                .await
                .unwrap()
                .entries
                .len(),
            1
        );
        {
            let mut buddy = app.buddy.buddy.lock().await;
            assert!(buddy.as_mut().unwrap().consume_draft(&draft_id).is_some());
        }
        let third = run_distillation(&app, root.path()).await;
        assert_eq!(third.opportunities.len(), 1);
        assert_eq!(
            app.buddy
                .buddy
                .lock()
                .await
                .as_ref()
                .unwrap()
                .draft_store
                .snapshot()
                .len(),
            1
        );
        assert_eq!(
            runtime
                .list_work_items(None, 10)
                .await
                .unwrap()
                .entries
                .len(),
            2
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn different_handbook_slug_has_its_own_draft() {
        let root = tempfile::tempdir().unwrap();
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let runtime = setup_draft_context(&app, root.path()).await;
        seed_cards(&app, &["memory", "testing"]).await;
        std::fs::create_dir_all(root.path().join(".refact/knowledge/handbook")).unwrap();
        std::fs::write(
            root.path().join(handbook_target_path("memory")),
            "already distilled",
        )
        .unwrap();

        let result = run_distillation(&app, root.path()).await;
        assert_eq!(result.opportunities.len(), 1);
        assert_eq!(
            app.buddy
                .buddy
                .lock()
                .await
                .as_ref()
                .unwrap()
                .draft_store
                .snapshot()
                .len(),
            1
        );
        let action = &result.opportunities[0].0.proposed_actions[0];
        assert!(
            matches!(action, BuddyAction::ApplyConfigPatch { target_path, .. } if target_path == &handbook_target_path("testing"))
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn handbook_draft_expiry_allows_a_fresh_proposal() {
        let root = tempfile::tempdir().unwrap();
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let runtime = setup_draft_context(&app, root.path()).await;
        seed_cards(&app, &["memory"]).await;

        let first = run_distillation(&app, root.path()).await;
        assert_eq!(first.opportunities.len(), 1);
        {
            let mut buddy = app.buddy.buddy.lock().await;
            let service = buddy.as_mut().unwrap();
            let draft = service.draft_store.snapshot().remove(0);
            service
                .draft_store
                .restore(vec![crate::buddy::types::BuddyDraft {
                    expires_at: Utc::now() - chrono::Duration::seconds(1),
                    ..draft
                }]);
        }
        {
            let mut index = app.gcx.knowledge_index.lock().await;
            for idx in 0..3 {
                index.add_card(card(
                    &format!("memory revised {idx}"),
                    &format!("/knowledge/memory-revised-{idx}.md"),
                    "memory",
                ));
            }
        }
        let second = run_distillation(&app, root.path()).await;
        assert_eq!(second.opportunities.len(), 1);
        assert_eq!(
            app.buddy
                .buddy
                .lock()
                .await
                .as_ref()
                .unwrap()
                .draft_store
                .snapshot()
                .len(),
            2
        );
        assert_eq!(
            runtime
                .list_work_items(None, 10)
                .await
                .unwrap()
                .entries
                .len(),
            2
        );
        runtime.shutdown().await;
    }
}
