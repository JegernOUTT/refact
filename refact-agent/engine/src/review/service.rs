use std::path::PathBuf;
use refact_review_core::*;
use serde::{Deserialize, Serialize};
use crate::app_state::AppState;

pub async fn trusted_root(app: &AppState, chat_id: &str) -> std::result::Result<PathBuf, String> {
    let sessions = app.chat.sessions.clone();
    if !crate::chat::try_restore_session_if_trajectory_exists(app.clone(), &sessions, chat_id).await
    {
        return Err("chat unavailable".into());
    }
    let session = sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("chat unavailable")?;
    let (source, worktree) = {
        let session = session.lock().await;
        if session.closed {
            return Err("chat closed".into());
        }
        (
            crate::coordination::scope::session_source_root(&session)
                .ok_or("chat project unavailable")?,
            session.thread.worktree.clone(),
        )
    };
    let root =
        crate::coordination::roots::resolve_source_workspace_root(app.gcx.clone(), source.to_str())
            .await?;
    if let Some(worktree) = worktree {
        let roots =
            crate::coordination::roots::load_project_roots(app.gcx.clone(), root.clone()).await;
        let registered = roots
            .get(&worktree.id)
            .and_then(|r| r.path.as_ref())
            .ok_or("unregistered worktree")?;
        if dunce::canonicalize(registered).map_err(|e| e.to_string())?
            != dunce::canonicalize(&worktree.root).map_err(|e| e.to_string())?
        {
            return Err("worktree identity mismatch".into());
        }
    }
    Ok(root)
}

