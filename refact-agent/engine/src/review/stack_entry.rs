use std::collections::{HashMap, HashSet};
use refact_chat_api::PendingQuestion;
use refact_review_core::*;
use serde::{Deserialize, Serialize};
use crate::app_state::AppState;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonMode {
    LiveChain,
    RecordedComparisons,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryMember {
    pub key: String,
    pub branch_ref: String,
    pub pr_identity: Option<PrIdentity>,
    pub parent: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRequest {
    pub title: String,
    pub source: StackSource,
    pub selected_member: String,
    pub comparison_mode: ComparisonMode,
    pub members: Vec<DiscoveryMember>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemberComparison {
    pub member: String,
    pub base_ref: String,
    pub head_ref: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcquisitionRequest {
    pub preparation_token: String,
    pub scope_receipt_tool_call_id: String,
    pub request_key: String,
    pub root_base_ref: Option<String>,
    pub comparisons: Vec<MemberComparison>,
}
#[derive(Serialize)]
pub struct PreparedStack {
    pub status: &'static str,
    pub manifest: StackManifest,
    pub question: PendingQuestion,
    pub ask_questions: serde_json::Value,
    pub fingerprint: String,
    pub acquisition: &'static str,
}
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn historical(m: &StackManifest) -> bool {
    m.members.iter().any(|m| {
        m.reason
            .as_deref()
            .is_some_and(|r| r.starts_with("entry:recorded_comparisons:"))
    })
}
pub(super) fn is_historical(m: &StackManifest) -> bool {
    historical(m)
}
fn member_key(m: &StackMember) -> &str {
    m.reason
        .as_deref()
        .and_then(|r| r.splitn(3, ':').nth(2))
        .unwrap_or("")
}
fn question(manifest: &StackManifest, token: &str) -> PendingQuestion {
    let selected = manifest
        .members
        .iter()
        .find(|m| m.id == manifest.scope_choice.selected_member_id)
        .unwrap();
    PendingQuestion {
        id: format!("review-stack-scope:{token}"),
        question_type: "single_select".into(),
        text: format!(
            "Selected target `{}` belongs to known stack **{}** ({} members), source {}: {}. {} Review only this PR, or the whole stack? No comparison has been acquired or captured.",
            selected.branch_ref,
            manifest.title,
            manifest.members.len(),
            manifest.source.kind,
            manifest.source.reference.as_deref().unwrap_or(""),
            if historical(manifest) {
                "These are explicitly recorded historical comparisons, not live parent-relative comparisons; cumulative projection is unavailable."
            } else {
                "Live comparisons must retain strict parent pins."
            }
        ),
        options: Some(vec![
            "Review this PR only".into(),
            "Review the whole stack".into(),
        ]),
    }
}
fn discovery(
    request: DiscoveryRequest,
    owner: &str,
    repository: String,
) -> std::result::Result<StackManifest, String> {
    let mut errors = vec![];
    if request.title.trim().is_empty() {
        errors.push("title is required".to_string());
    }
    if !["provided_manifest", "graphite_cli"].contains(&request.source.kind.as_str())
        || request
            .source
            .reference
            .as_deref()
            .is_none_or(|r| r.trim().is_empty())
    {
        errors.push("source.kind must be provided_manifest or graphite_cli and source.reference must disclose actually acquired membership evidence (CLI version/capability for Graphite)".into());
    }
    if request.members.is_empty() || request.members.len() > 200 {
        errors.push("members must contain 1..200 known targets".into());
    }
    let mut ids = HashMap::new();
    let mut branches = HashSet::new();
    let mut prs = HashSet::new();
    for (i, m) in request.members.iter().enumerate() {
        if m.key.trim().is_empty() || ids.contains_key(&m.key) {
            errors.push(format!("members[{i}].key must be nonempty and unique"));
        }
        if m.branch_ref.trim().is_empty() || !branches.insert(&m.branch_ref) {
            errors.push(format!(
                "members[{i}].branch_ref must be nonempty and unique"
            ));
        }
        if let Some(pr) = &m.pr_identity {
            if pr.host.is_empty()
                || pr.repository.is_empty()
                || pr.number == 0
                || !prs.insert((
                    pr.host.to_lowercase(),
                    pr.repository.to_lowercase(),
                    pr.number,
                ))
            {
                errors.push(format!("members[{i}].pr_identity invalid or duplicate"));
            }
        }
        if m.parent.as_ref().is_some_and(|p| !ids.contains_key(p)) {
            errors.push(format!(
                "members[{i}].parent must name an earlier member key"
            ));
        }
        ids.insert(m.key.clone(), uuid::Uuid::new_v4().to_string());
    }
    if !ids.contains_key(&request.selected_member) {
        errors.push("selected_member must name a member key".into());
    }
    if !errors.is_empty() {
        return Err(format!(
            "Invalid stack discovery:\n- {}",
            errors.join("\n- ")
        ));
    }
    let mode = if request.comparison_mode == ComparisonMode::RecordedComparisons {
        "recorded_comparisons"
    } else {
        "live_chain"
    };
    Ok(StackManifest {
        schema_version: SCHEMA_VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        owner_chat_id: owner.into(),
        revision: 0,
        title: request.title,
        repository,
        source: request.source,
        root_base_ref: String::new(),
        root_base_oid: None,
        scope_choice: StackScopeChoice {
            scope: StackScope::Single,
            selected_member_id: ids[&request.selected_member].clone(),
            confirmed: false,
            explicit_request: false,
        },
        members: request
            .members
            .into_iter()
            .map(|m| StackMember {
                id: ids[&m.key].clone(),
                branch_ref: m.branch_ref,
                pr_identity: m.pr_identity,
                parent_member_id: m.parent.map(|p| ids[&p].clone()),
                declared_base_ref: String::new(),
                resolved_base_oid: None,
                head_oid: None,
                review_id: None,
                state: StackMemberState::Missing,
                reason: Some(format!("entry:{mode}:{}", m.key)),
            })
            .collect(),
    })
}
pub async fn prepare(
    app: &AppState,
    chat: &str,
    request: DiscoveryRequest,
) -> std::result::Result<PreparedStack, String> {
    let root = super::service::trusted_root(app, chat).await?;
    crate::files_in_workspace::check_file_privacy_for_model_context(app.gcx.clone(), &root).await?;
    let manifest = discovery(request, chat, root.to_string_lossy().into())?;
    let fingerprint = format!(
        "entry:{}:{}",
        uuid::Uuid::new_v4(),
        content_hash(&serde_json::to_vec(&manifest).map_err(|e| e.to_string())?)
    );
    let owner = chat.to_string();
    let saved = manifest.clone();
    let token = fingerprint.clone();
    super::service::with_store(root, move |s| {
        s.put_stack_preparation(&owner, &token, &saved)
    })
    .await
    .map_err(|e| e.to_string())?;
    let question = question(&manifest, &fingerprint);
    let ask_questions = serde_json::json!({"questions":[{"id":question.id,"type":question.question_type,"text":question.text,"options":question.options}]});
    Ok(PreparedStack {
        status: "pending_scope_choice",
        question,
        ask_questions,
        manifest,
        fingerprint,
        acquisition: "Ask the exact returned question with ask_questions. After its actual owning-chat answer, provide comparisons ONLY for chosen scope, keyed by discovery member key. No network, credentials, checkout, restack or Graphite mutations. Recorded historical comparisons must use source-disclosed immutable base/head OIDs. Live single may omit root_base_ref: its explicit base/head comparison must retain the declared parent identity, without acquiring peers.",
    })
}
fn acquire(
    mut manifest: StackManifest,
    scope: StackScope,
    request: &AcquisitionRequest,
) -> Result<StackManifest> {
    let selected: Vec<_> = manifest
        .members
        .iter()
        .filter(|m| scope == StackScope::Whole || m.id == manifest.scope_choice.selected_member_id)
        .map(|m| member_key(m).to_owned())
        .collect();
    let mut comparisons = HashMap::new();
    let mut errors = vec![];
    for (i, c) in request.comparisons.iter().enumerate() {
        if !selected.contains(&c.member) || comparisons.insert(c.member.clone(), c).is_some() {
            errors.push(format!(
                "comparisons[{i}].member is duplicate or outside chosen scope"
            ));
        }
        if c.base_ref.is_empty()
            || c.head_ref.is_empty()
            || c.base_ref.len() > 1024
            || c.head_ref.len() > 1024
        {
            errors.push(format!("comparisons[{i}] requires valid base_ref/head_ref"));
        }
    }
    for (i, key) in selected.iter().enumerate() {
        if !comparisons.contains_key(key) {
            errors.push(format!(
                "missing comparison for selected member at index {i}"
            ));
        }
    }
    if !errors.is_empty() {
        return Err(invalid(errors.join("; ")));
    }
    let repo = git2::Repository::open(&manifest.repository)?;
    let resolve = |r: &str| -> Result<String> {
        Ok(repo.revparse_single(r)?.peel_to_commit()?.id().to_string())
    };
    let recorded = historical(&manifest);
    if let Some(root) = &request.root_base_ref {
        manifest.root_base_ref = root.clone();
        manifest.root_base_oid = Some(resolve(root)?);
    }
    for m in &mut manifest.members {
        if let Some(c) = comparisons.get(member_key(m)) {
            if recorded
                && (git2::Oid::from_str(&c.base_ref).is_err()
                    || git2::Oid::from_str(&c.head_ref).is_err())
            {
                return Err(invalid(
                    "Recorded historical comparisons require immutable base/head OIDs disclosed by the source",
                ));
            }
            if !recorded && resolve(&c.head_ref)? != resolve(&m.branch_ref)? {
                return Err(invalid(
                    "Live head_ref must equal the selected target branch head",
                ));
            }
            m.declared_base_ref = c.base_ref.clone();
            m.resolved_base_oid = Some(resolve(&c.base_ref)?);
            m.head_oid = Some(resolve(&c.head_ref)?);
            m.state = StackMemberState::Ready;
        }
    }
    if !recorded {
        for m in manifest
            .members
            .iter()
            .filter(|m| selected.contains(&member_key(m).to_string()))
        {
            let expected = match &m.parent_member_id {
                Some(id) => {
                    let parent = manifest
                        .members
                        .iter()
                        .find(|p| &p.id == id)
                        .ok_or_else(|| invalid("parent missing"))?;
                    if scope == StackScope::Whole {
                        parent.head_oid.clone()
                    } else {
                        // Resolve only the explicitly chosen base; never touch a peer ref.
                        if m.declared_base_ref != parent.branch_ref {
                            return Err(invalid(
                                "Live single base_ref must identify its declared parent branch; use recorded_comparisons for historical pins",
                            ));
                        }
                        m.resolved_base_oid.clone()
                    }
                }
                None => manifest.root_base_oid.clone().or_else(|| {
                    if scope == StackScope::Single {
                        m.resolved_base_oid.clone()
                    } else {
                        None
                    }
                }),
            };
            if expected.is_none() || expected != m.resolved_base_oid {
                return Err(invalid(
                    "Live comparison base must equal pinned parent head/root; whole live scope requires root_base_ref",
                ));
            }
        }
    }
    manifest.scope_choice.scope = scope;
    manifest.scope_choice.confirmed = true;
    manifest.scope_choice.explicit_request = true;
    Ok(manifest)
}
pub async fn import(
    app: &AppState,
    chat: &str,
    request: AcquisitionRequest,
) -> std::result::Result<StackManifest, String> {
    if request.request_key.is_empty() || request.request_key.len() > 256 {
        return Err("request_key must contain 1..256 characters".into());
    }
    let root = super::service::trusted_root(app, chat).await?;
    let owner = chat.to_string();
    let (token, manifest) =
        super::service::with_store(root.clone(), move |s| s.get_stack_preparation(&owner))
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Prepare stack discovery first")?;
    if token != request.preparation_token || !token.starts_with("entry:") {
        return Err("Stack preparation superseded; ask the current question".into());
    }
    let expected = question(&manifest, &token);
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
        super::stacks::receipt_scope(
            &session.pending_questions,
            &request.scope_receipt_tool_call_id,
            &expected,
        )
        .map_err(|e| e.to_string())?
    };
    publish(app, chat, root, token, manifest, scope, request).await
}

pub struct OpenMember {
    pub key: String,
    pub branch_ref: String,
    pub pr: Option<u64>,
    pub base_oid: String,
    pub head_oid: String,
    pub branch_is_head: bool,
}

pub struct OpenStack {
    pub repository: String,
    pub title: String,
    pub reference: String,
    pub selected: String,
    pub identity: Option<(String, String)>,
    pub members: Vec<OpenMember>,
    pub request_key: String,
}

/// Whole-stack review is the product default, so opening one needs no scope question.
/// A strictly linear chain of live branch heads keeps its cumulative comparison.
pub async fn open_whole(
    app: &AppState,
    chat: &str,
    request: OpenStack,
) -> std::result::Result<StackManifest, String> {
    let root = super::service::trusted_root(app, chat).await?;
    crate::files_in_workspace::check_file_privacy_for_model_context(app.gcx.clone(), &root).await?;
    let live = request.members.iter().enumerate().all(|(index, member)| {
        member.branch_is_head
            && (index == 0 || request.members[index - 1].head_oid == member.base_oid)
    });
    let root_base = request
        .members
        .first()
        .map(|member| member.base_oid.clone())
        .ok_or("`stack` must list at least one pull request")?;
    let comparisons = request
        .members
        .iter()
        .map(|member| MemberComparison {
            member: member.key.clone(),
            base_ref: member.base_oid.clone(),
            head_ref: member.head_oid.clone(),
        })
        .collect();
    let mut parent = None;
    let members = request
        .members
        .into_iter()
        .map(|member| DiscoveryMember {
            pr_identity: member.pr.zip(request.identity.clone()).map(
                |(number, (host, repository))| PrIdentity {
                    host,
                    repository,
                    number,
                },
            ),
            parent: parent.replace(member.key.clone()),
            key: member.key,
            branch_ref: member.branch_ref,
        })
        .collect();
    let manifest = discovery(
        DiscoveryRequest {
            title: request.title,
            source: StackSource {
                kind: "provided_manifest".into(),
                reference: Some(request.reference),
            },
            selected_member: request.selected,
            comparison_mode: if live {
                ComparisonMode::LiveChain
            } else {
                ComparisonMode::RecordedComparisons
            },
            members,
        },
        chat,
        request.repository,
    )?;
    let token = format!(
        "entry:{}:{}",
        uuid::Uuid::new_v4(),
        content_hash(&serde_json::to_vec(&manifest).map_err(|e| e.to_string())?)
    );
    let (owner, saved, nonce) = (chat.to_string(), manifest.clone(), token.clone());
    super::service::with_store(root.clone(), move |s| {
        s.put_stack_preparation(&owner, &nonce, &saved)
    })
    .await
    .map_err(|e| e.to_string())?;
    let acquisition = AcquisitionRequest {
        preparation_token: token.clone(),
        scope_receipt_tool_call_id: String::new(),
        request_key: request.request_key,
        root_base_ref: live.then_some(root_base),
        comparisons,
    };
    publish(
        app,
        chat,
        root,
        token,
        manifest,
        StackScope::Whole,
        acquisition,
    )
    .await
}

async fn publish(
    app: &AppState,
    chat: &str,
    root: std::path::PathBuf,
    token: String,
    manifest: StackManifest,
    scope: StackScope,
    request: AcquisitionRequest,
) -> std::result::Result<StackManifest, String> {
    let acquisition_fingerprint =
        content_hash(&serde_json::to_vec(&(&request, &scope)).map_err(|e| e.to_string())?);
    let replay_owner = chat.to_string();
    let replay_token = token.clone();
    let replay_key = request.request_key.clone();
    let replay_fp = acquisition_fingerprint.clone();
    if let Some(result) = super::service::with_store(root.clone(), move |s| {
        s.replay_stack_entry_import(&replay_owner, &replay_token, &replay_key, &replay_fp)
    })
    .await
    .map_err(|e| e.to_string())?
    {
        return Ok(result);
    }
    crate::files_in_workspace::check_file_privacy_for_model_context(app.gcx.clone(), &root).await?;
    let req = request.clone();
    let manifest =
        super::service::with_store(root.clone(), move |_| acquire(manifest, scope, &req))
            .await
            .map_err(|e| e.to_string())?;
    let staged = manifest.clone();
    let candidates = super::service::with_store(root.clone(), move |s| {
        staged
            .members
            .iter()
            .filter(|m| {
                staged.scope_choice.scope == StackScope::Whole
                    || m.id == staged.scope_choice.selected_member_id
            })
            .map(|m| {
                Ok((
                    m.id.clone(),
                    s.capture(
                        std::path::Path::new(&staged.repository),
                        CaptureRequest {
                            base_oid: m
                                .resolved_base_oid
                                .clone()
                                .ok_or_else(|| invalid("base pin missing"))?,
                            head_oid: m
                                .head_oid
                                .clone()
                                .ok_or_else(|| invalid("head pin missing"))?,
                            policy: CapturePolicy::default(),
                            origin: Origin::Imported,
                        },
                    )?,
                ))
            })
            .collect::<Result<Vec<_>>>()
    })
    .await
    .map_err(|e| e.to_string())?;
    for (_, c) in &candidates {
        for f in &c.files {
            crate::files_in_workspace::check_file_privacy_for_model_context(
                app.gcx.clone(),
                &std::path::Path::new(&c.repository).join(&f.path),
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
                    .map(|f| std::path::PathBuf::from(&candidate.repository).join(&f.path)),
            )?);
            Ok((id.clone(), records))
        })
        .collect::<std::result::Result<Vec<_>, String>>()?;
    super::service::with_store(root, move |s| {
        s.publish_stack_entry_acquisition_with_privacy(
            manifest,
            &token,
            &request.request_key,
            candidates,
            &acquisition_fingerprint,
            privacy,
        )
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request() -> DiscoveryRequest {
        serde_json::from_value(json!({"title":"Known stack","source":{"kind":"provided_manifest","reference":"User supplied membership"},"selected_member":"first","comparison_mode":"live_chain","members":[{"key":"first","branch_ref":"first"},{"key":"peer","branch_ref":"inaccessible-peer","parent":"first"}]})).unwrap()
    }
    fn repo() -> (tempfile::TempDir, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(root.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@localhost").unwrap();
        let mut ids = vec![];
        for n in 0..3 {
            let mut builder = repo.treebuilder(None).unwrap();
            builder
                .insert(
                    "file",
                    repo.blob(format!("{n}").as_bytes()).unwrap(),
                    0o100644,
                )
                .unwrap();
            let tree = repo.find_tree(builder.write().unwrap()).unwrap();
            let oid = repo
                .commit(None, &sig, &sig, "fixture", &tree, &[])
                .unwrap();
            ids.push(oid.to_string());
        }
        repo.reference(
            "refs/heads/first",
            git2::Oid::from_str(&ids[1]).unwrap(),
            true,
            "fixture",
        )
        .unwrap();
        repo.reference(
            "refs/heads/second",
            git2::Oid::from_str(&ids[2]).unwrap(),
            true,
            "fixture",
        )
        .unwrap();
        (root, ids)
    }
    fn acquisition(ids: &[String]) -> AcquisitionRequest {
        AcquisitionRequest {
            preparation_token: "entry:token".into(),
            scope_receipt_tool_call_id: "ask".into(),
            request_key: "import".into(),
            root_base_ref: None,
            comparisons: vec![MemberComparison {
                member: "first".into(),
                base_ref: ids[0].clone(),
                head_ref: ids[1].clone(),
            }],
        }
    }
    #[test]
    fn acquisition_rejects_retired_context_metadata() {
        let baseline = serde_json::to_value(acquisition(&["base".into(), "head".into()])).unwrap();
        assert!(serde_json::from_value::<AcquisitionRequest>(baseline.clone()).is_ok());
        for field in ["context_intake", "member_context_intakes"] {
            let mut input = baseline.clone();
            input[field] = json!([]);
            assert!(serde_json::from_value::<AcquisitionRequest>(input).is_err());
        }
    }
    #[test]
    fn public_discovery_requires_no_repository_or_peer_refs_and_aggregates_errors() {
        let m = discovery(request(), "owner", "/not/a/repository".into()).unwrap();
        assert!(m.members.iter().all(|m| m.head_oid.is_none()));
        assert!(question(&m, "nonce").text.contains("`first`"));
        let mut bad = request();
        bad.title.clear();
        bad.source.reference = None;
        bad.selected_member = "absent".into();
        bad.members[1].parent = Some("absent".into());
        let error = discovery(bad, "owner", "repo".into()).unwrap_err();
        for name in ["title", "source", "selected_member", "parent"] {
            assert!(error.contains(name), "{error}");
        }
        let mut protected = serde_json::to_value(request()).unwrap();
        protected["owner_chat_id"] = json!("forged");
        assert!(serde_json::from_value::<DiscoveryRequest>(protected).is_err());
    }
    #[test]
    fn chosen_single_ignores_inaccessible_peer_and_whole_enforces_live_pins() {
        let (root, ids) = repo();
        let m = discovery(request(), "owner", root.path().to_string_lossy().into()).unwrap();
        let one = acquire(m.clone(), StackScope::Single, &acquisition(&ids)).unwrap();
        assert_eq!(one.members[0].state, StackMemberState::Ready);
        assert_eq!(one.members[1], m.members[1]);
        assert!(acquire(m, StackScope::Whole, &acquisition(&ids)).is_err());
        let mut req = request();
        req.members[1].branch_ref = "second".into();
        let m = discovery(req, "owner", root.path().to_string_lossy().into()).unwrap();
        let mut both = acquisition(&ids);
        both.root_base_ref = Some(ids[0].clone());
        both.comparisons.push(MemberComparison {
            member: "peer".into(),
            base_ref: ids[1].clone(),
            head_ref: ids[2].clone(),
        });
        assert!(acquire(m.clone(), StackScope::Whole, &both).is_ok());
        both.comparisons[1].base_ref = ids[0].clone();
        assert!(acquire(m, StackScope::Whole, &both).is_err());
    }
    #[test]
    fn recorded_merged_comparisons_do_not_claim_parent_relative_or_cumulative_dag() {
        let (root, ids) = repo();
        let mut req = request();
        req.comparison_mode = ComparisonMode::RecordedComparisons;
        let m = discovery(req, "owner", root.path().to_string_lossy().into()).unwrap();
        let mut both = acquisition(&ids);
        both.comparisons.push(MemberComparison {
            member: "peer".into(),
            base_ref: ids[0].clone(),
            head_ref: ids[2].clone(),
        });
        let result = acquire(m, StackScope::Whole, &both).unwrap();
        assert_ne!(
            result.members[1].resolved_base_oid,
            result.members[0].head_oid
        );
        assert!(is_historical(&result));
        assert!(question(&result, "nonce")
            .text
            .contains("cumulative projection is unavailable"));
    }

    #[test]
    fn historical_selected_child_survives_deleted_parent() {
        let (root, ids) = repo();
        let mut req = request();
        req.comparison_mode = ComparisonMode::RecordedComparisons;
        req.selected_member = "peer".into();
        req.members[0].branch_ref = "deleted-parent".into();
        let m = discovery(req, "owner", root.path().to_string_lossy().into()).unwrap();
        let mut selected = acquisition(&ids);
        selected.comparisons[0].member = "peer".into();
        let result = acquire(m.clone(), StackScope::Single, &selected).unwrap();
        assert_eq!(result.members[0], m.members[0]);
        assert_eq!(result.members[1].state, StackMemberState::Ready);
    }

    #[tokio::test]
    async fn entry_receipt_retry_before_ref_resolution_and_selected_only_refresh() {
        use refact_chat_api::{
            PendingQuestionSet, PendingQuestionStatus, QuestionResponse, QuestionResponseItem,
            QuestionResponseValue,
        };
        let (root, ids) = repo();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.path().join("cache"),
            root.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let mut session = crate::chat::types::ChatSession::new("owner".into());
        session.work_project_root = Some(root.path().into());
        let source_record: refact_privacy::FileRecord = serde_json::from_value(json!({
            "path":root.path().join("design.md"), "zone":"normal", "attribution":"declared"
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
        let prepared = prepare(&app, "owner", request()).await.unwrap();
        let mut input = acquisition(&ids);
        input.preparation_token = prepared.fingerprint.clone();
        assert!(import(&app, "owner", input.clone()).await.is_err());
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
                resolution_id: Some("user".into()),
                response_protocol_version: 2,
                response: Some(QuestionResponse {
                    answers: vec![QuestionResponseItem {
                        id: prepared.question.id.clone(),
                        value: QuestionResponseValue::Answered {
                            answer: json!("Review this PR only"),
                        },
                        explanation: String::new(),
                    }],
                    overall_comments: String::new(),
                }),
            });
        let published = import(&app, "owner", input.clone()).await.unwrap();
        let store = ReviewStore::open(root.path()).unwrap();
        let member_review = store
            .stack_review_for_member(&published.id, &published.members[0].id)
            .unwrap()
            .unwrap();
        assert!(member_review.privacy.contains(&source_record));
        let candidate = &member_review.candidates[0];
        assert_eq!(candidate.base_oid, ids[0]);
        assert_eq!(candidate.head_oid, ids[1]);
        let repo = git2::Repository::open(root.path()).unwrap();
        repo.find_reference("refs/heads/first")
            .unwrap()
            .delete()
            .unwrap();
        assert_eq!(
            import(&app, "owner", input.clone()).await.unwrap(),
            published
        );
        repo.reference(
            "refs/heads/first",
            git2::Oid::from_str(&ids[2]).unwrap(),
            true,
            "move",
        )
        .unwrap();
        let refreshed =
            super::super::stacks::refresh(&app, "owner", "refresh".into(), published.revision)
                .await
                .unwrap();
        assert_eq!(refreshed.members[1], published.members[1]);
        assert_eq!(
            refreshed.members[0].head_oid.as_deref(),
            Some(ids[2].as_str())
        );
        prepare(&app, "owner", request()).await.unwrap();
        assert!(import(&app, "owner", input).await.is_err());
    }
}
