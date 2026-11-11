//! Model-facing authoring over the durable review store. The server owns revisions,
//! idempotency keys, page versions and anchors; the model supplies content, paths and lines.

pub mod comments;
pub mod inserts;
pub mod open;
pub mod read;
pub mod write;

use std::{path::PathBuf, sync::Arc};

use refact_review_core::*;
use tokio::sync::Mutex;

use super::service;
use crate::{app_state::AppState, at_commands::at_commands::AtCommandsContext};

pub type Ccx = Arc<Mutex<AtCommandsContext>>;
pub type Outcome<T> = std::result::Result<T, String>;
type Build = Arc<dyn Fn(&ReviewStore, &Review) -> Outcome<Operation> + Send + Sync>;

pub struct Target {
    pub app: AppState,
    pub chat: String,
    pub root: PathBuf,
    pub review: Review,
    pub stack: Option<StackManifest>,
}

pub fn describe(error: Error) -> String {
    match error {
        Error::NotFound => "Not found in this review".into(),
        Error::Conflict {
            ..
        } => "The review changed while this call was being applied; repeat the call".into(),
        Error::IdempotencyConflict => "This call was already applied with different content".into(),
        Error::Invalid(message) | Error::Limit(message) | Error::Integrity(message) => message,
        Error::NonText => "The file is binary".into(),
        other => other.to_string(),
    }
}

pub async fn owner(ccx: &Ccx) -> Outcome<(AppState, String)> {
    let context = ccx.lock().await;
    if context.background_agent_id.is_some()
        || context.subchat_depth != 0
        || context.task_meta.is_some()
    {
        return Err(
            "Review tools run in the chat that owns the review; report your results to it".into(),
        );
    }
    Ok((context.app.clone(), context.chat_id.clone()))
}

pub fn member_label(member: &StackMember) -> String {
    match &member.pr_identity {
        Some(pr) => format!("#{} {}", pr.number, member.branch_ref),
        None => member.branch_ref.clone(),
    }
}

