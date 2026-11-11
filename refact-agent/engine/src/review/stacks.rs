//! Same-chat stack orchestration. A model assertion is never a scope receipt.
//! Acquisition is deliberately external: use existing authorized read-only tools to inspect
//! `gt --version` and `gt log --help`, then provide an explicit manifest. We do not assume
//! Graphite has a stable JSON format or execute network/checkout/restack commands.
use std::{collections::HashSet, path::PathBuf};
use refact_review_core::*;
use refact_chat_api::{
    PendingQuestion, PendingQuestionSet, PendingQuestionStatus, QuestionResponseValue,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::app_state::AppState;

const SINGLE: &str = "Review this PR only";
const WHOLE: &str = "Review the whole stack";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    pub manifest: StackManifest,
    pub scope_receipt_tool_call_id: Option<String>,
    pub request_key: String,
}
#[derive(Serialize)]
pub struct PreparedStack {
    pub status: &'static str,
    pub manifest: StackManifest,
    pub question: PendingQuestion,
    pub fingerprint: String,
    pub acquisition: &'static str,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// Normalize only untrusted consent flags; bind the receipt to every comparison input.
fn question_for(manifest: &StackManifest) -> Result<(String, PendingQuestion)> {
    let mut input = manifest.clone();
    input.scope_choice.confirmed = false;
    input.scope_choice.explicit_request = false;
    input.scope_choice.scope = StackScope::Single;
    let fingerprint = content_hash(&serde_json::to_vec(&input)?);
    let question = PendingQuestion {
        id: format!("review-stack-scope:{fingerprint}"),
        question_type: "single_select".into(),
        text: format!(
            "The selected PR/branch `{}` belongs to the known stack **{}** ({} members). Review only this PR, or the whole stack? No review material will be captured until you choose.",
            input.scope_choice.selected_member_id,
            input.title,
            input.members.len()
        ),
        options: Some(vec![SINGLE.into(), WHOLE.into()]),
    };
    Ok((fingerprint, question))
}

fn validate_parent_pins(manifest: &StackManifest) -> Result<()> {
    for member in &manifest.members {
        let expected = match &member.parent_member_id {
            Some(parent) => manifest
                .members
                .iter()
                .find(|member| &member.id == parent)
                .and_then(|member| member.head_oid.as_ref()),
            None => manifest.root_base_oid.as_ref(),
        };
        if expected.is_none() || member.resolved_base_oid.as_ref() != expected {
            return Err(invalid(
                "Declared member base is not the pinned parent head/root base; restacked or divergent comparisons require a corrected explicit manifest",
            ));
        }
    }
    Ok(())
}

pub(super) fn receipt_scope(
    sets: &[PendingQuestionSet],
    tool_call_id: &str,
    expected: &PendingQuestion,
) -> Result<StackScope> {
    let set = sets
        .iter()
        .find(|set| set.tool_call_id == tool_call_id)
        .ok_or_else(|| {
            invalid("Scope choice requires an actual ask_questions receipt in the owning chat")
        })?;
    if set.status != PendingQuestionStatus::Answered
        || set.resolution_id.as_deref().is_none_or(str::is_empty)
        || set
            .questions
            .iter()
            .filter(|question| *question == expected)
            .count()
            != 1
    {
        return Err(invalid(
            "Scope receipt is pending, cancelled, stale or mismatched",
        ));
    }
    let response = set.response.as_ref().ok_or_else(|| {
        invalid(
            "Explicit answered disposition required; legacy/default answers are not scope consent",
        )
    })?;
    let answers = response
        .answers
        .iter()
        .filter(|answer| answer.id == expected.id)
        .collect::<Vec<_>>();
    if answers.len() != 1 {
        return Err(invalid("Exactly one explicit scope answer required"));
    }
    match &answers[0].value {
        QuestionResponseValue::Answered {
            answer,
        } if answer.as_str() == Some(SINGLE) => Ok(StackScope::Single),
        QuestionResponseValue::Answered {
            answer,
        } if answer.as_str() == Some(WHOLE) => Ok(StackScope::Whole),
        _ => Err(invalid(
            "Skipped, defaulted, custom or ambiguous answers do not authorize stack scope",
        )),
    }
}

pub async fn get(app: &AppState, chat: &str) -> std::result::Result<Option<StackManifest>, String> {
    let root = super::service::trusted_root(app, chat).await?;
    if !root.join(".refact/reviews/reviews.sqlite").exists() {
        return Ok(None);
    }
    let owner = chat.to_owned();
    super::service::with_store(root, move |store| store.get_stack_for_chat(&owner))
        .await
        .map_err(|e| e.to_string())
}

/// Validates an already acquired local manifest without capturing files or changing execution.
pub async fn prepare(
    app: &AppState,
    chat: &str,
    manifest: StackManifest,
) -> std::result::Result<PreparedStack, String> {
    prepare_internal(app, chat, manifest, true).await
}
async fn prepare_internal(
    app: &AppState,
    chat: &str,
    mut manifest: StackManifest,
    persist: bool,
) -> std::result::Result<PreparedStack, String> {
    let root = super::service::trusted_root(app, chat).await?;
    if manifest.owner_chat_id != chat
        || manifest.revision != 0
        || manifest.members.is_empty()
        || manifest.members.len() > 200
    {
        return Err("Invalid stack owner, initial revision or member count".into());
    }
    let repository = dunce::canonicalize(&manifest.repository).map_err(|e| e.to_string())?;
    if repository != root {
        return Err("Stack repository must be the owning chat's trusted source repository".into());
    }
    crate::files_in_workspace::check_file_privacy_for_model_context(app.gcx.clone(), &repository)
        .await?;
    manifest.repository = repository.to_string_lossy().into();
    manifest.scope_choice.confirmed = false;
    manifest.scope_choice.explicit_request = false;
    manifest.scope_choice.scope = StackScope::Single;
    if manifest.source.kind != "provided_manifest" && manifest.source.kind != "graphite_cli" {
        return Err("Disclose stack membership provenance as provided_manifest or graphite_cli; unknown membership must not be guessed".into());
    }
    if manifest
        .source
        .reference
        .as_deref()
        .is_none_or(str::is_empty)
    {
        return Err("Membership source reference (including CLI version/capability when applicable) required".into());
    }
    manifest = super::service::with_store(root, move |_| {
        let repo = git2::Repository::open(&repository)?;
        let resolve = |reference: &str| -> Result<String> {
            if reference.is_empty() || reference.len() > 1024 {
                return Err(invalid("Invalid local git reference"));
            }
            Ok(repo
                .revparse_single(reference)?
                .peel_to_commit()?
                .id()
                .to_string())
        };
        manifest.root_base_oid = Some(resolve(&manifest.root_base_ref)?);
        let mut ids = HashSet::new();
        let mut branches = HashSet::new();
        for member in &mut manifest.members {
            if uuid::Uuid::parse_str(&member.id).is_err()
                || !ids.insert(member.id.clone())
                || !branches.insert(member.branch_ref.clone())
                || member.review_id.is_some()
            {
                return Err(invalid(
                    "Invalid/duplicate member identity or untrusted review binding",
                ));
            }
            if member
                .parent_member_id
                .as_ref()
                .is_some_and(|parent| parent == &member.id || !ids.contains(parent))
            {
                return Err(invalid(
                    "Parents must precede children; cycles are forbidden",
                ));
            }
            member.resolved_base_oid = Some(resolve(&member.declared_base_ref)?);
            member.head_oid = Some(resolve(&member.branch_ref)?);
            member.state = StackMemberState::Ready;
            member.reason = None;
        }
        validate_parent_pins(&manifest)?;
        if !ids.contains(&manifest.scope_choice.selected_member_id) {
            return Err(invalid("Selected member missing"));
        }
        Ok(manifest)
    })
    .await
    .map_err(|e| e.to_string())?;
    let (hash, mut question) = question_for(&manifest).map_err(|e| e.to_string())?;
    let owner = chat.to_owned();
    let root = super::service::trusted_root(app, chat).await?;
    let pending = manifest.clone();
    let fingerprint = if persist {
        let fingerprint = format!("{}:{hash}", uuid::Uuid::new_v4());
        let saved = fingerprint.clone();
        super::service::with_store(root, move |store| {
            store.put_stack_preparation(&owner, &saved, &pending)
        })
        .await
        .map_err(|e| e.to_string())?;
        fingerprint
    } else {
        let (fingerprint, saved) =
            super::service::with_store(root, move |store| store.get_stack_preparation(&owner))
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Prepare a scope question before importing")?;
        if saved != manifest || !fingerprint.ends_with(&format!(":{hash}")) {
            return Err("Prepared comparison changed; ask a fresh scope question".into());
        }
        fingerprint
    };
    question.id = format!("review-stack-scope:{fingerprint}");
    Ok(PreparedStack {
        status: "pending_scope_choice",
        manifest,
        question,
        fingerprint,
        acquisition: "Existing authorized tools only: inspect gt --version and gt log --help before read-only gt log; no JSON capability assumed. Provide explicit local refs and provenance. Never gt get/sync/checkout/restack/submit; no automatic gh/network/credentials.",
    })
}

pub async fn import(
    app: &AppState,
    chat: &str,
    request: ImportRequest,
) -> std::result::Result<StackManifest, String> {
    if request.request_key.is_empty() || request.request_key.len() > 256 {
        return Err("Invalid request key".into());
    }
    let prepared = prepare_internal(app, chat, request.manifest, false).await?;
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat)
        .cloned()
        .ok_or("Owning chat unavailable")?;
    let scope = {
        let session = session.lock().await;
        receipt_scope(
            &session.pending_questions,
            request
                .scope_receipt_tool_call_id
                .as_deref()
                .ok_or("Ask the user single PR vs whole stack with ask_questions first")?,
            &prepared.question,
        )
        .map_err(|e| e.to_string())?
    };
    let mut manifest = prepared.manifest;
    manifest.scope_choice.scope = scope;
    manifest.scope_choice.confirmed = true;
    // True only because the actual user answered, not because the model supplied a bool.
    manifest.scope_choice.explicit_request = true;
    capture_members(
        app,
        chat,
        manifest,
        &prepared.fingerprint,
        &request.request_key,
    )
    .await
}

