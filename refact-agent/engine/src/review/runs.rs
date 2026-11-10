//! Runtime-owned review delegation. Candidate identity is captured before launch,
//! never inferred from the review's current candidate when a child returns.
#[cfg(test)]
mod candidate_fence_tests {
    use super::*;
    #[test]
    fn injected_review_contract_is_bounded_and_carries_argument_and_refutation_guidance() {
        // Prompt wiring/content regression only, not proof of model review quality.
        assert!(REVIEW_CRITICISM_CONTRACT.len() < 3000);
        for required in [
            "captured base",
            "ADVERSARIAL refutation",
            "caller prevents trigger",
            "trust-boundary validation",
            "Delta",
            "Trigger",
            "Impact",
            "Counterevidence",
            "Remedy",
            "Evidence status",
            "There is no finding quota",
            "Only review_record_evidence",
            "owning foreground chat authors Tour",
        ] {
            assert!(
                REVIEW_CRITICISM_CONTRACT.contains(required),
                "missing {required}"
            );
        }
    }

    #[tokio::test]
    async fn ordinary_child_is_not_reclassified_when_parent_later_creates_review() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let app = AppState::from_gcx(gcx).await;
        let (record, _, _) = app
            .agents
            .create(crate::agents::types::CreateAgentRequest {
                parent_chat_id: "late-review-owner".into(),
                parent_root_chat_id: Some("late-review-owner".into()),
                parent_tool_call_id: None,
                kind: crate::agents::types::BgAgentKind::Subagent,
                config_name: "subagent".into(),
                title: "Ordinary worker".into(),
                prompt: "Inspect ordinary workspace".into(),
                target_files: vec![],
                model: "test/default".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        app.agents
            .mark_running(&record.agent_id, "ordinary-child".into())
            .await
            .unwrap();
        app.agent_executions
            .remember_scope(&record.agent_id, root.clone());
        let mut child = crate::chat::types::ChatSession::new("ordinary-child".into());
        child.work_project_root = Some(root.clone());
        let child = Arc::new(Mutex::new(child));
        app.chat
            .sessions
            .write()
            .await
            .insert("ordinary-child".into(), child.clone());
        verify_child_binding(&app, "ordinary-child").await.unwrap();
        ReviewStore::open(&root)
            .unwrap()
            .create(
                "late-review-owner",
                "Later review",
                Origin::Imported,
                "later-create",
            )
            .unwrap();
        verify_child_binding(&app, "ordinary-child").await.unwrap();
        // A known review worker is not permitted to take this no-op path.
        let review = super::super::service::get_for_chat(root.clone(), "late-review-owner".into())
            .await
            .unwrap()
            .unwrap();
        assert!(review.runs.is_empty());
        assert!(child.lock().await.thread.worktree.is_none());
    }

    #[tokio::test]
    async fn registered_child_privacy_and_candidate_binding_survive_foreground_update() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let repo = git2::Repository::init(&root).unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Test", "test@localhost").unwrap();
        let head = repo
            .commit(Some("HEAD"), &signature, &signature, "seed", &tree, &[])
            .unwrap()
            .to_string();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let app = AppState::from_gcx(gcx).await;
        let store = ReviewStore::open(&root).unwrap();
        let review = store
            .create("owner", "review", Origin::Imported, "create")
            .unwrap();
        let candidate = store
            .capture(
                &root,
                CaptureRequest {
                    base_oid: head.clone(),
                    head_oid: head,
                    policy: CapturePolicy::default(),
                    origin: Origin::Imported,
                },
            )
            .unwrap();
        apply(
            root.clone(),
            &review.id,
            Operation::StageCandidate {
                candidate: candidate.clone(),
            },
        )
        .await
        .unwrap();
        let worktree = WorktreeService::new_async(app.gcx.cache_dir.clone(), root.clone())
            .await
            .unwrap()
            .create_review_snapshot("owner", &review.id, vec![])
            .await
            .unwrap()
            .worktree
            .meta;
        let (record, _, _) = app
            .agents
            .create(crate::agents::types::CreateAgentRequest {
                parent_chat_id: "owner".into(),
                parent_root_chat_id: Some("owner".into()),
                parent_tool_call_id: None,
                kind: crate::agents::types::BgAgentKind::Subagent,
                config_name: "subagent".into(),
                title: "Review worker".into(),
                prompt: "Inspect".into(),
                target_files: vec![],
                model: "test/default".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: Some(worktree.id.clone()),
                worktree_branch: worktree.branch.clone(),
            })
            .await
            .unwrap();
        let record = app
            .agents
            .mark_running(&record.agent_id, "child".into())
            .await
            .unwrap();
        app.agent_executions
            .remember_scope(&record.agent_id, root.clone());
        let mut inputs = std::collections::BTreeMap::new();
        inputs.insert("derived_origin_labels".into(), serde_json::json!({"private.txt":{"zone":"private","origin":"/retained/original/private.txt"}}).to_string());
        apply(
            root.clone(),
            &review.id,
            Operation::AddMaterial {
                material: Material {
                    id: "child-input".into(),
                    kind: MaterialKind::Investigation,
                    candidate_id: candidate.id.clone(),
                    agent_id: Some(record.agent_id.clone()),
                    description: "Accepted child input".into(),
                    content_digest: candidate.content_digest.clone(),
                    input_digests: inputs,
                    environment: None,
                    worktree_id: Some(worktree.id.clone()),
                    worktree_root: Some(worktree.root.to_string_lossy().into()),
                    artifacts: vec![],
                },
            },
        )
        .await
        .unwrap();
        apply(
            root.clone(),
            &review.id,
            Operation::PutRun {
                run: runtime_run(&record, "run".into(), candidate.id.clone()),
            },
        )
        .await
        .unwrap();
        let mut child = crate::chat::types::ChatSession::new("child".into());
        child.work_project_root = Some(root.clone());
        child.thread.worktree = Some(worktree.clone());
        let child = Arc::new(Mutex::new(child));
        app.chat
            .sessions
            .write()
            .await
            .insert("child".into(), child.clone());
        verify_child_binding(&app, "child").await.unwrap();
        let derived = child.lock().await.derived_privacy_zones.clone();
        assert_eq!(
            derived
                .read()
                .unwrap()
                .get(&worktree.root.join("private.txt"))
                .unwrap()
                .origin
                .as_deref(),
            Some("/retained/original/private.txt")
        );
        let newer = store
            .capture(
                &root,
                CaptureRequest {
                    base_oid: candidate.base_oid.clone(),
                    head_oid: candidate.head_oid.clone(),
                    policy: CapturePolicy::default(),
                    origin: Origin::Imported,
                },
            )
            .unwrap();
        apply(
            root.clone(),
            &review.id,
            Operation::StageCandidate {
                candidate: newer.clone(),
            },
        )
        .await
        .unwrap();
        let review = apply(
            root.clone(),
            &review.id,
            Operation::ActivateCandidate {
                candidate_id: newer.id,
            },
        )
        .await
        .unwrap();
        verify_child_binding(&app, "child").await.unwrap();
        assert_eq!(
            candidate_for_identity(&review, "child", Some(&record.agent_id)).unwrap(),
            candidate.id
        );
        child.lock().await.thread.worktree.as_mut().unwrap().id = "foreign".into();
        assert!(verify_child_binding(&app, "child").await.is_err());
        child.lock().await.thread.worktree = None;
        assert!(verify_child_binding(&app, "child").await.is_err());
    }

