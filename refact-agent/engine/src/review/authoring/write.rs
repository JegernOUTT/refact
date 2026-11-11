//! Owner-authored writes: tour pages, findings, sources and thread replies.

use std::{collections::HashSet, sync::Arc};

use refact_review_core::*;
use serde_json::{json, Value};

use super::{
    anchor_in, candidate, inserts, mutate, parse_lines, provenance, resolve_path, short_id,
    valid_id, Ccx, Outcome, Target,
};

const MAX_PAGE_BYTES: usize = 1024 * 1024;

pub struct PageInput {
    pub id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub html: Option<String>,
    pub edits: Vec<(String, String)>,
}

fn tour_index(review: &Review) -> Value {
    json!(review
        .tour
        .iter()
        .map(|step| json!({"id":step.id,"title":step.current.title,"version":step.current.version}))
        .collect::<Vec<_>>())
}

/// Writes or replaces one tour page. Versions, history and anchors are derived here.
pub async fn tour_page(
    ccx: &Ccx,
    target: &Target,
    input: PageInput,
    call_id: &str,
) -> Outcome<(Review, Value, Vec<String>)> {
    if !valid_id(&input.id) {
        return Err("`id` may use letters, digits, `-`, `_`, `.` (at most 64)".into());
    }
    let existing = target.review.tour.iter().find(|step| step.id == input.id);
    let title = input
        .title
        .filter(|title| !title.trim().is_empty())
        .or_else(|| existing.map(|step| step.current.title.clone()))
        .ok_or("`title` is required for a new page")?;
    let summary = input
        .summary
        .filter(|summary| !summary.trim().is_empty())
        .or_else(|| existing.map(|step| step.current.body.clone()))
        .ok_or("`summary` is required for a new page")?;
    let mut html = input
        .html
        .filter(|html| !html.trim().is_empty())
        .or_else(|| existing.and_then(|step| step.current.html.clone()))
        .ok_or("`html` is required: this page has no HTML yet")?;
    for (index, (old, new)) in input.edits.iter().enumerate() {
        match html.matches(old.as_str()).count() {
            1 if !old.is_empty() => html = html.replacen(old.as_str(), new, 1),
            0 => {
                return Err(format!(
                    "`edits[{index}].old` does not occur in the current page; read the page and copy the text exactly"
                ))
            }
            found => {
                return Err(format!(
                    "`edits[{index}].old` occurs {found} times in the current page; include more surrounding text so it matches once"
                ))
            }
        }
    }
    let carried = inserts::slot_ids(&html);
    let taken: HashSet<String> = carried.iter().cloned().collect();
    if taken.len() != carried.len() {
        return Err("The page shows the same insert twice; each insert appears once".into());
    }
    let (html, fresh) = inserts::rewrite(&html, &taken)?;
    if carried.len() + fresh.len() > inserts::MAX_PAGE_INSERTS {
        return Err(format!(
            "A page holds at most {} inserts; this one has {}. Split it into two pages",
            inserts::MAX_PAGE_INSERTS,
            carried.len() + fresh.len()
        ));
    }
    if html.len() > MAX_PAGE_BYTES {
        return Err(format!(
            "The page is {} bytes; the limit is {MAX_PAGE_BYTES}. Split it into two pages",
            html.len()
        ));
    }
    let quoted = inserts::quoted_paths(&target.review, candidate(&target.review)?, &fresh);
    let privacy = provenance(ccx, target, &quoted).await?;
    let (id, page_html, described) = (input.id.clone(), html.clone(), fresh.clone());
    let review = mutate(
        target,
        format!("tool:{call_id}"),
        privacy,
        Arc::new(move |store: &ReviewStore, review: &Review| {
            let current = candidate(review)?;
            let old = review.tour.iter().find(|step| step.id == id);
            let mut blocks = Vec::new();
            for slot in &carried {
                let block = old
                    .and_then(|step| step.current.blocks.iter().find(|block| &block.id == slot))
                    .ok_or_else(|| {
                        format!(
                            "`<refact-slot data-block=\"{slot}\">` matches no insert of this page; tag code with <refact-code> or <refact-diff>"
                        )
                    })?;
                let stale = rich_blocks::block_anchors(block)
                    .iter()
                    .any(|anchor| anchor.position.candidate_id != current.id)
                    || matches!(&block.content, RichBlockContent::Diff { candidate_id, .. } if candidate_id != &current.id);
                if stale {
                    return Err(format!(
                        "Page `{id}` was written for an earlier version of the code; send its complete `html` again"
                    ));
                }
                blocks.push(block.clone());
            }
            for insert in &fresh {
                blocks.push(inserts::block(store, review, current, insert)?);
            }
            let anchors = old
                .map(|step| step.current.anchors.clone())
                .unwrap_or_default();
            if anchors
                .iter()
                .any(|anchor| anchor.position.candidate_id != current.id)
            {
                return Err(format!(
                    "Page `{id}` predates the current version of the code; write it under a new id"
                ));
            }
            Ok(Operation::PutTourStep {
                step: TourStep {
                    id: id.clone(),
                    current: TourVersion {
                        html: Some(page_html.clone()),
                        blocks,
                        candidate_id: Some(current.id.clone()),
                        version: old.map_or(1, |step| step.current.version + 1),
                        title: title.clone(),
                        body: summary.clone(),
                        anchors,
                    },
                    history: old.map(|step| step.history.clone()).unwrap_or_default(),
                },
            })
        }),
    )
    .await?;
    let step = review
        .tour
        .iter()
        .find(|step| step.id == input.id)
        .ok_or("The saved page is unavailable")?;
    let receipt = json!({
        "page": {"id":step.id,"title":step.current.title,"version":step.current.version,"bytes":html.len()},
        "inserts": described.iter().map(|insert| json!({
            "id":insert.id,
            "path":insert.path,
            "lines":insert.lines.map(|(start, end)| format!("{start}-{end}")),
        })).collect::<Vec<_>>(),
        "tour": tour_index(&review),
    });
    Ok((review, receipt, quoted))
}