/// Stage every immutable comparison and privacy check before committing any review candidate.
/// Confirmation, member bindings and candidate journals publish in one token-bound transaction.
async fn capture_members(
    app: &AppState,
    chat: &str,
    manifest: StackManifest,
    preparation: &str,
    key: &str,
) -> std::result::Result<StackManifest, String> {
    let root = super::service::trusted_root(app, chat).await?;
    let selected = manifest
        .members
        .iter()
        .filter(|member| {
            manifest.scope_choice.scope == StackScope::Whole
                || member.id == manifest.scope_choice.selected_member_id
        })
        .cloned()
        .collect::<Vec<_>>();
    let repository = PathBuf::from(&manifest.repository);
    let captured = super::service::with_store(root.clone(), move |store| {
        selected
            .into_iter()
            .map(|member| {
                if member.state != StackMemberState::Ready {
                    return Err(invalid(
                        "Cannot capture unavailable stack member; previous candidates preserved",
                    ));
                }
                let candidate = store.capture(
                    &repository,
                    CaptureRequest {
                        base_oid: member
                            .resolved_base_oid
                            .clone()
                            .ok_or_else(|| invalid("Missing base pin"))?,
                        head_oid: member
                            .head_oid
                            .clone()
                            .ok_or_else(|| invalid("Missing head pin"))?,
                        policy: CapturePolicy::default(),
                        origin: Origin::Imported,
                    },
                )?;
                Ok((member, candidate))
            })
            .collect::<Result<Vec<_>>>()
    })
    .await
    .map_err(|e| e.to_string())?;
    for (_, candidate) in &captured {
        for file in &candidate.files {
            crate::files_in_workspace::check_file_privacy_for_model_context(
                app.gcx.clone(),
                &PathBuf::from(&candidate.repository).join(&file.path),
            )
            .await?;
        }
    }
    let source_privacy = super::context::chat_source_privacy(app, chat).await?;
    let privacy = captured
        .iter()
        .map(|(member, candidate)| {
            let mut records = source_privacy.clone();
            records.extend(crate::privacy::records::declared_file_records(
                &app.gcx,
                candidate
                    .files
                    .iter()
                    .map(|f| PathBuf::from(&candidate.repository).join(&f.path)),
            )?);
            Ok((member.id.clone(), records))
        })
        .collect::<std::result::Result<Vec<_>, String>>()?;
    let preparation = preparation.to_owned();
    let key = key.to_owned();
    super::service::with_store(root, move |store| {
        let candidates = captured
            .into_iter()
            .map(|(member, candidate)| (member.id, candidate))
            .collect();
        store.publish_stack_import_with_privacy(manifest, &preparation, &key, candidates, privacy)
    })
    .await
    .map_err(|e| e.to_string())
}