    #[test]
    fn late_child_results_keep_original_candidate_and_foreign_identity_fails() {
        let temp = tempfile::tempdir().unwrap();
        let store = ReviewStore::open(temp.path()).unwrap();
        let mut review = store
            .create("owner", "review", Origin::Imported, "create")
            .unwrap();
        review.current_candidate_id = Some("new".into());
        review.runs.push(Run {
            id: "run".into(),
            candidate_id: "old".into(),
            agent_id: "actual-child".into(),
            purpose: "inspect".into(),
            model: None,
            domain: None,
            status: RunStatus::Completed,
            started_at_ms: 1,
            finished_at_ms: Some(2),
            error: None,
        });
        assert_eq!(
            candidate_for_identity(&review, "owner", None).unwrap(),
            "new"
        );
        assert_eq!(
            candidate_for_identity(&review, "child-chat", Some("actual-child")).unwrap(),
            "old"
        );
        assert!(candidate_for_identity(&review, "child-chat", Some("foreign-child")).is_err());
        assert!(candidate_for_identity(&review, "child-chat", None).is_err());
    }
}

use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use refact_review_core::*;
use crate::{
    app_state::AppState,
    at_commands::at_commands::AtCommandsContext,
    agents::{
        spawn::{SpawnRequest, SpawnHandle, SpawnWorktreeMode},
        types::{BackgroundAgent, BgAgentStatus},
    },
    worktrees::{service::WorktreeService, types::WorktreeMeta},
};

