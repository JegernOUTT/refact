use std::collections::BTreeSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use crate::app_state::AppState;
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::call_validation::{ChatContent, ChatMessage};
use crate::coordination::scope::WorkCaller;
use crate::organization::{
    candidates::{self, CandidatePage, ReviewDecision, ReviewOutcome, ReviewRequest},
    service,
    types::{ItemsRequest, TaxonomyKind},
};
use crate::subchat::{SubchatConfig, ToolsPolicy};

pub struct BuddyOrganizationJob;
const MAX_CANDIDATES: usize = 12;
const TAXONOMY_LIMIT: usize = 100;
const MAX_PROPOSAL_BYTES: usize = 32 * 1024;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    outcomes: Vec<ReviewOutcome>,
}

fn admit_proposal(text: &str, page: &CandidatePage) -> Result<ReviewRequest, String> {
    if text.len() > MAX_PROPOSAL_BYTES {
        return Err("organization proposal exceeded its size bound".into());
    }
    let proposal: Proposal = serde_json::from_str(text.trim())
        .map_err(|_| "organization model returned invalid proposal JSON")?;
    let issued: BTreeSet<_> = page
        .candidates
        .iter()
        .map(|candidate| (candidate.item.clone(), candidate.fingerprint.clone()))
        .collect();
    let returned: BTreeSet<_> = proposal
        .outcomes
        .iter()
        .map(|outcome| (outcome.item.clone(), outcome.fingerprint.clone()))
        .collect();
    if issued.len() != page.candidates.len()
        || proposal.outcomes.len() != issued.len()
        || returned.len() != issued.len()
        || returned != issued
    {
        return Err("organization proposal did not match the issued batch".into());
    }
    for outcome in &proposal.outcomes {
        if outcome.tag_names.len() > 16
            || outcome
                .topic_name
                .iter()
                .chain(outcome.tag_names.iter())
                .any(|name| {
                    name.trim().is_empty()
                        || name.chars().count() > 120
                        || name.chars().any(char::is_control)
                })
            || (outcome.decision != ReviewDecision::Assign
                && (outcome.topic_name.is_some() || !outcome.tag_names.is_empty()))
        {
            return Err("organization proposal has invalid semantic labels".into());
        }
    }
    Ok(ReviewRequest {
        expected_revision: page.revision,
        request_id: uuid::Uuid::new_v4().to_string(),
        outcomes: proposal.outcomes,
        operations: Vec::new(),
    })
}

async fn configuration(app: &AppState) -> Result<(SubchatConfig, WorkCaller), String> {
    let mut config = crate::subchat::resolve_subchat_config(
        app.gcx.clone(),
        "buddy_organization",
        false,
        None,
        None,
        None,
        None,
        None,
        Some(Vec::new()),
        1,
        false,
        None,
        "NO_TOOLS".into(),
    )
    .await?;
    config.tools = ToolsPolicy::None;
    config.auto_approve_editing_tools = false;
    config.auto_approve_dangerous_commands = false;
    config.autonomous_no_confirm = false;
    config.final_step_force_answer = false;
    config.abort_flag = Some(app.gcx.shutdown_flag.clone());
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
        .await
        .map_err(|_| "organization model capabilities unavailable")?;
    let model = crate::caps::resolve_chat_model(caps, &config.model)?
        .base
        .clone();
    let source_root =
        crate::coordination::roots::resolve_source_workspace_root(app.gcx.clone(), None)
            .await
            .map_err(|_| "organization project scope unavailable")?;
    let caller = WorkCaller {
        checkout_root: source_root.clone(),
        source_root,
        root_chat_id: None,
        principal: "buddy:organization".into(),
        model: Some(model),
    };
    let caller = service::project_caller(app, &caller)
        .await
        .map_err(|e| e.to_string())?;
    Ok((config, caller))
}

struct MessageAudit<'a>(&'a [ChatMessage]);
impl refact_privacy::PrivacyAudited for MessageAudit<'_> {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        refact_privacy::records_from_messages(self.0)
    }
}