pub async fn history(
    app: &AppState,
    chat: &str,
) -> std::result::Result<Vec<StackManifest>, String> {
    let manifest = get(app, chat).await?.ok_or("Stack not found")?;
    let root = super::service::trusted_root(app, chat).await?;
    super::service::with_store(root, move |store| store.stack_history(&manifest.id))
        .await
        .map_err(|e| e.to_string())
}

pub async fn member(
    app: &AppState,
    chat: &str,
    member_id: String,
) -> std::result::Result<Option<Review>, String> {
    let manifest = get(app, chat).await?.ok_or("Stack not found")?;
    let root = super::service::trusted_root(app, chat).await?;
    super::service::with_store(root, move |store| {
        store.stack_review_for_member(&manifest.id, &member_id)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Preflight all pins, captures and privacy before atomic manifest and candidate publication.
pub async fn refresh(
    app: &AppState,
    chat: &str,
    request_key: String,
    expected_revision: u64,
) -> std::result::Result<StackManifest, String> {
    let mut manifest = get(app, chat).await?.ok_or("Stack not found")?;
    if super::stack_entry::is_historical(&manifest) {
        return Err("Recorded historical comparisons are immutable; prepare a fresh explicitly disclosed import instead of refreshing live refs".into());
    }
    let root = super::service::trusted_root(app, chat).await?;
    let owner = chat.to_owned();
    let stack_id = manifest.id.clone();
    let retry_key = request_key.clone();
    if let Some(response) = super::service::with_store(root, move |store| {
        store.replay_stack_refresh(&owner, &stack_id, &retry_key, expected_revision)
    })
    .await
    .map_err(|e| e.to_string())?
    {
        return Ok(response);
    }
    if manifest.revision != expected_revision {
        return Err("Stale stack revision".into());
    }
    let repository = PathBuf::from(&manifest.repository);
    crate::files_in_workspace::check_file_privacy_for_model_context(app.gcx.clone(), &repository)
        .await?;
    let root = super::service::trusted_root(app, chat).await?;
    let (manifest, candidates) = super::service::with_store(root.clone(), move |store| {
        let repo = git2::Repository::open(&repository)?;
        let resolve = |reference: &str| -> Result<String> {
            Ok(repo
                .revparse_single(reference)?
                .peel_to_commit()?
                .id()
                .to_string())
        };
        let entry_single = manifest.scope_choice.scope == StackScope::Single
            && manifest.members.iter().any(|m| {
                m.reason
                    .as_deref()
                    .is_some_and(|r| r.starts_with("entry:live_chain:"))
            });
        if !entry_single || !manifest.root_base_ref.is_empty() {
            manifest.root_base_oid = Some(resolve(&manifest.root_base_ref)?);
        }
        for member in &mut manifest.members {
            if member.state == StackMemberState::Removed
                || (entry_single && member.id != manifest.scope_choice.selected_member_id)
            {
                continue;
            }
            member.resolved_base_oid = Some(resolve(&member.declared_base_ref)?);
            member.head_oid = Some(resolve(&member.branch_ref)?);
            if !member
                .reason
                .as_deref()
                .is_some_and(|r| r.starts_with("entry:"))
            {
                member.reason = None;
            }
        }
        if !entry_single {
            validate_parent_pins(&manifest)?;
        }
        let candidates = manifest
            .members
            .iter()
            .filter(|member| {
                member.state != StackMemberState::Removed
                    && (manifest.scope_choice.scope == StackScope::Whole
                        || member.id == manifest.scope_choice.selected_member_id)
            })
            .map(|member| {
                if member.state != StackMemberState::Ready {
                    return Err(invalid("Cannot refresh unavailable member"));
                }
                Ok((
                    member.id.clone(),
                    store.capture(
                        &repository,
                        CaptureRequest {
                            base_oid: member
                                .resolved_base_oid
                                .clone()
                                .ok_or_else(|| invalid("Missing base pin"))?,
                            head_oid: member
                                .head_oid
                                .clone()
                                .ok_or_else(|| invalid("Missing head pin"))?,
                            policy: CapturePolicy::default(),
                            origin: Origin::Imported,
                        },
                    )?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((manifest, candidates))
    })
    .await
    .map_err(|e| e.to_string())?;
    for (_, candidate) in &candidates {
        for file in &candidate.files {
            crate::files_in_workspace::check_file_privacy_for_model_context(
                app.gcx.clone(),
                &PathBuf::from(&candidate.repository).join(&file.path),
            )
            .await?;
        }
    }
    let source_privacy = super::context::chat_source_privacy(app, chat).await?;
    let privacy = candidates
        .iter()
        .map(|(id, candidate)| {
            let mut records = source_privacy.clone();
            records.extend(crate::privacy::records::declared_file_records(
                &app.gcx,
                candidate
                    .files
                    .iter()
                    .map(|f| PathBuf::from(&candidate.repository).join(&f.path)),
            )?);
            Ok((id.clone(), records))
        })
        .collect::<std::result::Result<Vec<_>, String>>()?;
    super::service::with_store(root, move |store| {
        store.publish_stack_refresh_with_privacy(
            manifest,
            expected_revision,
            &request_key,
            candidates,
            privacy,
        )
    })
    .await
    .map_err(|e| e.to_string())
}

/// Explicit activation stays exclusively on the trusted live root tool path.
pub async fn capture(
    ccx: std::sync::Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>>,
    member_id: String,
    request_key: String,
) -> std::result::Result<super::context::ContextReceipt, String> {
    let (app, chat) = {
        let context = ccx.lock().await;
        if context.background_agent_id.is_some()
            || context.subchat_depth != 0
            || context.task_meta.is_some()
        {
            return Err("Only the owning root may select stack execution context".into());
        }
        (context.app.clone(), context.chat_id.clone())
    };
    let stack = get(&app, &chat).await?.ok_or("Stack not found")?;
    if !stack.scope_choice.confirmed
        || (stack.scope_choice.scope == StackScope::Single
            && stack.scope_choice.selected_member_id != member_id)
    {
        return Err("Member outside confirmed user scope".into());
    }
    let member = stack
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or("Member not found")?
        .clone();
    let base_ref = member
        .resolved_base_oid
        .clone()
        .ok_or("Pinned member base missing")?;
    let candidate_ref = member
        .head_oid
        .clone()
        .ok_or("Pinned member head missing")?;
    let root = super::service::trusted_root(&app, &chat).await?;
    let key = format!("member:{request_key}");
    let sid = stack.id.clone();
    let mid = member_id.clone();
    let title = member.branch_ref.clone();
    let review = super::service::with_store(root, move |store| {
        store.create_member_review(&sid, &mid, stack.revision, &title, Origin::Imported, &key)
    })
    .await
    .map_err(|e| e.to_string())?;
    super::context::set_context(
        ccx,
        super::context::SetContextRequest {
            repository: stack.repository,
            base_ref,
            candidate_ref,
            policy: CapturePolicy::default(),
            target_reference: member.branch_ref,
            title: review.title.clone(),
            review_id: Some(review.id),
            expected_revision: Some(review.revision),
            request_key,
        },
    )
    .await
}

pub async fn cumulative_file(
    app: &AppState,
    chat: &str,
    member: String,
    path: String,
    expected_revision: u64,
) -> std::result::Result<FileView, String> {
    let value = cumulative(app, chat, member).await?;
    if value["stack_revision"].as_u64() != Some(expected_revision) {
        return Err(
            "Cumulative comparison revision changed; reload comparison before reading files".into(),
        );
    }
    let candidate: Candidate =
        serde_json::from_value(value["candidate"].clone()).map_err(|e| e.to_string())?;
    let root = super::service::trusted_root(app, chat).await?;
    super::service::with_store(root, move |store| store.file(&candidate, &path))
        .await
        .map_err(|e| e.to_string())
}

/// A separate immutable comparison identity, never an annotatable member candidate.
pub async fn cumulative(
    app: &AppState,
    chat: &str,
    member_id: String,
) -> std::result::Result<Value, String> {
    let manifest = get(app, chat).await?.ok_or("Stack not found")?;
    if super::stack_entry::is_historical(&manifest) {
        return Err("Cumulative projection is unavailable for recorded historical comparisons: their DAG cannot honestly be relabeled as a live parent-relative chain".into());
    }
    let member = manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or("Member not found")?;
    if !manifest.scope_choice.confirmed || manifest.scope_choice.scope != StackScope::Whole {
        return Err("Member outside confirmed scope".into());
    }
    let base = manifest
        .root_base_oid
        .clone()
        .ok_or("Pinned root base missing")?;
    let head = member
        .head_oid
        .clone()
        .ok_or("Pinned member head missing")?;
    let root = super::service::trusted_root(app, chat).await?;
    let repository = PathBuf::from(&manifest.repository);
    let candidate = super::service::with_store(root, move |store| {
        store.capture(
            &repository,
            CaptureRequest {
                base_oid: base,
                head_oid: head,
                policy: CapturePolicy::default(),
                origin: Origin::Imported,
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    for file in &candidate.files {
        crate::files_in_workspace::check_file_privacy_for_model_context(
            app.gcx.clone(),
            &PathBuf::from(&candidate.repository).join(&file.path),
        )
        .await?;
    }
    Ok(
        json!({"comparison_id":format!("cumulative:{}:{}:{}",manifest.id,manifest.revision,member_id),"stack_id":manifest.id,"stack_revision":manifest.revision,"member_id":member_id,"read_only":true,"annotation_target":null,"candidate":candidate}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::{QuestionResponse, QuestionResponseItem};
    #[tokio::test]
    async fn whole_import_requires_user_receipt_captures_parent_relative_and_never_activates() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let repo = git2::Repository::init(&root).unwrap();
        let signature = git2::Signature::now("Test", "test@localhost").unwrap();
        let mut heads = Vec::new();
        for content in [
            b"base".as_slice(),
            b"first".as_slice(),
            b"second".as_slice(),
        ] {
            let mut builder = repo.treebuilder(None).unwrap();
            builder
                .insert("file", repo.blob(content).unwrap(), 0o100644)
                .unwrap();
            let tree = repo.find_tree(builder.write().unwrap()).unwrap();
            let parents = heads
                .last()
                .map(|id| repo.find_commit(*id).unwrap())
                .into_iter()
                .collect::<Vec<_>>();
            heads.push(
                repo.commit(
                    None,
                    &signature,
                    &signature,
                    "fixture",
                    &tree,
                    &parents.iter().collect::<Vec<_>>(),
                )
                .unwrap(),
            );
        }
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let app = AppState::from_gcx(gcx).await;
        let mut session = crate::chat::types::ChatSession::new("owner".into());
        session.work_project_root = Some(root.clone());
        let source_record: refact_privacy::FileRecord = serde_json::from_value(json!({
            "path":root.join("design.md"), "zone":"normal", "attribution":"declared"
        }))
        .unwrap();
        let mut source_message = crate::call_validation::ChatMessage::new(
            "tool".into(),
            "Retrieved design context".into(),
        );
        crate::privacy::records::merge_records(&mut source_message, vec![source_record.clone()]);
        session.add_message(source_message);

        let session = std::sync::Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("owner".into(), session.clone());
        let store = ReviewStore::open(&root).unwrap();
        let legacy = store
            .create("owner", "Legacy", Origin::Imported, "legacy")
            .unwrap();
        let first = uuid::Uuid::new_v4().to_string();
        let second = uuid::Uuid::new_v4().to_string();
        let manifest = StackManifest {
            schema_version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            owner_chat_id: "owner".into(),
            revision: 0,
            title: "Fixture stack".into(),
            repository: root.to_string_lossy().into(),
            source: StackSource {
                kind: "provided_manifest".into(),
                reference: Some("test fixture explicitly supplied".into()),
            },
            root_base_ref: heads[0].to_string(),
            root_base_oid: None,
            scope_choice: StackScopeChoice {
                scope: StackScope::Whole,
                selected_member_id: second.clone(),
                confirmed: true,
                explicit_request: true,
            },
            members: vec![
                StackMember {
                    id: first.clone(),
                    branch_ref: heads[1].to_string(),
                    pr_identity: None,
                    parent_member_id: None,
                    declared_base_ref: heads[0].to_string(),
                    resolved_base_oid: None,
                    head_oid: None,
                    review_id: None,
                    state: StackMemberState::Ready,
                    reason: None,
                },
                StackMember {
                    id: second.clone(),
                    branch_ref: heads[2].to_string(),
                    pr_identity: None,
                    parent_member_id: Some(first.clone()),
                    declared_base_ref: heads[1].to_string(),
                    resolved_base_oid: None,
                    head_oid: None,
                    review_id: None,
                    state: StackMemberState::Ready,
                    reason: None,
                },
            ],
        };
        repo.reference("refs/heads/retry-member", heads[2], true, "retry fixture")
            .unwrap();
        let mut manifest = manifest;
        manifest.members[1].branch_ref = "refs/heads/retry-member".into();
        let prepared = prepare(&app, "owner", manifest).await.unwrap();
        assert!(!prepared.manifest.scope_choice.confirmed);
        assert!(store.get_stack_preparation("owner").unwrap().is_some());
        let request = ImportRequest {
            manifest: prepared.manifest.clone(),
            scope_receipt_tool_call_id: Some("ask".into()),
            request_key: "import".into(),
        };
        assert!(import(&app, "owner", request.clone()).await.is_err());
        assert!(store.get_stack_for_chat("owner").unwrap().is_none());
        session
            .lock()
            .await
            .pending_questions
            .push(PendingQuestionSet {
                tool_call_id: "ask".into(),
                questions: vec![prepared.question.clone()],
                asked_at_ms: 1,
                status: PendingQuestionStatus::Answered,
                answers: vec![],
                resolution_id: Some("actual-user-command".into()),
                response_protocol_version: 2,
                response: Some(QuestionResponse {
                    answers: vec![QuestionResponseItem {
                        id: prepared.question.id.clone(),
                        value: QuestionResponseValue::Answered {
                            answer: json!(WHOLE),
                        },
                        explanation: String::new(),
                    }],
                    overall_comments: String::new(),
                }),
            });
        let stack = import(&app, "owner", request.clone()).await.unwrap();
        assert_eq!(stack.members.len(), 2);
        for (index, member) in stack.members.iter().enumerate() {
            let review = store
                .stack_review_for_member(&stack.id, &member.id)
                .unwrap()
                .unwrap();
            assert_ne!(review.id, legacy.id);
            assert!(review.privacy.contains(&source_record));
            assert_eq!(review.candidates.len(), 1);
            let candidate = &review.candidates[0];
            assert_eq!(candidate.base_oid, heads[index].to_string());
            assert_eq!(candidate.head_oid, heads[index + 1].to_string());
        }
        assert_eq!(store.get_for_chat("owner").unwrap().unwrap(), legacy);
        assert_eq!(store.load(&legacy.id).unwrap(), legacy);
        assert!(session.lock().await.thread.worktree.is_none());
        let retried = import(&app, "owner", request.clone()).await.unwrap();
        assert_eq!(retried, stack);
        let mut wrong_parent = prepared.manifest.clone();
        wrong_parent.members[1].declared_base_ref = heads[0].to_string();
        assert!(prepare(&app, "owner", wrong_parent).await.is_err());
        let refreshed = refresh(&app, "owner", "refresh-retry".into(), stack.revision)
            .await
            .unwrap();
        let reviews_before_retry = store.list_for_chat("owner").unwrap();
        repo.reference(
            "refs/heads/retry-member",
            heads[0],
            true,
            "move after success",
        )
        .unwrap();
        assert_eq!(
            refresh(&app, "owner", "refresh-retry".into(), stack.revision)
                .await
                .unwrap(),
            refreshed
        );
        assert_eq!(store.list_for_chat("owner").unwrap(), reviews_before_retry);
        assert!(
            refresh(&app, "owner", "different-key".into(), stack.revision)
                .await
                .is_err()
        );
        repo.reference("refs/heads/retry-member", heads[2], true, "restore fixture")
            .unwrap();
        repo.reference("refs/heads/main", heads[2], true, "execution fixture")
            .unwrap();
        repo.set_head("refs/heads/main").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        let abort = {
            let mut live = session.lock().await;
            live.set_runtime_state(crate::chat::types::SessionState::ExecutingTools, None);
            live.abort_flag.clone()
        };
        let ccx = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new_with_abort(
                app.clone(),
                4096,
                10,
                false,
                vec![],
                "owner".into(),
                None,
                String::new(),
                None,
                None,
                Some(abort),
            )
            .await,
        ));
        let a = capture(ccx.clone(), first.clone(), "activate-a".into())
            .await
            .unwrap();
        let b = capture(ccx.clone(), second.clone(), "activate-b".into())
            .await
            .unwrap();
        let execution_before = session.lock().await.thread.worktree.clone();
        let (_, inactive) = super::super::runs::bound_review(ccx.clone(), Some(&a.review.id))
            .await
            .unwrap();
        assert_eq!(inactive.id, a.review.id);
        use crate::tools::tool_review::{ReviewToolKind, ToolReview};
        use crate::tools::tools_description::Tool;
        // Reading and authoring another stack member never moves the execution checkout.
        let read = serde_json::from_value(json!({"member":first,"path":"file"})).unwrap();
        ToolReview(ReviewToolKind::Read)
            .tool_execute(ccx.clone(), &"inactive-source-read".into(), &read)
            .await
            .unwrap();
        let page = |member: &str| {
            serde_json::from_value(json!({
                "member":member,"id":"orientation","title":"Member tour",
                "summary":"Persisted without changing execution","html":"<p>narrative</p>"
            }))
            .unwrap()
        };
        ToolReview(ReviewToolKind::Tour)
            .tool_execute(ccx.clone(), &"inactive-authoring".into(), &page(&first))
            .await
            .unwrap();
        assert_eq!(store.load(&inactive.id).unwrap().tour.len(), 1);
        assert_eq!(session.lock().await.thread.worktree, execution_before);
        let active = store.load(&b.review.id).unwrap();
        ToolReview(ReviewToolKind::Tour)
            .tool_execute(ccx.clone(), &"active-authoring".into(), &page(&second))
            .await
            .unwrap();
        let authored = store.load(&active.id).unwrap();
        assert_eq!(authored.tour.len(), 1);
        assert!(authored.privacy.contains(&source_record));
        assert_eq!(
            authored.tour[0].current.candidate_id.as_deref(),
            Some(b.candidate_id.as_str())
        );
        assert_eq!(
            store.get_for_chat("owner").unwrap().unwrap().id,
            b.review.id
        );
        assert_eq!(session.lock().await.thread.worktree, execution_before);
        assert!(
            super::super::runs::bound_review(ccx.clone(), Some(&legacy.id))
                .await
                .is_err()
        );
        assert!(
            super::super::runs::bound_review(ccx, Some("foreign-review"))
                .await
                .is_err()
        );
        let fresh = prepare(&app, "owner", prepared.manifest.clone())
            .await
            .unwrap();
        assert_ne!(fresh.question.id, prepared.question.id);
        assert!(
            import(&app, "owner", request).await.is_err(),
            "An older answered receipt cannot satisfy a fresh identical preparation"
        );
        let projection = cumulative(&app, "owner", second.clone()).await.unwrap();
        assert_eq!(projection["candidate"]["base_oid"], heads[0].to_string());
        assert!(
            cumulative_file(&app, "owner", second, "file".into(), refreshed.revision + 1)
                .await
                .is_err()
        );
    }

    #[test]
    fn scope_requires_exact_answered_user_receipt_not_bool_skip_or_stale_question() {
        let question = PendingQuestion {
            id: "scope:fingerprint".into(),
            question_type: "single_select".into(),
            text: "Single or whole?".into(),
            options: Some(vec![SINGLE.into(), WHOLE.into()]),
        };
        let mut set = PendingQuestionSet {
            tool_call_id: "actual-call".into(),
            questions: vec![question.clone()],
            asked_at_ms: 1,
            status: PendingQuestionStatus::Pending,
            answers: vec![],
            resolution_id: None,
            response_protocol_version: 2,
            response: None,
        };
        assert!(receipt_scope(&[set.clone()], "actual-call", &question).is_err());
        set.status = PendingQuestionStatus::Answered;
        set.resolution_id = Some("user-command".into());
        set.response = Some(QuestionResponse {
            answers: vec![QuestionResponseItem {
                id: question.id.clone(),
                value: QuestionResponseValue::Answered {
                    answer: json!(WHOLE),
                },
                explanation: String::new(),
            }],
            overall_comments: String::new(),
        });
        assert_eq!(
            receipt_scope(&[set.clone()], "actual-call", &question).unwrap(),
            StackScope::Whole
        );
        assert!(receipt_scope(&[set.clone()], "model-invented", &question).is_err());
        let mut stale = question.clone();
        stale.id = "scope:other-revision".into();
        assert!(receipt_scope(&[set.clone()], "actual-call", &stale).is_err());
        set.response.as_mut().unwrap().answers[0].value = QuestionResponseValue::Skipped;
        assert!(receipt_scope(&[set.clone()], "actual-call", &question).is_err());
        set.response.as_mut().unwrap().answers[0].value = QuestionResponseValue::Answered {
            answer: json!(true),
        };
        assert!(receipt_scope(&[set], "actual-call", &question).is_err());
    }
}