pub struct LaunchBinding {
    root: PathBuf,
    review_id: String,
    owner: String,
    candidate: String,
    material: Material,
}

pub async fn bound_review(
    ccx: Arc<Mutex<AtCommandsContext>>,
    review_id: Option<&str>,
) -> std::result::Result<(PathBuf, Review), String> {
    let (app, chat, agent, scope) = {
        let c = ccx.lock().await;
        (
            c.app.clone(),
            c.chat_id.clone(),
            c.background_agent_id.clone(),
            c.execution_scope_worktree(),
        )
    };
    let root = super::service::trusted_root(&app, &chat).await?;
    let owner = if let Some(agent) = &agent {
        let record = app.agents.get_any(&agent).await?;
        if record.child_chat_id.as_deref() != Some(&chat) || record.status.is_terminal() {
            return Err("Review child execution identity unavailable".into());
        }
        record.parent_chat_id
    } else {
        chat.clone()
    };
    let mut review = if let Some(agent) = agent.as_ref() {
        super::service::for_agent(root.clone(), owner.clone(), agent.clone()).await
    } else {
        super::service::get_for_chat(root.clone(), owner.clone()).await
    }
    .map_err(|e| e.to_string())?
    .ok_or("No bound review")?;
    if review_id.is_some_and(|id| id != review.id) {
        if agent.is_some() {
            return Err("Review not found".into());
        }
        let manifest = super::stacks::get(&app, &owner)
            .await?
            .ok_or("Review not found")?;
        if !manifest.scope_choice.confirmed {
            return Err("Review not found".into());
        }
        let mut matched = None;
        for member in &manifest.members {
            if manifest.scope_choice.scope == StackScope::Single
                && member.id != manifest.scope_choice.selected_member_id
            {
                continue;
            }
            if let Some(candidate) = super::stacks::member(&app, &owner, member.id.clone()).await? {
                if Some(candidate.id.as_str()) == review_id && candidate.owner_chat_id == owner {
                    matched = Some(candidate);
                    break;
                }
            }
        }
        review = matched.ok_or("Review not found")?;
    }
    if owner == chat {
        super::context::verify_binding_for_chat(&app, &chat).await?;
    } else {
        let record = app
            .agents
            .find_agent_id_by_child_chat_id(&chat)
            .await
            .ok_or("Child is not registry owned")?;
        let run = review
            .runs
            .iter()
            .find(|r| r.agent_id == record)
            .ok_or("Child has no accepted review run")?;
        let material = review
            .materials
            .iter()
            .find(|m| m.agent_id.as_deref() == Some(&record) && m.candidate_id == run.candidate_id)
            .ok_or("Child has no pinned material")?;
        let scope = scope.ok_or("Review child lost its checkout")?;
        if material.worktree_id.as_deref() != Some(&scope.id)
            || material.worktree_root.as_deref() != scope.root.to_str()
        {
            return Err("Review child checkout changed".into());
        }
        WorktreeService::new_async(app.gcx.cache_dir.clone(), root.clone())
            .await?
            .validate_worktree_meta_strict(&scope)
            .await?;
    }
    Ok((root, review))
}

