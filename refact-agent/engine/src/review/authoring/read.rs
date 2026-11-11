//! Compact projections for the model. Nothing here is a mutation payload.

use refact_review_core::*;
use serde_json::{json, Value};

use super::{candidate, changed_files, describe, parse_lines, resolve_path, side_name};
use super::{member_label, Outcome, Target};

const MAX_READ_LINES: u32 = 500;
const MAX_READ_CHARS: usize = 64_000;

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(limit).collect();
    clipped.push('…');
    clipped
}

fn place(anchor: &Anchor) -> String {
    let position = &anchor.position;
    let lines = if position.start_line == position.end_line {
        position.start_line.to_string()
    } else {
        format!("{}-{}", position.start_line, position.end_line)
    };
    let side = if position.side == Side::Base {
        " (base)"
    } else {
        ""
    };
    format!("{}:{lines}{side}", position.path)
}

pub fn finding_line(item: &Annotation) -> Value {
    json!({
        "id": item.id,
        "kind": item.kind,
        "severity": item.severity,
        "title": clip(&item.title, 200),
        "at": item.anchors.first().map(place),
        "assessment": item.assessment,
        "status": item.disposition,
    })
}

fn author(review: &Review, contribution: &Contribution) -> String {
    match contribution.actor.kind {
        ActorKind::User => "user".into(),
        ActorKind::Agent if contribution.actor.id == review.owner_chat_id => "you".into(),
        ActorKind::Agent => "worker".into(),
        ActorKind::System => "system".into(),
        ActorKind::External => contribution.actor.id.clone(),
    }
}

fn thread_line(review: &Review, thread: &Thread) -> Value {
    let last = thread.contributions.last();
    json!({
        "id": thread.id,
        "at": thread.anchors.first().map(place),
        "page": thread.target.as_ref().map(|target| target.step_id.clone()),
        "finding": thread.annotation_id,
        "resolved": thread.resolved,
        "messages": thread.contributions.len(),
        "last_from": last.map(|contribution| author(review, contribution)),
        "last": last.map(|contribution| clip(&contribution.body, 400)),
        "awaiting_your_reply": !thread.resolved
            && last.is_some_and(|contribution| contribution.actor.kind == ActorKind::User),
    })
}

fn members(target: &Target, per_member: &[(String, usize, usize, usize)]) -> Option<Value> {
    let stack = target.stack.as_ref()?;
    Some(json!({
        "title": stack.title,
        "members": stack.members.iter().map(|member| {
            let counts = per_member.iter().find(|(id, ..)| Some(id) == member.review_id.as_ref());
            json!({
                "member": member_label(member),
                "pr": member.pr_identity.as_ref().map(|pr| pr.number),
                "open_here": member.review_id.as_deref() == Some(target.review.id.as_str()),
                "changed_files": counts.map(|(_, files, ..)| *files),
                "pages": counts.map(|(_, _, pages, _)| *pages),
                "findings": counts.map(|(.., findings)| *findings),
            })
        }).collect::<Vec<_>>(),
    }))
}

async fn member_counts(target: &Target) -> Outcome<Vec<(String, usize, usize, usize)>> {
    let Some(stack) = target.stack.clone() else {
        return Ok(vec![]);
    };
    super::service::with_store(target.root.clone(), move |store| {
        let mut counts = Vec::new();
        for member in &stack.members {
            if let Some(review) = store.stack_review_for_member(&stack.id, &member.id)? {
                let files = candidate(&review).map_or(0, |c| changed_files(c).len());
                counts.push((
                    review.id,
                    files,
                    review.tour.len(),
                    review.annotations.len(),
                ));
            }
        }
        Ok(counts)
    })
    .await
    .map_err(describe)
}

/// Everything the owning chat needs to decide its next write, in a few kilobytes.
pub async fn overview(target: &Target) -> Outcome<Value> {
    let review = &target.review;
    let current = candidate(review)?;
    let changed = changed_files(current);
    let counts = member_counts(target).await?;
    Ok(json!({
        "review": {
            "title": review.title,
            "target": review.target_reference,
            "base": &current.base_oid[..current.base_oid.len().min(12)],
            "head": &current.head_oid[..current.head_oid.len().min(12)],
        },
        "stack": members(target, &counts),
        "changed_files": changed.iter().take(200).map(|(path, status)| json!({"path":path,"status":status})).collect::<Vec<_>>(),
        "changed_files_total": changed.len(),
        "tour": review.tour.iter().map(|step| json!({
            "id": step.id,
            "title": clip(&step.current.title, 200),
            "version": step.current.version,
            "inserts": step.current.blocks.len(),
            "summary": clip(&step.current.body, 300),
        })).collect::<Vec<_>>(),
        "findings": review.annotations.iter().rev().take(60).map(finding_line).collect::<Vec<_>>(),
        "findings_total": review.annotations.len(),
        "threads": review.threads.iter().rev().take(30).map(|thread| thread_line(review, thread)).collect::<Vec<_>>(),
        "threads_total": review.threads.len(),
        "sources": review.evidence.iter().rev().take(30).map(|item| json!({
            "id": item.id,
            "summary": clip(&item.summary, 200),
            "executed": matches!(item.source, EvidenceSource::Executed { .. }),
        })).collect::<Vec<_>>(),
        "sources_total": review.evidence.len(),
        "workers": review.runs.iter().rev().take(20).map(|run| json!({
            "agent_id": run.agent_id,
            "purpose": clip(&run.purpose, 120),
            "status": run.status,
        })).collect::<Vec<_>>(),
        "waiting_for_pr": review.remote_links.iter().filter(|link| link.state == RemoteLinkState::Requested).count(),
    }))
}