pub fn find_member<'a>(stack: &'a StackManifest, selector: &str) -> Outcome<&'a StackMember> {
    let wanted = selector.trim().trim_start_matches('#');
    let number = wanted.parse::<u64>().ok();
    let mut matches: Vec<&StackMember> = stack
        .members
        .iter()
        .filter(|member| {
            member.id == wanted
                || member.review_id.as_deref() == Some(wanted)
                || member.branch_ref == wanted
                || number
                    .is_some_and(|n| member.pr_identity.as_ref().is_some_and(|pr| pr.number == n))
        })
        .collect();
    if matches.is_empty() && !wanted.is_empty() {
        matches = stack
            .members
            .iter()
            .filter(|member| member.branch_ref.ends_with(wanted))
            .collect();
    }
    match matches.as_slice() {
        [one] => Ok(one),
        _ => Err(format!(
            "`member` must name exactly one stack member by PR number or branch. Members: {}",
            stack
                .members
                .iter()
                .map(member_label)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// `Ok(None)` is the ordinary state before a review has been opened in this chat.
pub async fn target(ccx: &Ccx, member: Option<&str>) -> Outcome<Option<Target>> {
    let (app, chat) = owner(ccx).await?;
    let root = service::trusted_root(&app, &chat).await?;
    let stack = super::stacks::get(&app, &chat).await?;
    let review = match member
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
    {
        Some(selector) => {
            let stack = stack.as_ref().ok_or(
                "`member` selects a PR of a stack review; this chat reviews a single comparison",
            )?;
            let found = find_member(stack, selector)?;
            let id = found
                .review_id
                .clone()
                .ok_or_else(|| format!("Stack member {} was not captured", member_label(found)))?;
            Some(
                service::load_owned(root.clone(), chat.clone(), id)
                    .await
                    .map_err(describe)?,
            )
        }
        None => service::get_for_chat(root.clone(), chat.clone())
            .await
            .map_err(describe)?,
    };
    let Some(review) = review else {
        return Ok(None);
    };
    super::context::verify_binding_for_chat(&app, &chat).await?;
    Ok(Some(Target {
        app,
        chat,
        root,
        review,
        stack,
    }))
}

pub const NOT_OPEN: &str = "No review is open in this chat yet. Call review_open first.";

pub async fn require(ccx: &Ccx, member: Option<&str>) -> Outcome<Target> {
    target(ccx, member)
        .await?
        .ok_or_else(|| NOT_OPEN.to_string())
}

/// The review of this chat that holds `thread`: the open one, or another stack member.
pub async fn thread_home(ccx: &Ccx, thread: &str) -> Outcome<Target> {
    let mut target = require(ccx, None).await?;
    if target.review.threads.iter().any(|item| item.id == thread) {
        return Ok(target);
    }
    let Some(stack) = target.stack.clone() else {
        return Ok(target);
    };
    let (wanted, owner) = (thread.to_string(), target.chat.clone());
    let found = service::with_store(target.root.clone(), move |store| {
        for member in &stack.members {
            if let Some(review) = store.stack_review_for_member(&stack.id, &member.id)? {
                if review.owner_chat_id == owner
                    && review.threads.iter().any(|item| item.id == wanted)
                {
                    return Ok(Some(review));
                }
            }
        }
        Ok(None)
    })
    .await
    .map_err(describe)?;
    if let Some(review) = found {
        target.review = review;
    }
    Ok(target)
}

pub fn candidate(review: &Review) -> Outcome<&Candidate> {
    let id = review
        .current_candidate_id
        .as_deref()
        .ok_or("This review has no captured comparison yet; call review_open")?;
    review
        .candidates
        .iter()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| "The active comparison is unavailable".to_string())
}

pub fn changed_files(candidate: &Candidate) -> Vec<(&str, &'static str)> {
    candidate
        .files
        .iter()
        .filter_map(|file| {
            let status = match (&file.base, &file.head) {
                (None, Some(_)) => "added",
                (Some(_), None) => "deleted",
                (Some(base), Some(head)) if base != head || file.base_mode != file.head_mode => {
                    "modified"
                }
                _ => return None,
            };
            Some((file.path.as_str(), status))
        })
        .collect()
}

/// Accepts snapshot-relative paths, absolute checkout or repository paths and unique suffixes.
pub fn resolve_path(review: &Review, candidate: &Candidate, given: &str) -> Outcome<String> {
    let mut wanted = given.trim().replace('\\', "/");
    let roots = std::iter::once(candidate.repository.as_str()).chain(
        review
            .materials
            .iter()
            .filter_map(|material| material.worktree_root.as_deref()),
    );
    for root in roots {
        if let Some(rest) = wanted.strip_prefix(root) {
            wanted = rest.to_string();
            break;
        }
    }
    let wanted = wanted
        .trim_start_matches('/')
        .trim_start_matches("./")
        .to_string();
    if candidate.files.iter().any(|file| file.path == wanted) {
        return Ok(wanted);
    }
    let suffix = format!("/{wanted}");
    let matches: Vec<&str> = candidate
        .files
        .iter()
        .map(|file| file.path.as_str())
        .filter(|path| path.ends_with(&suffix))
        .collect();
    if let [one] = matches.as_slice() {
        return Ok((*one).to_string());
    }
    let name = wanted.rsplit('/').next().unwrap_or(&wanted);
    let close: Vec<&str> = candidate
        .files
        .iter()
        .map(|file| file.path.as_str())
        .filter(|path| path.rsplit('/').next() == Some(name))
        .take(5)
        .collect();
    Err(if close.is_empty() {
        format!("`{given}` is not a file in this review; paths are relative to the repository root")
    } else {
        format!(
            "`{given}` is not a file in this review. Did you mean: {}",
            close.join(", ")
        )
    })
}

/// `"12"`, `"12-40"` and `"L12-L40"` all name an inclusive 1-based range.
pub fn parse_lines(spec: &str) -> Outcome<(u32, u32)> {
    let number = |part: &str| {
        part.trim()
            .trim_start_matches(['L', 'l'])
            .parse::<u32>()
            .ok()
    };
    let cleaned = spec.trim();
    let parsed = match cleaned.split_once(['-', ':', '–']) {
        Some((start, end)) => number(start).zip(number(end)),
        None => number(cleaned).map(|line| (line, line)),
    };
    match parsed {
        Some((start, end)) if start >= 1 && end >= start => Ok((start, end)),
        _ => Err("lines must look like `12` or `12-40` (1-based, inclusive)".into()),
    }
}

pub fn side_name(side: &Side) -> &'static str {
    match side {
        Side::Base => "base",
        Side::Head => "head",
    }
}

/// Ranges running past the end of the file are clamped; a range starting past it is an error.
pub fn anchor_in(
    store: &ReviewStore,
    candidate: &Candidate,
    path: &str,
    side: Side,
    start: u32,
    end: u32,
) -> Outcome<Anchor> {
    let file = store.file(candidate, path).map_err(describe)?;
    let bytes = match side {
        Side::Base => file.base,
        Side::Head => file.head,
    }
    .ok_or_else(|| {
        format!(
            "`{path}` does not exist on the {} side of this comparison",
            side_name(&side)
        )
    })?;
    let text = String::from_utf8(bytes).map_err(|_| format!("`{path}` is binary"))?;
    let total = text.lines().count() as u32;
    if total == 0 || start > total {
        return Err(format!(
            "`{path}` has {total} lines on the {} side; line {start} does not exist",
            side_name(&side)
        ));
    }
    store
        .anchor(candidate, path, side, start, end.min(total))
        .map_err(describe)
}

/// Provenance carried by the chat so far plus the files this write quotes.
pub async fn provenance(
    ccx: &Ccx,
    target: &Target,
    paths: &[String],
) -> Outcome<Vec<refact_privacy::FileRecord>> {
    let mut sources = ccx.lock().await.messages.clone();
    let live = target
        .app
        .chat
        .sessions
        .read()
        .await
        .get(&target.chat)
        .cloned();
    if let Some(live) = live {
        sources.extend(live.lock().await.messages.iter().cloned());
    }
    let mut records = crate::privacy::records::records_to_carry(&sources)
        .map_err(|error| format!("Review source provenance unavailable: {error}"))?;
    records.extend(crate::privacy::records::declared_file_records(
        &target.app.gcx,
        origin_paths(target, paths),
    )?);
    Ok(records)
}

pub fn origin_paths(target: &Target, paths: &[String]) -> Vec<PathBuf> {
    let repository = candidate(&target.review)
        .ok()
        .map(|candidate| PathBuf::from(&candidate.repository));
    paths
        .iter()
        .flat_map(|path| {
            std::iter::once(target.root.join(path))
                .chain(repository.as_ref().map(|repository| repository.join(path)))
        })
        .collect()
}

/// Applies one owner-authored operation. A concurrent writer is retried, never reported.
pub(super) async fn mutate(
    target: &Target,
    key: String,
    privacy: Vec<refact_privacy::FileRecord>,
    build: Build,
) -> Outcome<Review> {
    for _ in 0..8 {
        let id = target.review.id.clone();
        let owner = target.chat.clone();
        let builder = build.clone();
        let (revision, operation) = service::with_store(target.root.clone(), move |store| {
            let review = store.load(&id)?;
            if review.owner_chat_id != owner {
                return Err(Error::NotFound);
            }
            Ok((review.revision, builder(store, &review)))
        })
        .await
        .map_err(describe)?;
        let mutation = Mutation {
            expected_revision: revision,
            request_key: key.clone(),
            actor: Actor {
                kind: ActorKind::Agent,
                id: target.chat.clone(),
            },
            at_ms: service::now_ms(),
            operation: operation?,
        };
        match service::model_mutate_with_privacy(
            target.root.clone(),
            target.chat.clone(),
            target.review.id.clone(),
            target.chat.clone(),
            mutation,
            privacy.clone(),
        )
        .await
        {
            Ok(review) => {
                super::chat_bridge::invalidate(&target.app, &review).await;
                return Ok(review);
            }
            Err(Error::Conflict {
                ..
            }) => continue,
            // The key is unique to one tool call, so a stored request means it already applied.
            Err(Error::IdempotencyConflict) => {
                return service::load_owned(
                    target.root.clone(),
                    target.chat.clone(),
                    target.review.id.clone(),
                )
                .await
                .map_err(describe)
            }
            Err(error) => return Err(describe(error)),
        }
    }
    Err("The review is being changed by several writers at once; repeat this call".into())
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn short_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}