/// Called by chat preparation before any tool executes; preserves imported provenance
/// even when the foreground review has switched to another candidate.
pub async fn verify_child_binding(app: &AppState, chat: &str) -> std::result::Result<(), String> {
    let Some(agent) = app.agents.find_agent_id_by_child_chat_id(chat).await else {
        return Ok(());
    };
    let record = app.agents.get_any(&agent).await?;
    let root = match app.agent_executions.saved_scope(&agent) {
        Some(root) => root,
        None => {
            let session = app.chat.sessions.read().await.get(chat).cloned();
            let Some(session) = session else {
                return Ok(());
            };
            let session = session.lock().await;
            match crate::coordination::scope::session_source_root(&session) {
                Some(root) => root,
                None if session
                    .thread
                    .worktree
                    .as_ref()
                    .is_some_and(|w| w.kind == "review") =>
                {
                    return Err("Review child lost accepted project identity".into())
                }
                None => return Ok(()),
            }
        }
    };
    let Some(review) =
        super::service::for_agent(root.clone(), record.parent_chat_id, agent.clone())
            .await
            .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    let run = match review.runs.iter().find(|r| r.agent_id == agent) {
        Some(run) => run,
        None => {
            // A later review activation in the parent cannot retroactively turn
            // an ordinary accepted child into a review worker. Conversely an
            // accepted review checkout must never silently lose its Run fence.
            let accepted_review_scope = record.worktree_id.as_ref().is_some_and(|id| {
                review
                    .materials
                    .iter()
                    .any(|m| m.worktree_id.as_ref() == Some(id))
            });
            let session = app.chat.sessions.read().await.get(chat).cloned();
            let attached_review_scope = if let Some(session) = session {
                session
                    .lock()
                    .await
                    .thread
                    .worktree
                    .as_ref()
                    .is_some_and(|w| w.kind == "review")
            } else {
                false
            };
            if accepted_review_scope || attached_review_scope {
                return Err("Review child has no predispatch run binding".into());
            }
            return Ok(());
        }
    };
    let material = review
        .materials
        .iter()
        .find(|m| m.agent_id.as_deref() == Some(&agent) && m.candidate_id == run.candidate_id)
        .ok_or("Review child material unavailable")?;
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat)
        .cloned()
        .ok_or("Review child session unavailable")?;
    let scope = session
        .lock()
        .await
        .thread
        .worktree
        .clone()
        .ok_or("Review child checkout missing")?;
    if material.worktree_id.as_deref() != Some(&scope.id)
        || material.worktree_root.as_deref() != scope.root.to_str()
    {
        return Err("Review child checkout binding changed".into());
    }
    WorktreeService::new_async(app.gcx.cache_dir.clone(), root)
        .await?
        .validate_worktree_meta_strict(&scope)
        .await?;
    let candidate = review
        .candidates
        .iter()
        .find(|c| c.id == run.candidate_id)
        .ok_or("Pinned candidate unavailable")?;
    let origins = candidate
        .files
        .iter()
        .map(|f| PathBuf::from(&candidate.repository).join(&f.path))
        .collect::<Vec<_>>();
    for path in &origins {
        crate::files_in_workspace::check_file_privacy_for_model_context(app.gcx.clone(), path)
            .await?;
    }
    let records = crate::privacy::records::declared_file_records(&app.gcx, origins)?;
    let derived = session.lock().await.derived_privacy_zones.clone();
    let mut labels = derived.write().map_err(|_| "Privacy labels unavailable")?;
    if let Some(saved) = material.input_digests.get("derived_origin_labels") {
        let saved: std::collections::BTreeMap<String, serde_json::Value> =
            serde_json::from_str(saved).map_err(|e| e.to_string())?;
        for (relative, label) in saved {
            if !std::path::Path::new(&relative)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            {
                return Err("Unsafe retained privacy path".into());
            }
            labels.insert(
                scope.root.join(relative),
                crate::privacy::records::DerivedZone {
                    zone: label
                        .get("zone")
                        .and_then(|z| z.as_str())
                        .ok_or("Missing privacy zone")?
                        .into(),
                    origin: label
                        .get("origin")
                        .and_then(|o| o.as_str())
                        .map(str::to_string),
                },
            );
        }
    }
    for file in &candidate.files {
        let origin = PathBuf::from(&candidate.repository).join(&file.path);
        if let Some(record) = records.iter().find(|r| PathBuf::from(&r.path) == origin) {
            labels.insert(
                scope.root.join(&file.path),
                crate::privacy::records::DerivedZone {
                    zone: record.zone.clone(),
                    origin: Some(record.path.clone()),
                },
            );
        }
    }
    Ok(())
}