/// One complete page, as stored: the HTML with its slots and what each slot shows.
pub fn page(review: &Review, id: &str) -> Outcome<Value> {
    let step = review
        .tour
        .iter()
        .find(|step| step.id == id)
        .ok_or_else(|| {
            format!(
                "No page `{id}`. Pages: {}",
                review
                    .tour
                    .iter()
                    .map(|step| step.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    Ok(json!({
        "id": step.id,
        "title": step.current.title,
        "version": step.current.version,
        "summary": step.current.body,
        "html": step.current.html,
        "inserts": step.current.blocks.iter().map(|block| json!({
            "id": block.id,
            "shows": match &block.content {
                RichBlockContent::Code { anchor } => place(anchor),
                RichBlockContent::Diff { path, anchors, .. } => anchors.first().map_or(format!("diff of {path}"), |anchor| format!("diff of {}", place(anchor))),
                RichBlockContent::Sources { evidence_ids, .. } | RichBlockContent::Evidence { evidence_ids } => evidence_ids.join(", "),
                RichBlockContent::Markdown { .. } => "text".into(),
                RichBlockContent::Html { .. } => "embedded html".into(),
            },
        })).collect::<Vec<_>>(),
        "editing": "Send `edits` to change parts of this HTML, or a complete new `html`. Keep each <refact-slot> you want to keep.",
    }))
}

pub fn finding(review: &Review, id: &str) -> Outcome<Value> {
    let item = review
        .annotations
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("No finding `{id}`"))?;
    let mut value = finding_line(item);
    value["body"] = json!(clip(&item.body, 8000));
    value["confidence"] = json!(item.confidence);
    Ok(value)
}

pub fn thread(review: &Review, id: &str) -> Outcome<Value> {
    let thread = review
        .threads
        .iter()
        .find(|thread| thread.id == id)
        .ok_or_else(|| format!("No thread `{id}`"))?;
    let mut value = thread_line(review, thread);
    value["conversation"] = json!(thread
        .contributions
        .iter()
        .rev()
        .take(40)
        .rev()
        .map(|contribution| json!({
            "from": author(review, contribution),
            "text": clip(&contribution.body, 4000),
        }))
        .collect::<Vec<_>>());
    Ok(value)
}

/// A numbered range of one snapshot file, or its unified diff. Returns the resolved path too.
pub async fn file(
    target: &Target,
    path: &str,
    lines: Option<&str>,
    side: Side,
    diff: bool,
) -> Outcome<(Value, String)> {
    let current = candidate(&target.review)?.clone();
    let path = resolve_path(&target.review, &current, path)?;
    let range = lines.map(parse_lines).transpose()?;
    let wanted = path.clone();
    let view = super::service::with_store(target.root.clone(), move |store| {
        store.file(&current, &wanted)
    })
    .await
    .map_err(describe)?;
    if diff {
        return Ok((
            json!({"path":path,"diff":clip(&view.diff, MAX_READ_CHARS)}),
            path,
        ));
    }
    let bytes = match side {
        Side::Base => view.base,
        Side::Head => view.head,
    }
    .ok_or_else(|| {
        format!(
            "`{path}` does not exist on the {} side of this comparison",
            side_name(&side)
        )
    })?;
    let text = String::from_utf8(bytes).map_err(|_| format!("`{path}` is binary"))?;
    let total = text.lines().count() as u32;
    let (start, end) = range.unwrap_or((1, 200));
    if start > total.max(1) {
        return Err(format!(
            "`{path}` has {total} lines; line {start} does not exist"
        ));
    }
    let end = end.min(total).min(start.saturating_add(MAX_READ_LINES - 1));
    let mut used = 0usize;
    let shown: Vec<String> = text
        .lines()
        .enumerate()
        .skip(start as usize - 1)
        .take((end + 1).saturating_sub(start) as usize)
        .take_while(|(_, line)| {
            used += line.chars().count();
            used <= MAX_READ_CHARS
        })
        .map(|(index, line)| format!("{:>5} | {line}", index + 1))
        .collect();
    Ok((
        json!({
            "path": path,
            "side": side_name(&side),
            "total_lines": total,
            "shown": format!("{start}-{}", start as usize + shown.len().max(1) - 1),
            "text": shown.join("\n"),
        }),
        path,
    ))
}

/// Paths whose content a projection of this review may repeat.
pub fn quoted_paths(review: &Review) -> Vec<String> {
    let mut paths: Vec<String> = review
        .tour
        .iter()
        .flat_map(|step| step.current.blocks.iter())
        .flat_map(|block| {
            let mut found: Vec<String> = rich_blocks::block_anchors(block)
                .iter()
                .map(|anchor| anchor.position.path.clone())
                .collect();
            if let RichBlockContent::Diff {
                path, ..
            } = &block.content
            {
                found.push(path.clone());
            }
            found
        })
        .chain(
            review
                .annotations
                .iter()
                .flat_map(|item| item.anchors.iter())
                .chain(
                    review
                        .threads
                        .iter()
                        .flat_map(|thread| thread.anchors.iter()),
                )
                .map(|anchor| anchor.position.path.clone()),
        )
        .collect();
    paths.sort();
    paths.dedup();
    paths
}