#[derive(Default)]
pub struct FindingInput {
    pub id: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub path: Option<String>,
    pub lines: Option<String>,
    pub side: Option<Side>,
    pub kind: Option<AnnotationKind>,
    pub severity: Option<Severity>,
    pub confidence: Option<Confidence>,
    pub assessment: Option<Assessment>,
    pub disposition: Option<Disposition>,
}

/// Adds a finding, or updates the one named by `id`; omitted fields keep their value.
pub async fn finding(
    ccx: &Ccx,
    target: &Target,
    input: FindingInput,
    call_id: &str,
) -> Outcome<(Review, Value, Vec<String>)> {
    let existing = input
        .id
        .as_ref()
        .and_then(|id| target.review.annotations.iter().find(|item| &item.id == id));
    if let Some(id) = input.id.as_ref().filter(|id| !valid_id(id)) {
        return Err(format!(
            "`id` `{id}` may use letters, digits, `-`, `_`, `.` (at most 64)"
        ));
    }
    if existing.is_none() {
        for (name, missing) in [
            ("title", input.title.is_none()),
            ("body", input.body.is_none()),
            ("path", input.path.is_none()),
            ("lines", input.lines.is_none()),
        ] {
            if missing {
                return Err(format!("`{name}` is required for a new finding"));
            }
        }
    }
    let range = input.lines.as_deref().map(parse_lines).transpose()?;
    if input.path.is_some() != range.is_some() {
        return Err("Pass `path` and `lines` together".into());
    }
    let id = input.id.clone().unwrap_or_else(|| short_id("f"));
    let quoted: Vec<String> = input
        .path
        .as_deref()
        .and_then(|path| resolve_path(&target.review, candidate(&target.review).ok()?, path).ok())
        .into_iter()
        .collect();
    let privacy = provenance(ccx, target, &quoted).await?;
    let saved_id = id.clone();
    let review = mutate(
        target,
        format!("tool:{call_id}"),
        privacy,
        Arc::new(move |store: &ReviewStore, review: &Review| {
            let current = candidate(review)?;
            let old = review.annotations.iter().find(|item| item.id == id);
            let anchors = match (&input.path, range) {
                (Some(path), Some((start, end))) => {
                    let path = resolve_path(review, current, path)?;
                    let side = input.side.clone().unwrap_or(Side::Head);
                    let mut fresh = anchor_in(store, current, &path, side, start, end)?;
                    let mut anchors = old.map(|item| item.anchors.clone()).unwrap_or_default();
                    match anchors.first_mut() {
                        Some(first) if first.position == fresh.position => {}
                        Some(first) => {
                            fresh.history = first.history.clone();
                            fresh.history.push(first.position.clone());
                            *first = fresh;
                        }
                        None => anchors.push(fresh),
                    }
                    anchors
                }
                _ => old
                    .map(|item| item.anchors.clone())
                    .ok_or("`path` and `lines` are required for a new finding")?,
            };
            if anchors
                .iter()
                .any(|anchor| anchor.position.candidate_id != current.id)
            {
                return Err(format!(
                    "Finding `{id}` points at an earlier version of the code; pass `path` and `lines` to place it again"
                ));
            }
            let keep = |fresh: &Option<String>, old: Option<&String>, name: &str| {
                fresh
                    .clone()
                    .or_else(|| old.cloned())
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| format!("`{name}` must not be empty"))
            };
            Ok(Operation::PutAnnotation {
                annotation: Annotation {
                    id: id.clone(),
                    kind: input
                        .kind
                        .clone()
                        .or_else(|| old.map(|item| item.kind.clone()))
                        .unwrap_or(AnnotationKind::Finding),
                    title: keep(&input.title, old.map(|item| &item.title), "title")?,
                    body: keep(&input.body, old.map(|item| &item.body), "body")?,
                    anchors,
                    severity: input
                        .severity
                        .clone()
                        .or_else(|| old.map(|item| item.severity.clone()))
                        .unwrap_or(Severity::Medium),
                    confidence: input
                        .confidence
                        .clone()
                        .or_else(|| old.map(|item| item.confidence.clone()))
                        .unwrap_or(Confidence::Medium),
                    assessment: input
                        .assessment
                        .clone()
                        .or_else(|| old.map(|item| item.assessment.clone()))
                        .unwrap_or(Assessment::Unassessed),
                    disposition: input
                        .disposition
                        .clone()
                        .or_else(|| old.map(|item| item.disposition.clone()))
                        .unwrap_or(Disposition::Open),
                    applicability: Applicability::Current,
                    evidence_ids: old
                        .map(|item| item.evidence_ids.clone())
                        .unwrap_or_default(),
                    run_id: None,
                },
            })
        }),
    )
    .await?;
    let saved = review
        .annotations
        .iter()
        .find(|item| item.id == saved_id)
        .ok_or("The saved finding is unavailable")?;
    let receipt = json!({
        "finding": super::read::finding_line(saved),
        "findings_total": review.annotations.len(),
    });
    Ok((review, receipt, quoted))
}