fn clear_prompt(
    app: &AppState,
    messages: &mut [ChatMessage],
    model: &crate::caps::BaseModelRecord,
) -> Result<(), String> {
    let records = refact_privacy::records_from_messages(messages).map_err(|e| e.to_string())?;
    let refreshed = crate::privacy::records::declared_file_records(
        &app.gcx,
        records
            .iter()
            .map(|(_, record)| std::path::PathBuf::from(&record.path)),
    )?;
    if let Some(message) = messages.last_mut() {
        crate::privacy::records::merge_records(message, refreshed);
    }
    crate::privacy::destinations::clear_for_model(&app.gcx, MessageAudit(messages), model)
        .map_err(|_| "organization prompt privacy clearance denied".to_string())?;
    Ok(())
}

fn audited_message(
    payload: &serde_json::Value,
    records: Vec<refact_privacy::FileRecord>,
) -> Result<ChatMessage, String> {
    let mut message = ChatMessage::new(
        "user".into(),
        serde_json::to_string(payload).map_err(|e| e.to_string())?,
    );
    crate::privacy::records::merge_records(&mut message, records);
    Ok(message)
}

async fn review(
    app: &AppState,
    config: SubchatConfig,
    caller: &WorkCaller,
    page: CandidatePage,
) -> Result<(), String> {
    if page.candidates.is_empty() {
        return Ok(());
    }
    let topics = service::taxonomy(app, caller, TaxonomyKind::Topics, TAXONOMY_LIMIT, None)
        .await
        .map_err(|e| e.to_string())?;
    let tags = service::taxonomy(app, caller, TaxonomyKind::Tags, TAXONOMY_LIMIT, None)
        .await
        .map_err(|e| e.to_string())?;
    let assignments = service::items(
        app,
        caller,
        ItemsRequest {
            items: page.candidates.iter().map(|c| c.item.clone()).collect(),
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    let definition = crate::yaml_configs::customization_registry::get_subagent_config(
        app.gcx.clone(),
        "buddy_organization",
        None,
    )
    .await
    .ok_or("organization subagent configuration unavailable")?;
    let system = definition
        .messages
        .system_prompt
        .ok_or("organization system prompt unavailable")?;
    let mut records = service::clear_store(app, caller)
        .await
        .map_err(|e| e.to_string())?;
    records.extend(
        page.candidates
            .iter()
            .flat_map(|c| c.source_records.iter().cloned()),
    );
    let payload = serde_json::json!({
        "candidates": page.candidates,
        "partial_scope": page.partial,
        "assignments": assignments,
        "taxonomy": {
            "topics": topics.items, "tags": tags.items,
            "partial": topics.next_cursor.is_some() || tags.next_cursor.is_some(),
            "instruction": "Loaded labels are a bounded sample, not the complete taxonomy. Reuse semantic names when known; the server resolves normalized names across the complete taxonomy."
        }
    });
    let mut messages = vec![
        ChatMessage::new("system".into(), system),
        audited_message(&payload, records)?,
    ];
    if !live_allowed(app).await {
        return Err("organization review cancelled".into());
    }
    clear_prompt(
        app,
        &mut messages,
        caller.model.as_ref().ok_or("missing organization model")?,
    )?;
    let result = crate::subchat::run_subchat(app.gcx.clone(), messages, config).await?;
    let (calls, tokens_in, tokens_out) = super::autonomous_chats::sum_llm_usage(&result.messages);
    {
        let mut buddy = app.buddy.buddy.lock().await;
        if let Some(svc) = buddy.as_mut() {
            svc.state.llm_spend.record(
                &chrono::Local::now().format("%Y-%m-%d").to_string(),
                calls,
                tokens_in,
                tokens_out,
            );
            svc.dirty = true;
        }
    }
    if result.aborted || !live_allowed(app).await {
        return Err("organization review cancelled".into());
    }
    let context_records = refact_privacy::records_from_messages(&result.messages)
        .map_err(|_| "organization result provenance unavailable")?
        .into_iter()
        .map(|(_, record)| record)
        .collect();
    let text = result
        .messages
        .iter()
        .rev()
        .find_map(|message| {
            if message.role != "assistant"
                || message
                    .tool_calls
                    .as_ref()
                    .is_some_and(|calls| !calls.is_empty())
            {
                return None;
            }
            match &message.content {
                ChatContent::SimpleText(text) => Some(text.as_str()),
                _ => None,
            }
        })
        .ok_or("organization model returned no JSON proposal")?;
    let request = admit_proposal(text, &page)?;
    candidates::commit_review_with_context(app, caller, request, context_records)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn settings_allowed(settings: &crate::buddy::settings::BuddySettings) -> bool {
    settings.enabled && settings.organization_enabled && settings.autonomous_chats_enabled
}

async fn live_allowed(app: &AppState) -> bool {
    if app.gcx.shutdown_flag.load(Ordering::SeqCst) {
        return false;
    }
    let buddy = app.buddy.buddy.lock().await;
    buddy
        .as_ref()
        .is_some_and(|svc| settings_allowed(&svc.settings) && !svc.llm_budget_exhausted())
}

async fn bounded<F: std::future::Future<Output = Result<(), String>>>(
    shutdown: Arc<AtomicBool>,
    run: F,
    timeout: Duration,
) -> Result<(), String> {
    let cancelled = async {
        loop {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    tokio::select! {
        biased;
        _ = cancelled => Err("organization review cancelled".into()),
        result = tokio::time::timeout(timeout, run) => result.map_err(|_| "organization review timed out".to_string())?,
    }
}

#[async_trait::async_trait]
impl BuddyJob for BuddyOrganizationJob {
    fn id(&self) -> &str {
        "buddy_organization"
    }
    fn cooldown_seconds(&self) -> u64 {
        60
    }
    fn priority(&self) -> u32 {
        29
    }
    fn is_autonomous(&self) -> bool {
        true
    }
    async fn should_run(&self, app: AppState, ctx: &BuddyJobContext) -> bool {
        settings_allowed(&ctx.settings) && live_allowed(&app).await
    }
    async fn execute(&self, app: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        if !settings_allowed(&ctx.settings) || !live_allowed(&app).await {
            return BuddyJobResult::default();
        }
        let run = async {
            let (config, caller) = configuration(&app).await?;
            let page = candidates::list_candidates(&app, &caller, MAX_CANDIDATES)
                .await
                .map_err(|e| e.to_string())?;
            review(&app, config, &caller, page).await
        };
        if let Err(error) =
            bounded(app.gcx.shutdown_flag.clone(), run, Duration::from_secs(90)).await
        {
            tracing::debug!(%error, "Buddy organization review skipped or failed");
        }
        BuddyJobResult::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organization::candidates::Candidate;
    use crate::organization::types::{Item, ItemKind};

    fn page() -> CandidatePage {
        CandidatePage {
            revision: 41,
            next_cursor: None,
            partial: true,
            candidates: (0..2)
                .map(|i| Candidate {
                    item: Item {
                        kind: ItemKind::Chat,
                        id: format!("fixture-{i}"),
                    },
                    fingerprint: format!("fingerprint-{i}"),
                    title: Some("Synthetic".into()),
                    summary: None,
                    user_excerpts: vec!["Synthetic evidence".into()],
                    partial: true,
                    source_records: vec![],
                })
                .collect(),
        }
    }
    fn proposal(page: &CandidatePage) -> serde_json::Value {
        serde_json::json!({"outcomes": page.candidates.iter().map(|c| serde_json::json!({
            "item":c.item,"fingerprint":c.fingerprint,"decision":"assign",
            "topic_name":"Synthetic theme","tag_names":["Fixture"]
        })).collect::<Vec<_>>()})
    }
    #[test]
    fn semantic_proposal_owns_revision_request_identity_and_no_operations() {
        let page = page();
        let text = proposal(&page).to_string();
        let first = admit_proposal(&text, &page).unwrap();
        let second = admit_proposal(&text, &page).unwrap();
        assert_eq!(first.expected_revision, 41);
        assert_ne!(first.request_id, second.request_id);
        assert!(first.operations.is_empty());
        assert_eq!(
            first.outcomes[0].topic_name.as_deref(),
            Some("Synthetic theme")
        );
        let mut injected = proposal(&page);
        injected["expected_revision"] = 99.into();
        assert!(admit_proposal(&injected.to_string(), &page).is_err());
    }
    #[test]
    fn proposal_rejects_duplicate_missing_foreign_and_stale_outcomes() {
        let page = page();
        for mutation in 0..5 {
            let mut value = proposal(&page);
            match mutation {
                0 => value["outcomes"][1] = value["outcomes"][0].clone(),
                1 => {
                    value["outcomes"].as_array_mut().unwrap().pop();
                }
                2 => value["outcomes"][0]["item"]["id"] = "foreign".into(),
                3 => value["outcomes"][0]["fingerprint"] = "stale".into(),
                _ => value["outcomes"][0]["decision"] = "keep".into(),
            }
            assert!(
                admit_proposal(&value.to_string(), &page).is_err(),
                "mutation {mutation}"
            );
        }
    }
    #[tokio::test]
    async fn shipping_admission_commits_names_and_rejects_forged_candidate_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let task = dir.path().join(".refact/tasks/synthetic-task");
        std::fs::create_dir_all(&task).unwrap();
        std::fs::write(
            task.join("meta.yaml"),
            "id: synthetic-task\ntitle: Synthetic organization fixture\n",
        )
        .unwrap();
        let app = AppState::from_gcx(gcx).await;
        let caller = WorkCaller {
            source_root: dir.path().into(),
            checkout_root: dir.path().into(),
            root_chat_id: None,
            principal: "buddy:organization".into(),
            model: Some(crate::caps::BaseModelRecord {
                id: "fixture/model".into(),
                ..Default::default()
            }),
        };
        let page = candidates::list_candidates(&app, &caller, 12)
            .await
            .unwrap();
        assert_eq!(page.candidates.len(), 1);
        let mut forged = proposal(&page);
        forged["outcomes"][0]["item"]["id"] = "foreign".into();
        assert!(admit_proposal(&forged.to_string(), &page).is_err());
        let request = admit_proposal(&proposal(&page).to_string(), &page).unwrap();
        candidates::commit_review(&app, &caller, request)
            .await
            .unwrap();
        let assignments = service::items(
            &app,
            &caller,
            ItemsRequest {
                items: vec![page.candidates[0].item.clone()],
            },
        )
        .await
        .unwrap();
        assert!(assignments.assignments[0].topic_id.is_some());
        assert_eq!(assignments.assignments[0].tag_ids.len(), 1);
        let topics = service::taxonomy(&app, &caller, TaxonomyKind::Topics, 100, None)
            .await
            .unwrap();
        assert_eq!(topics.items[0].name, "Synthetic theme");
        assert!(candidates::list_candidates(&app, &caller, 12)
            .await
            .unwrap()
            .candidates
            .is_empty());
    }

    #[tokio::test]
    async fn attached_provenance_is_rechecked_after_policy_change() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let record = refact_privacy::FileRecord {
            path: "/synthetic/private.txt".into(),
            zone: "normal".into(),
            attribution: refact_privacy::Attribution::Declared,
        };
        let mut messages = vec![audited_message(
            &serde_json::json!({"title":"Synthetic private title"}),
            vec![record],
        )
        .unwrap()];
        let model = crate::caps::BaseModelRecord {
            id: "fixture/model".into(),
            ..Default::default()
        };
        assert_eq!(
            refact_privacy::records_from_messages(&messages)
                .unwrap()
                .len(),
            1
        );
        assert!(crate::privacy::destinations::clear_for_model(
            &gcx,
            MessageAudit(&messages),
            &model
        )
        .is_ok());
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            blocked: vec!["**/private.txt".into()],
            ..Default::default()
        });
        assert!(clear_prompt(&app, &mut messages, &model).is_err());
    }
    #[tokio::test]
    async fn timeout_and_shutdown_drop_owned_work_without_detached_commit() {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        for shutdown in [false, true] {
            let dropped = Arc::new(AtomicBool::new(false));
            let owned = Dropped(dropped.clone());
            let run = async move {
                let _owned = owned;
                std::future::pending::<()>().await;
                Ok(())
            };
            let result = bounded(
                Arc::new(AtomicBool::new(shutdown)),
                run,
                Duration::from_millis(1),
            )
            .await;
            assert!(result.is_err());
            assert!(dropped.load(Ordering::SeqCst));
        }
    }
    #[test]
    fn each_background_permission_is_required() {
        let settings = crate::buddy::settings::BuddySettings::default();
        assert!(settings_allowed(&settings));
        for gate in 0..3 {
            let mut denied = settings.clone();
            match gate {
                0 => denied.enabled = false,
                1 => denied.organization_enabled = false,
                _ => denied.autonomous_chats_enabled = false,
            }
            assert!(!settings_allowed(&denied));
        }
    }
}