pub async fn with_store<T, F>(root: PathBuf, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&ReviewStore) -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let store = ReviewStore::open(&root)?;
        operation(&store)
    })
    .await
    .map_err(|e| Error::Invalid(format!("review worker failed: {e}")))?
}
pub async fn get_for_chat(root: PathBuf, chat_id: String) -> Result<Option<Review>> {
    if !root.join(".refact/reviews/reviews.sqlite").exists() {
        return Ok(None);
    }
    with_store(root, move |s| s.get_for_chat(&chat_id)).await
}
pub async fn load_owned(root: PathBuf, chat_id: String, review_id: String) -> Result<Review> {
    with_store(root, move |s| {
        let r = s.load(&review_id)?;
        if r.owner_chat_id != chat_id {
            return Err(Error::NotFound);
        }
        Ok(r)
    })
    .await
}
pub(crate) async fn activate_owned(
    root: PathBuf,
    owner: String,
    review_id: String,
    expected_active: Option<String>,
    mutation: Mutation,
    privacy: Vec<refact_privacy::FileRecord>,
) -> Result<Review> {
    with_store(root, move |store| {
        store.mutate_and_set_active_with_privacy(
            &owner,
            &review_id,
            expected_active.as_deref(),
            mutation,
            privacy,
        )
    })
    .await
}
pub(crate) async fn for_agent(
    root: PathBuf,
    owner: String,
    agent: String,
) -> Result<Option<Review>> {
    if !root.join(".refact/reviews/reviews.sqlite").exists() {
        return Ok(None);
    }
    with_store(root, move |store| {
        store.find_review_for_agent(&owner, &agent)
    })
    .await
}
pub(crate) async fn for_worktree(
    root: PathBuf,
    owner: String,
    worktree_id: String,
) -> Result<Option<Review>> {
    if !root.join(".refact/reviews/reviews.sqlite").exists() {
        return Ok(None);
    }
    with_store(root, move |store| {
        let mut matches = store
            .list_for_chat(&owner)?
            .into_iter()
            .filter(|review| {
                review
                    .materials
                    .iter()
                    .any(|material| material.worktree_id.as_deref() == Some(&worktree_id))
            })
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(Error::Invalid("Ambiguous review checkout identity".into()));
        }
        Ok(matches.pop())
    })
    .await
}
pub(crate) async fn mutate_owned(
    root: PathBuf,
    chat_id: String,
    review_id: String,
    mutation: Mutation,
) -> Result<Review> {
    with_store(root, move |s| {
        let r = s.load(&review_id)?;
        if r.owner_chat_id != chat_id {
            return Err(Error::NotFound);
        }
        s.mutate(&review_id, mutation)
    })
    .await
}
pub async fn resolve_review(
    app: &AppState,
    id: &str,
) -> std::result::Result<(PathBuf, Review), String> {
    let roots = crate::files_correction::get_unscoped_project_dirs(app.gcx.clone()).await;
    if roots.len() > 64 {
        return Err("too many project roots".into());
    }
    let mut found = None;
    for root in roots {
        let root = match tokio::fs::canonicalize(root).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        if !root.join(".refact/reviews/reviews.sqlite").exists() {
            continue;
        }
        let id = id.to_owned();
        match with_store(root.clone(), move |s| s.load(&id)).await {
            Ok(review) => {
                let owner_root = trusted_root(app, &review.owner_chat_id).await?;
                if dunce::canonicalize(owner_root).map_err(|e| e.to_string())? != root {
                    continue;
                }
                if found.is_some() {
                    return Err("ambiguous review identity".into());
                }
                found = Some((root, review));
            }
            Err(Error::NotFound) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    found.ok_or_else(|| "review not found".into())
}
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn id(prefix: &str, review: &str, key: &str) -> String {
    format!(
        "{prefix}-{}",
        content_hash(format!("{prefix}\0{review}\0{key}").as_bytes())
    )
}
fn user_actor() -> Actor {
    Actor {
        kind: ActorKind::User,
        id: "user".into(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommentRequest {
    pub expected_revision: u64,
    pub request_key: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub annotation_id: Option<String>,
    #[serde(default)]
    pub anchor: Option<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TourDiscussionTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<RichBlock>,
    pub body: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorRequest {
    pub candidate_id: String,
    pub path: String,
    pub side: Side,
    pub start_line: u32,
    pub end_line: u32,
}
pub async fn file(
    root: PathBuf,
    review: Review,
    candidate_id: String,
    path: String,
) -> Result<FileView> {
    with_store(root, move |s| {
        let c = review
            .candidates
            .iter()
            .find(|c| c.id == candidate_id)
            .ok_or(Error::NotFound)?;
        s.file(c, &path)
    })
    .await
}
pub async fn anchor(root: PathBuf, review: Review, request: AnchorRequest) -> Result<Anchor> {
    with_store(root, move |s| {
        let c = review
            .candidates
            .iter()
            .find(|c| c.id == request.candidate_id)
            .ok_or(Error::NotFound)?;
        s.anchor(
            c,
            &request.path,
            request.side,
            request.start_line,
            request.end_line,
        )
    })
    .await
}
pub(crate) fn reply_contribution_id(review_id: &str, request_key: &str) -> String {
    id("contribution", review_id, request_key)
}

async fn add_comment(
    app: AppState,
    root: PathBuf,
    owner: String,
    review_id: String,
    actor: Actor,
    mut req: CommentRequest,
    mut privacy: Vec<refact_privacy::FileRecord>,
) -> Result<Review> {
    if req.request_key.trim().is_empty()
        || req.request_key.len() > 256
        || req.body.trim().is_empty()
        || req.body.len() > 65536
    {
        return Err(invalid("invalid comment or request key"));
    }
    let source_review = load_owned(root.clone(), owner.clone(), review_id.clone()).await?;
    let mut paths = rich_paths(&root, &source_review, &req.blocks);
    let target = req.target.as_ref().or_else(|| {
        source_review
            .threads
            .iter()
            .find(|t| Some(&t.id) == req.thread_id.as_ref())
            .and_then(|t| t.target.as_ref())
    });
    if let Some(target) = target {
        let (version, block) =
            refact_review_core::rich_blocks::resolve_tour_target(&source_review, target)?;
        if req.thread_id.is_none() {
            if let Some(RichBlock {
                content: RichBlockContent::Code {
                    anchor,
                },
                ..
            }) = block
            {
                if req.anchor.as_ref().is_some_and(|a| a != anchor) {
                    return Err(invalid("code block anchor context mismatch"));
                }
                req.anchor = Some(anchor.clone());
            }
        }
        paths.extend(rich_paths(&root, &source_review, &version.blocks));
        for a in &version.anchors {
            paths.push(root.join(&a.position.path));
            if let Some(c) = source_review
                .candidates
                .iter()
                .find(|c| c.id == a.position.candidate_id)
            {
                paths.push(PathBuf::from(&c.repository).join(&a.position.path));
            }
        }
    }
    privacy.extend(
        crate::privacy::records::declared_file_records(&app.gcx, paths).map_err(Error::Invalid)?,
    );
    if actor.kind == ActorKind::Agent && actor.id != owner {
        let review = load_owned(root.clone(), owner.clone(), review_id.clone()).await?;
        let candidate = &review
            .runs
            .iter()
            .find(|r| r.agent_id == actor.id)
            .ok_or_else(|| invalid("no pinned review run"))?
            .candidate_id;
        let thread = review
            .threads
            .iter()
            .find(|t| Some(&t.id) == req.thread_id.as_ref())
            .ok_or(Error::NotFound)?;
        if thread.anchors.iter().any(|a| {
            &a.position.candidate_id != candidate
                && !a.history.iter().any(|h| &h.candidate_id == candidate)
        }) || thread
            .target
            .as_ref()
            .is_some_and(|t| &t.candidate_id != candidate)
        {
            return Err(invalid("reply thread is outside inspected candidate"));
        }
    }
    let key = req.request_key.clone();
    let rid = review_id.clone();
    let owner_copy = owner.clone();
    let review = with_store(root.clone(), move |s| {
        let r = s.load(&rid)?;
        if r.owner_chat_id != owner_copy {
            return Err(Error::NotFound);
        }
        if let Some(a) = &req.anchor {
            let p = &a.position;
            let candidate = r
                .candidates
                .iter()
                .find(|c| c.id == p.candidate_id)
                .ok_or(Error::NotFound)?;
            if a.status != AnchorStatus::Current
                || a.stalled_reason.is_some()
                || !a.history.is_empty()
                || s.anchor(candidate, &p.path, p.side.clone(), p.start_line, p.end_line)? != *a
            {
                return Err(invalid("anchor does not match viewed candidate"));
            }
        }
        let thread_id = req
            .thread_id
            .clone()
            .unwrap_or_else(|| id("thread", &rid, &key));
        if req.thread_id.is_some() {
            let thread = r
                .threads
                .iter()
                .find(|t| t.id == thread_id)
                .ok_or(Error::NotFound)?;
            if req
                .anchor
                .as_ref()
                .is_some_and(|a| !thread.anchors.contains(a))
                || req
                    .annotation_id
                    .as_ref()
                    .is_some_and(|a| thread.annotation_id.as_ref() != Some(a))
                || req
                    .target
                    .as_ref()
                    .is_some_and(|target| thread.target.as_ref() != Some(target))
            {
                return Err(invalid("thread context mismatch"));
            }
        }
        let run = r
            .runs
            .iter()
            .find(|run| actor.kind == ActorKind::Agent && run.agent_id == actor.id);
        let candidate_id = run
            .map(|run| run.candidate_id.clone())
            .or_else(|| req.anchor.as_ref().map(|a| a.position.candidate_id.clone()))
            .or_else(|| req.target.as_ref().map(|t| t.candidate_id.clone()))
            .or_else(|| {
                r.threads
                    .iter()
                    .find(|t| t.id == thread_id)
                    .and_then(|t| t.target.as_ref())
                    .map(|t| t.candidate_id.clone())
            })
            .or_else(|| {
                r.threads
                    .iter()
                    .find(|t| t.id == thread_id)
                    .and_then(|t| t.anchors.first())
                    .map(|a| a.position.candidate_id.clone())
            })
            .or_else(|| r.current_candidate_id.clone());
        let applicability = if candidate_id.is_some() && candidate_id != r.current_candidate_id {
            Applicability::Historical
        } else {
            Applicability::Current
        };
        let context = Some(ContributionContext {
            candidate_id,
            run_id: run.map(|run| run.id.clone()),
            applicability,
            privacy: privacy.clone(),
        });
        let contribution_id = reply_contribution_id(&rid, &key);
        if let Some(candidate) = context.as_ref().and_then(|c| c.candidate_id.as_deref()) {
            validate_block_candidate(&r, &req.blocks, candidate)?;
        }
        let c = Contribution {
            id: contribution_id.clone(),
            thread_id: thread_id.clone(),
            actor: actor.clone(),
            body: req.body,
            blocks: req.blocks,
            created_at_ms: now_ms(),
            chat_message_id: Some(id("message", &rid, &key)),
            context,
        };
        let operation = if req.thread_id.is_some() {
            Operation::AddContribution {
                contribution: c,
            }
        } else {
            Operation::CreateThread {
                thread: Thread {
                    id: thread_id,
                    anchors: req.anchor.into_iter().collect(),
                    annotation_id: req.annotation_id,
                    target: req.target,
                    resolved: false,
                    contributions: vec![],
                },
                contribution: c,
            }
        };
        s.mutate_with_privacy(
            &rid,
            Mutation {
                expected_revision: req.expected_revision,
                request_key: key,
                actor,
                at_ms: now_ms(),
                operation,
            },
            privacy,
        )
    })
    .await?;
    crate::review::chat_bridge::invalidate(&app, &review).await;
    if let Err(e) = crate::review::chat_bridge::flush(&app, root, &review).await {
        tracing::warn!(error=%e, "review comment remains pending delivery");
    }
    Ok(review)
}
pub async fn comment(
    app: AppState,
    root: PathBuf,
    owner: String,
    review_id: String,
    request: CommentRequest,
) -> Result<Review> {
    add_comment(app, root, owner, review_id, user_actor(), request, vec![]).await
}
pub async fn reply(
    app: AppState,
    root: PathBuf,
    owner: String,
    review_id: String,
    agent_id: String,
    request: CommentRequest,
    privacy: Vec<refact_privacy::FileRecord>,
) -> Result<Review> {
    add_comment(
        app,
        root,
        owner,
        review_id,
        Actor {
            kind: ActorKind::Agent,
            id: agent_id,
        },
        request,
        privacy,
    )
    .await
}

pub(crate) fn validate_block_candidate(
    r: &Review,
    blocks: &[RichBlock],
    candidate: &str,
) -> Result<()> {
    for block in blocks {
        if refact_review_core::rich_blocks::block_anchors(block)
            .iter()
            .any(|a| a.position.candidate_id != candidate)
            || refact_review_core::rich_blocks::block_evidence_ids(block)
                .iter()
                .any(|id| {
                    !r.evidence
                        .iter()
                        .any(|e| &e.id == id && e.candidate_id == candidate)
                })
            || matches!(&block.content, RichBlockContent::Diff { candidate_id, .. } if candidate_id != candidate)
        {
            return Err(invalid("rich block differs from inspected candidate"));
        }
    }
    Ok(())
}

pub(crate) fn rich_paths(
    root: &std::path::Path,
    review: &Review,
    blocks: &[RichBlock],
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut add = |candidate: &str, path: &str| {
        paths.push(root.join(path));
        if let Some(c) = review.candidates.iter().find(|c| c.id == candidate) {
            paths.push(PathBuf::from(&c.repository).join(path));
        }
    };
    for block in blocks {
        for anchor in refact_review_core::rich_blocks::block_anchors(block) {
            add(&anchor.position.candidate_id, &anchor.position.path);
            for p in &anchor.history {
                add(&p.candidate_id, &p.path);
            }
        }
        if let RichBlockContent::Diff {
            candidate_id,
            path,
            ..
        } = &block.content
        {
            add(candidate_id, path);
        }
        for id in refact_review_core::rich_blocks::block_evidence_ids(block) {
            if let Some(e) = review.evidence.iter().find(|e| &e.id == id) {
                if let EvidenceSource::Source {
                    path, ..
                } = &e.source
                {
                    add(&e.candidate_id, path);
                }
            }
        }
    }
    paths
}

fn check_anchors(new: &[Anchor], old: &[Anchor]) -> Result<()> {
    if new.len() < old.len() {
        return Err(invalid("anchor history cannot be removed"));
    }
    for (n, o) in new.iter().zip(old) {
        if n == o {
            continue;
        }
        let mut history = o.history.clone();
        history.push(o.position.clone());
        if n.history != history || n.status != AnchorStatus::Current || n.stalled_reason.is_some() {
            return Err(invalid("anchor history cannot be replaced"));
        }
    }
    Ok(())
}
fn validate_operation(r: &Review, op: &Operation) -> Result<()> {
    match op {
        Operation::Rename {
            ..
        }
        | Operation::SetTargetReference {
            ..
        } => Ok(()),
        Operation::PutTourStep {
            step,
        } => {
            if let Some(old) = r.tour.iter().find(|x| x.id == step.id) {
                if step.history != old.history {
                    return Err(invalid("tour history protected"));
                }
                check_anchors(&step.current.anchors, &old.current.anchors)?;
            } else if !step.history.is_empty()
                || step.current.anchors.iter().any(|a| !a.history.is_empty())
            {
                return Err(invalid("new tour history must be empty"));
            }
            Ok(())
        }
        Operation::PutAnnotation {
            annotation,
        } => {
            if annotation
                .evidence_ids
                .iter()
                .any(|id| !r.evidence.iter().any(|e| &e.id == id))
                || annotation
                    .run_id
                    .as_ref()
                    .is_some_and(|id| !r.runs.iter().any(|run| &run.id == id))
            {
                return Err(invalid("unknown annotation provenance"));
            }
            if let Some(old) = r.annotations.iter().find(|a| a.id == annotation.id) {
                check_anchors(&annotation.anchors, &old.anchors)?;
            } else if annotation.anchors.iter().any(|a| !a.history.is_empty()) {
                return Err(invalid("anchor history protected"));
            }
            Ok(())
        }
        Operation::PutThread {
            thread,
        } => {
            let old = r
                .threads
                .iter()
                .find(|t| t.id == thread.id)
                .ok_or(Error::NotFound)?;
            if thread.contributions != old.contributions
                || thread.annotation_id != old.annotation_id
                || thread.target != old.target
            {
                return Err(invalid("thread history protected: contributions are append-only; answer a discussion with review_reply(thread_id, body). put_thread only re-anchors or resolves existing threads"));
            }
            check_anchors(&thread.anchors, &old.anchors)
        }
        _ => Err(invalid("operation requires a trusted runtime")),
    }
}
pub async fn user_mutate(
    root: PathBuf,
    owner: String,
    review_id: String,
    mut mutation: Mutation,
) -> Result<Review> {
    safe_mutate(root, owner, review_id, user_actor(), &mut mutation).await
}
pub async fn model_mutate(
    root: PathBuf,
    owner: String,
    review_id: String,
    agent_id: String,
    mutation: Mutation,
) -> Result<Review> {
    model_mutate_with_privacy(root, owner, review_id, agent_id, mutation, vec![]).await
}

pub async fn model_mutate_with_privacy(
    root: PathBuf,
    owner: String,
    review_id: String,
    agent_id: String,
    mutation: Mutation,
    privacy: Vec<refact_privacy::FileRecord>,
) -> Result<Review> {
    let mut mutation = mutation;
    let actor = Actor {
        kind: ActorKind::Agent,
        id: agent_id.clone(),
    };
    mutation.actor = actor;
    mutation.at_ms = now_ms();
    with_store(root, move |s| {
        let r = s.load(&review_id)?;
        if r.owner_chat_id != owner {
            return Err(Error::NotFound);
        }
        let candidate = if agent_id == owner {
            r.current_candidate_id
                .as_deref()
                .ok_or_else(|| invalid("no active candidate"))?
        } else {
            &r.runs
                .iter()
                .find(|run| run.agent_id == agent_id)
                .ok_or_else(|| invalid("no pinned review run"))?
                .candidate_id
        };
        let historical = agent_id != owner && r.current_candidate_id.as_deref() != Some(candidate);
        if let Operation::PutTourStep {
            step,
        } = &mut mutation.operation
        {
            if agent_id != owner
                || s.get_for_chat(&owner)?.as_ref().map(|active| &active.id) != Some(&r.id)
            {
                return Err(invalid(
                    "only the owning foreground active tour author may update the tour",
                ));
            }
            if step
                .current
                .candidate_id
                .as_deref()
                .is_some_and(|id| id != candidate)
            {
                return Err(invalid("tour candidate differs from inspected candidate"));
            }
            step.current.candidate_id = Some(candidate.to_owned());
            // A stale request can only be an exact replay or a CAS failure in the store.
            // Let its fingerprint check run before validating against newer object history.
            if mutation.expected_revision != r.revision {
                return s.mutate_with_privacy(&review_id, mutation, privacy);
            }
            validate_block_candidate(&r, &step.current.blocks, candidate)?;
        }
        if historical {
            match &mut mutation.operation {
                Operation::PutAnnotation {
                    annotation,
                } if !r.annotations.iter().any(|a| a.id == annotation.id) => {
                    annotation.applicability = Applicability::Historical;
                    annotation.run_id = r
                        .runs
                        .iter()
                        .find(|run| run.agent_id == agent_id)
                        .map(|run| run.id.clone());
                }
                Operation::AddEvidence {
                    evidence,
                } => evidence.applicability = Applicability::Historical,
                _ => {
                    return Err(invalid(
                        "historical run cannot overwrite current review objects",
                    ))
                }
            }
        }
        if agent_id != owner {
            if let Operation::PutAnnotation {
                annotation,
            } = &mut mutation.operation
            {
                let own_run = r
                    .runs
                    .iter()
                    .find(|run| run.agent_id == agent_id)
                    .ok_or_else(|| invalid("no pinned review run"))?;
                if annotation
                    .run_id
                    .as_ref()
                    .is_some_and(|id| id != &own_run.id)
                {
                    return Err(invalid("annotation run identity mismatch"));
                }
                annotation.run_id = Some(own_run.id.clone());
            }
        }
        if let Operation::PutAnnotation {
            annotation,
        } = &mutation.operation
        {
            if annotation.evidence_ids.iter().any(|id| {
                !r.evidence
                    .iter()
                    .any(|e| &e.id == id && e.candidate_id == candidate)
            }) || annotation.run_id.as_ref().is_some_and(|id| {
                !r.runs
                    .iter()
                    .any(|run| &run.id == id && run.agent_id == agent_id)
            }) {
                return Err(invalid(
                    "annotation provenance does not match runtime candidate",
                ));
            }
        }
        if let Operation::AddEvidence {
            evidence,
        } = &mut mutation.operation
        {
            if evidence.candidate_id != candidate
                || evidence.material_id.is_some()
                || matches!(evidence.source, EvidenceSource::Executed { .. })
            {
                return Err(invalid("evidence provenance must be runtime recorded"));
            }
            if let EvidenceSource::Source {
                path,
                anchor,
            } = &evidence.source
            {
                let c = r
                    .candidates
                    .iter()
                    .find(|c| c.id == candidate)
                    .ok_or(Error::NotFound)?;
                if !c.files.iter().any(|f| &f.path == path) {
                    return Err(Error::NotFound);
                }
                if let Some(a) = anchor {
                    if a.position.candidate_id != candidate || &a.position.path != path {
                        return Err(invalid("evidence anchor mismatch"));
                    }
                    let p = &a.position;
                    if a.status != AnchorStatus::Current
                        || a.stalled_reason.is_some()
                        || !a.history.is_empty()
                        || s.anchor(c, &p.path, p.side.clone(), p.start_line, p.end_line)? != *a
                    {
                        return Err(invalid("evidence anchor does not match snapshot"));
                    }
                }
            }
            evidence.run_id = r
                .runs
                .iter()
                .find(|run| run.agent_id == agent_id)
                .map(|run| run.id.clone());
            evidence.created_at_ms = now_ms();
            return s.mutate_with_privacy(&review_id, mutation, privacy);
        }
        let anchors: &[Anchor] = match &mutation.operation {
            Operation::PutTourStep {
                step,
            } => &step.current.anchors,
            Operation::PutAnnotation {
                annotation,
            } => &annotation.anchors,
            Operation::PutThread {
                thread,
            } => &thread.anchors,
            _ if agent_id == owner => &[],
            _ => return Err(invalid("review child cannot change review control state")),
        };
        if anchors.iter().any(|a| a.position.candidate_id != candidate) {
            return Err(invalid("operation differs from inspected candidate"));
        }
        validate_operation(&r, &mutation.operation)?;
        s.mutate_with_privacy(&review_id, mutation, privacy)
    })
    .await
}
async fn safe_mutate(
    root: PathBuf,
    owner: String,
    review_id: String,
    actor: Actor,
    mutation: &mut Mutation,
) -> Result<Review> {
    mutation.actor = actor;
    mutation.at_ms = now_ms();
    let mutation = mutation.clone();
    with_store(root, move |s| {
        let r = s.load(&review_id)?;
        if r.owner_chat_id != owner {
            return Err(Error::NotFound);
        }
        validate_operation(&r, &mutation.operation)?;
        s.mutate(&review_id, mutation)
    })
    .await
}

#[cfg(test)]
mod lookup_side_effect_tests {
    #[tokio::test]
    async fn absent_review_lookups_do_not_create_project_state() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        assert!(
            super::for_agent(root.clone(), "owner".into(), "ordinary-child".into())
                .await
                .unwrap()
                .is_none()
        );
        assert!(!root.join(".refact").exists());
        assert!(
            super::for_worktree(root.clone(), "owner".into(), "ordinary-checkout".into())
                .await
                .unwrap()
                .is_none()
        );
        assert!(!root.join(".refact").exists());
    }
}