pub struct SourceInput {
    pub id: Option<String>,
    pub summary: String,
    pub url: Option<String>,
    pub path: Option<String>,
    pub lines: Option<String>,
}

/// Registers something that was consulted: a web page, a document or a file range.
pub async fn source(
    ccx: &Ccx,
    target: &Target,
    input: SourceInput,
    call_id: &str,
) -> Outcome<(Review, Value, Vec<String>)> {
    if input.summary.trim().is_empty() {
        return Err("`summary` must say what this source establishes".into());
    }
    if input.url.is_some() == input.path.is_some() {
        return Err("Pass exactly one of `url` or `path`".into());
    }
    if let Some(id) = input.id.as_ref().filter(|id| !valid_id(id)) {
        return Err(format!(
            "`id` `{id}` may use letters, digits, `-`, `_`, `.` (at most 64)"
        ));
    }
    let range = input.lines.as_deref().map(parse_lines).transpose()?;
    let id = input.id.clone().unwrap_or_else(|| short_id("src"));
    let quoted: Vec<String> = input
        .path
        .as_deref()
        .and_then(|path| resolve_path(&target.review, candidate(&target.review).ok()?, path).ok())
        .into_iter()
        .collect();
    let privacy = provenance(ccx, target, &quoted).await?;
    let saved_id = id.clone();
    let review = mutate(
        target,
        format!("tool:{call_id}"),
        privacy,
        Arc::new(move |store: &ReviewStore, review: &Review| {
            let current = candidate(review)?;
            if review.evidence.iter().any(|item| item.id == id) {
                return Err(format!(
                    "Source `{id}` already exists; sources are append-only, so add a new one"
                ));
            }
            let source = match (&input.url, &input.path) {
                (Some(url), _) => EvidenceSource::External {
                    url: url.clone(),
                },
                (None, Some(path)) => {
                    let path = resolve_path(review, current, path)?;
                    let anchor = range
                        .map(|(start, end)| {
                            anchor_in(store, current, &path, Side::Head, start, end)
                        })
                        .transpose()?;
                    EvidenceSource::Source {
                        path,
                        anchor,
                    }
                }
                (None, None) => return Err("Pass exactly one of `url` or `path`".into()),
            };
            Ok(Operation::AddEvidence {
                evidence: Evidence {
                    id: id.clone(),
                    candidate_id: current.id.clone(),
                    material_id: None,
                    run_id: None,
                    source,
                    summary: input.summary.clone(),
                    applicability: Applicability::Current,
                    created_at_ms: 0,
                },
            })
        }),
    )
    .await?;
    let receipt = json!({
        "source": {"id":saved_id},
        "use": format!("Cite it on a tour page with <refact-evidence ref=\"{saved_id}\"></refact-evidence>"),
        "sources_total": review.evidence.len(),
    });
    Ok((review, receipt, quoted))
}

/// Answers a discussion thread; the reply is saved in the thread and shown in chat.
pub async fn reply(
    ccx: &Ccx,
    target: &Target,
    thread_id: &str,
    body: String,
    call_id: &str,
) -> Outcome<(Review, Value)> {
    if !target
        .review
        .threads
        .iter()
        .any(|thread| thread.id == thread_id)
    {
        return Err(format!(
            "Thread `{thread_id}` is not in this review. Open threads: {}",
            target
                .review
                .threads
                .iter()
                .filter(|thread| !thread.resolved)
                .map(|thread| thread.id.as_str())
                .take(20)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let key = format!("tool:{call_id}");
    let privacy = provenance(ccx, target, &[]).await?;
    let changed = super::service::reply(
        target.app.clone(),
        target.root.clone(),
        target.chat.clone(),
        target.review.id.clone(),
        target.chat.clone(),
        super::service::CommentRequest {
            expected_revision: target.review.revision,
            request_key: key.clone(),
            thread_id: Some(thread_id.to_string()),
            annotation_id: None,
            anchor: None,
            target: None,
            blocks: vec![],
            body,
        },
        privacy,
    )
    .await
    .map_err(super::describe)?;
    let receipt = json!({
        "thread_id": thread_id,
        "contribution_id": super::service::reply_contribution_id(&changed.id, &key),
        "next": "The reply is saved in the thread and shown in chat as its linked message. Do not repeat it; end the turn with one short sentence.",
    });
    Ok((changed, receipt))
}