pub async fn inspected_candidate(
    ccx: Arc<Mutex<AtCommandsContext>>,
    review: &Review,
) -> std::result::Result<String, String> {
    let c = ccx.lock().await;
    candidate_for_identity(review, &c.chat_id, c.background_agent_id.as_deref())
}

fn candidate_for_identity(
    review: &Review,
    chat: &str,
    agent: Option<&str>,
) -> std::result::Result<String, String> {
    if chat == review.owner_chat_id {
        return review
            .current_candidate_id
            .clone()
            .ok_or("No active candidate".into());
    }
    let agent = agent.ok_or("No runtime review identity")?;
    review
        .runs
        .iter()
        .find(|r| r.agent_id == agent)
        .map(|r| r.candidate_id.clone())
        .ok_or("No pinned run".into())
}

pub async fn prepare_launch(
    app: &AppState,
    req: &mut SpawnRequest,
) -> std::result::Result<Option<LaunchBinding>, String> {
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&req.parent_chat_id)
        .cloned();
    let Some(session) = session else {
        return Ok(None);
    };
    if crate::coordination::scope::session_source_root(&*session.lock().await).is_none() {
        return Ok(None);
    }

    let root = super::service::trusted_root(app, &req.parent_chat_id).await?;
    if let Some(agent) = app
        .agents
        .find_agent_id_by_child_chat_id(&req.parent_chat_id)
        .await
    {
        let record = app.agents.get_any(&agent).await?;
        if let Some(parent_review) =
            super::service::for_agent(root.clone(), record.parent_chat_id, agent.clone())
                .await
                .map_err(|e| e.to_string())?
        {
            if parent_review.runs.iter().any(|r| r.agent_id == agent) {
                return Err("Nested review delegation requires a separately pinned child scope; launch from the owning review chat".into());
            }
        }
    }
    let Some(review) = super::service::get_for_chat(root.clone(), req.parent_chat_id.clone())
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    super::context::verify_binding_for_chat(app, &req.parent_chat_id).await?;
    let candidate = review
        .current_candidate_id
        .clone()
        .ok_or("No active review candidate")?;
    let source = req
        .parent_worktree
        .as_ref()
        .ok_or("Review execution checkout missing")?
        .root
        .clone();
    let (snapshot, files, artifact) =
        super::evidence::capture_saved_state(root.clone(), source).await?;
    for file in &snapshot.files {
        crate::files_in_workspace::check_file_privacy_for_model_context(
            app.gcx.clone(),
            &PathBuf::from(&snapshot.repository).join(&file.path),
        )
        .await?;
    }
    let worktree = WorktreeService::new_async(app.gcx.cache_dir.clone(), root.clone())
        .await?
        .create_review_snapshot(&req.parent_chat_id, &review.id, files)
        .await?
        .worktree
        .meta;
    let mut inputs = std::collections::BTreeMap::new();
    inputs.insert("saved_state".into(), snapshot.content_digest.clone());
    inputs.insert(
        "execution_commit".into(),
        worktree
            .base_commit
            .clone()
            .ok_or("Missing review snapshot commit")?,
    );
    let parent_labels = session.lock().await.derived_privacy_zones.clone();
    let remapped = {
        let labels = parent_labels
            .read()
            .map_err(|_| "Parent privacy labels unavailable")?;
        snapshot
            .files
            .iter()
            .filter_map(|f| {
                labels
                    .get(&PathBuf::from(&snapshot.repository).join(&f.path))
                    .map(|label| (f.path.clone(), label.clone()))
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    inputs.insert(
        "derived_origin_labels".into(),
        serde_json::to_string(&remapped).map_err(|e| e.to_string())?,
    );
    let material = Material {
        id: uuid::Uuid::new_v4().to_string(),
        kind: MaterialKind::Investigation,
        candidate_id: candidate.clone(),
        agent_id: None,
        description:
            "Accepted isolated reviewer input; saved local state, not immutable imported comparison"
                .into(),
        content_digest: snapshot.content_digest,
        input_digests: inputs,
        environment: None,
        worktree_id: Some(worktree.id.clone()),
        worktree_root: Some(worktree.root.to_string_lossy().into()),
        artifacts: vec![artifact],
    };
    // Persist the exact scope before accepted launch; this also fences queued setup.
    let binding = LaunchBinding {
        root: root.clone(),
        review_id: review.id.clone(),
        owner: review.owner_chat_id.clone(),
        candidate: candidate.clone(),
        material: material.clone(),
    };
    apply(
        root,
        &review.id,
        Operation::AddMaterial {
            material,
        },
    )
    .await?;
    req.parent_worktree = Some(worktree);
    req.worktree_mode = SpawnWorktreeMode::Inherit;
    req.prompt.push_str(&format!("\n\n# Runtime Review Binding\nReview {}. Inspect only candidate {} using your accepted isolated checkout. It includes saved investigation state; it is NOT proof of the original imported candidate. Candidate updates in the owning chat do not retarget this run. Retain repairs as material; never merge or publish automatically.\n", review.id, candidate));
    req.prompt.push_str(REVIEW_CRITICISM_CONTRACT);
    Ok(Some(binding))
}

// Content guidance only; candidate/material/checkout authority remains runtime-owned above.
const REVIEW_CRITICISM_CONTRACT: &str = r#"

# Review criticism contract
Actually inspect applicable scoped repository rules, relevant docs/specifications, nearby prior art,
callers and tests before judging. Cite actual rules and source anchors; disclose unavailable sources.
Compare the pinned candidate with its captured base: identify introduced or worsened behavior, not
unrelated existing debt. Find ordinary bugs and concrete performance regressions, not just architecture
concerns. Proactively challenge unnecessary defensive checks, speculative fallbacks, silent recovery,
repeated conversions, missed reuse and overabstracted AI scaffolding. Stricter is not automatically better.
Verify trust boundaries/caller guarantees and existing helper semantics, errors and side effects before
calling checks redundant or recommending reuse. For performance establish reachability, workload,
frequency and scaling cost; distinguish hot paths from cold setup. There is no finding quota.

Discover independently before exchanging findings. Then perform a separate ADVERSARIAL refutation pass
on every claim: already in base? caller prevents trigger? suggested reuse really equivalent? necessary
trust-boundary validation? Seek the strongest source counterevidence. Preserve supported, refuted and
unresolved outcomes as attributed canonical annotations/replies, including disagreements; never vote.
Use supported/refuted assessments only when established; unresolved is disputed or unassessed with limits.
Each final finding body must explicitly state Delta, Trigger (reachable input/state/caller), Impact,
Counterevidence (sources inspected and attempted refutation), Remedy (smallest fix and inspected existing
reuse or why lookalikes differ), and Evidence status (source-supported vs actually executed, exact
candidate/material, unavailable checks). Use review_anchor and registered evidence IDs; retain run
attribution and existing histories/contributions. Do not fabricate execution or promote investigation
results into proof of imported content. Only review_record_evidence records execution-registry results.
Contribute findings, source evidence and canonical replies; the owning foreground chat authors Tour.
"#;

pub async fn is_pinned_launch(
    app: &AppState,
    root: &std::path::Path,
    owner: &str,
    worktree: Option<&WorktreeMeta>,
) -> std::result::Result<bool, String> {
    let Some(worktree) = worktree else {
        return Ok(false);
    };
    let Some(review) =
        super::service::for_worktree(root.to_path_buf(), owner.into(), worktree.id.clone())
            .await
            .map_err(|e| e.to_string())?
    else {
        return Ok(false);
    };
    let valid = worktree.kind == "review"
        && review.materials.iter().any(|m| {
            m.kind == MaterialKind::Investigation
                && m.worktree_id.as_deref() == Some(&worktree.id)
                && m.worktree_root.as_deref() == worktree.root.to_str()
        });
    if valid {
        WorktreeService::new_async(app.gcx.cache_dir.clone(), root.into())
            .await?
            .validate_worktree_meta_strict(worktree)
            .await?;
    }
    Ok(valid)
}

/// Install the registry identity before the runner can perform any review tools.
pub async fn record_accepted_launch(
    app: &AppState,
    req: &SpawnRequest,
    record: &BackgroundAgent,
) -> std::result::Result<(), String> {
    let Some(worktree) = req.parent_worktree.as_ref().filter(|w| w.kind == "review") else {
        return Ok(());
    };
    let root = worktree.source_workspace_root.clone();
    let Some(review) = super::service::for_worktree(
        root.clone(),
        req.parent_chat_id.clone(),
        worktree.id.clone(),
    )
    .await
    .map_err(|e| e.to_string())?
    else {
        return Err("Accepted review checkout has no retained owning review".into());
    };
    let Some(mut material) = review
        .materials
        .iter()
        .find(|m| {
            m.worktree_id.as_deref() == Some(&worktree.id)
                && m.description
                    .starts_with("Accepted isolated reviewer input")
        })
        .cloned()
    else {
        return Err("Review launch requires a separately retained isolated input".into());
    };
    if record.parent_chat_id != review.owner_chat_id {
        return Err("Review registry owner mismatch".into());
    }
    material.id = uuid::Uuid::new_v4().to_string();
    material.agent_id = Some(record.agent_id.clone());
    let candidate = material.candidate_id.clone();
    apply(
        root.clone(),
        &review.id,
        Operation::AddMaterial {
            material,
        },
    )
    .await?;
    apply(
        root,
        &review.id,
        Operation::PutRun {
            run: runtime_run(record, uuid::Uuid::new_v4().to_string(), candidate),
        },
    )
    .await?;
    Ok(())
}

pub async fn record_launch(
    app: &AppState,
    mut binding: LaunchBinding,
    handle: &SpawnHandle,
) -> std::result::Result<Review, String> {
    let existing = super::service::load_owned(
        binding.root.clone(),
        binding.owner.clone(),
        binding.review_id.clone(),
    )
    .await
    .map_err(|e| e.to_string())?;
    if existing
        .runs
        .iter()
        .any(|r| r.agent_id == handle.agent_id && r.candidate_id == binding.candidate)
    {
        return Ok(existing);
    }

    let record = app.agents.get_any(&handle.agent_id).await?;
    if record.parent_chat_id != binding.owner
        || record.child_chat_id.as_deref() != Some(&handle.child_chat_id)
    {
        return Err("Accepted reviewer owner mismatch".into());
    }
    binding.material.agent_id = Some(record.agent_id.clone());
    // Material identities are immutable; add a runtime-linked copy rather than rewriting launch input.
    binding.material.id = uuid::Uuid::new_v4().to_string();
    apply(
        binding.root.clone(),
        &binding.review_id,
        Operation::AddMaterial {
            material: binding.material,
        },
    )
    .await?;
    let run = runtime_run(&record, uuid::Uuid::new_v4().to_string(), binding.candidate);
    apply(
        binding.root,
        &binding.review_id,
        Operation::PutRun {
            run,
        },
    )
    .await
}

fn runtime_run(record: &BackgroundAgent, id: String, candidate_id: String) -> Run {
    let status = match record.status {
        BgAgentStatus::Queued => RunStatus::Queued,
        BgAgentStatus::Running => RunStatus::Running,
        BgAgentStatus::Completed => RunStatus::Completed,
        BgAgentStatus::Failed => RunStatus::Failed,
        BgAgentStatus::Cancelled => RunStatus::Cancelled,
        BgAgentStatus::Interrupted | BgAgentStatus::Closed => RunStatus::Interrupted,
        BgAgentStatus::WaitingForApproval => RunStatus::Running,
    };
    Run {
        id,
        candidate_id,
        agent_id: record.agent_id.clone(),
        purpose: record.title.chars().take(256).collect(),
        model: Some(record.model.clone()),
        domain: record.worktree_id.clone(),
        status,
        started_at_ms: record
            .started_at
            .unwrap_or(record.created_at)
            .timestamp_millis(),
        finished_at_ms: record.finished_at.map(|t| t.timestamp_millis()),
        error: record.error.clone(),
    }
}

pub async fn refresh_runs(
    app: &AppState,
    root: PathBuf,
    review: Review,
) -> std::result::Result<Review, String> {
    let mut latest = review.clone();
    for run in &review.runs {
        if run.status == RunStatus::CancelRequested {
            let _ = app
                .agents
                .cancel(
                    &review.owner_chat_id,
                    &run.agent_id,
                    Some("Review cancellation requested".into()),
                )
                .await;
        }

        if run.finished_at_ms.is_some() {
            continue;
        }
        let updated = match app.agents.get_any(&run.agent_id).await {
            Ok(record) if record.parent_chat_id == review.owner_chat_id => {
                let mut updated = runtime_run(&record, run.id.clone(), run.candidate_id.clone());
                updated.started_at_ms = run.started_at_ms;
                if run.status == RunStatus::CancelRequested && !record.status.is_terminal() {
                    updated.status = RunStatus::CancelRequested;
                }

                updated
            }
            _ => {
                let mut interrupted = run.clone();
                interrupted.status = RunStatus::Interrupted;
                interrupted.finished_at_ms = Some(chrono::Utc::now().timestamp_millis());
                interrupted.error = Some("Runtime registry identity unavailable; retained historical review input and result are not retargeted".into());
                interrupted
            }
        };
        if &updated != run {
            latest = apply(
                root.clone(),
                &review.id,
                Operation::PutRun {
                    run: updated,
                },
            )
            .await?;
        }
    }
    Ok(latest)
}

pub(crate) async fn apply(
    root: PathBuf,
    review_id: &str,
    operation: Operation,
) -> std::result::Result<Review, String> {
    apply_with_privacy(root, review_id, operation, vec![]).await
}

pub(crate) async fn apply_with_privacy(
    root: PathBuf,
    review_id: &str,
    operation: Operation,
    privacy: Vec<refact_privacy::FileRecord>,
) -> std::result::Result<Review, String> {
    let id = review_id.to_owned();
    super::service::with_store(root, move |store| {
        let review = store.load(&id)?;
        store.mutate_with_privacy(
            &id,
            Mutation {
                expected_revision: review.revision,
                request_key: uuid::Uuid::new_v4().to_string(),
                actor: Actor {
                    kind: ActorKind::System,
                    id: "review-runtime".into(),
                },
                at_ms: chrono::Utc::now().timestamp_millis(),
                operation,
            },
            privacy,
        )
    })
    .await
    .map_err(|e| e.to_string())
}
