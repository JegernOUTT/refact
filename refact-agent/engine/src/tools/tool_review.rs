#[cfg(test)]
#[path = "tool_review/pinning_tests.rs"]
mod pinning_tests;
#[path = "tool_review/schema.rs"]
mod schema;
#[cfg(test)]
#[path = "tool_review/smoke_tests.rs"]
mod smoke_tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use refact_review_core::{Actor, ActorKind, Mutation, Operation, Review, Side};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

#[derive(Clone, Copy)]
pub enum ReviewToolKind {
    Context,
    Get,
    Anchor,
    Update,
    Reply,
    PrepareExecution,
    RecordEvidence,
}

pub struct ToolReview(pub ReviewToolKind);

impl ToolReview {
    fn name(&self) -> &'static str {
        match self.0 {
            ReviewToolKind::Context => "set_review_context",
            ReviewToolKind::Get => "review_get",
            ReviewToolKind::Anchor => "review_anchor",
            ReviewToolKind::Update => "review_update",
            ReviewToolKind::Reply => "review_reply",
            ReviewToolKind::PrepareExecution => "review_prepare_execution",
            ReviewToolKind::RecordEvidence => "review_record_evidence",
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(args: &HashMap<String, Value>) -> Result<T, String> {
    serde_json::from_value(json!(args))
        .map_err(|_| "Invalid review arguments: value outside supported representation".into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetArgs {
    review_id: Option<String>,
    candidate_id: Option<String>,
    object_id: Option<String>,
    object_kind: Option<String>,
    path: Option<String>,
    side: Option<Side>,
    start_line: Option<u32>,
    end_line: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorArgs {
    review_id: Option<String>,
    candidate_id: Option<String>,
    path: String,
    side: Side,
    start_line: u32,
    end_line: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateArgs {
    review_id: Option<String>,
    expected_revision: u64,
    request_key: String,
    operation: Operation,
    context_intake: Option<crate::review::intake::ContextIntakeRequest>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyArgs {
    review_id: Option<String>,
    expected_revision: u64,
    request_key: String,
    thread_id: String,
    body: String,
    #[serde(default)]
    blocks: Vec<refact_review_core::RichBlock>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareArgs {
    request_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceArgs {
    review_id: Option<String>,
    process_id: String,
    summary: String,
    retained_kind: refact_review_core::MaterialKind,
    prepared_material_id: Option<String>,
}

fn bounded_text(value: &str) -> (String, bool) {
    let text: String = value.chars().take(4000).collect();
    let truncated = text.len() < value.len();
    (text, truncated)
}

fn anchors(anchors: &[refact_review_core::Anchor]) -> Vec<Value> {
    anchors.iter().take(50).map(|anchor| json!({
        "position": anchor.position,
        "status": anchor.status,
        "stalled_reason": anchor.stalled_reason.as_deref().map(|reason| bounded_text(reason).0),
        "stalled_reason_truncated": anchor.stalled_reason.as_deref().map_or(false, |reason| bounded_text(reason).1),
        "history_total": anchor.history.len(),
        "history_truncated": !anchor.history.is_empty(),
    })).collect()
}

fn block_inventory(blocks: &[refact_review_core::RichBlock]) -> Value {
    json!({"items": blocks.iter().take(100).map(|b| {
        let kind = match &b.content {
            refact_review_core::RichBlockContent::Markdown { .. } => "markdown",
            refact_review_core::RichBlockContent::Html { .. } => "html",
            refact_review_core::RichBlockContent::Code { .. } => "code",
            refact_review_core::RichBlockContent::Diff { .. } => "diff",
            refact_review_core::RichBlockContent::Sources { .. } => "sources",
            refact_review_core::RichBlockContent::Evidence { .. } => "evidence",
        };
        json!({"id":b.id,"revision":b.revision,"kind":kind,"evidence_ids":refact_review_core::rich_blocks::block_evidence_ids(b).iter().take(100).collect::<Vec<_>>()})
    }).collect::<Vec<_>>(), "total":blocks.len(),"truncated":blocks.len()>100})
}

fn blocks_readable_at(
    review: &Review,
    blocks: &[refact_review_core::RichBlock],
    pinned: &str,
) -> bool {
    crate::review::service::validate_block_candidate(review, blocks, pinned).is_ok()
        && blocks.iter().all(|block| {
            refact_review_core::rich_blocks::block_anchors(block)
                .into_iter()
                .all(|anchor| {
                    anchor.position.candidate_id == pinned
                        && anchor
                            .history
                            .iter()
                            .all(|position| position.candidate_id == pinned)
                })
                && refact_review_core::rich_blocks::block_evidence_ids(block)
                    .iter()
                    .all(|id| exact_object(review, "evidence", id, Some(pinned)).is_ok())
        })
}

fn exact_object(
    review: &Review,
    kind: &str,
    id: &str,
    pinned: Option<&str>,
) -> Result<Value, String> {
    let check_anchors = |groups: Vec<&[refact_review_core::Anchor]>| {
        let all: Vec<_> = groups.into_iter().flatten().collect();
        !all.is_empty()
            && all.iter().all(|anchor| {
                Some(anchor.position.candidate_id.as_str()) == pinned
                    && anchor
                        .history
                        .iter()
                        .all(|position| Some(position.candidate_id.as_str()) == pinned)
            })
    };
    let (value, allowed) = match kind {
        "tour_revision_proposal" => {
            let item = review
                .tour_revision_proposals
                .iter()
                .find(|item| item.id == id)
                .ok_or("Unknown review object")?;
            (
                json!(item),
                Some(item.target.candidate_id.as_str()) == pinned,
            )
        }
        "tour_step" => {
            let item = review
                .tour
                .iter()
                .find(|item| item.id == id)
                .ok_or("Unknown review object")?;
            let bound = std::iter::once(&item.current)
                .chain(item.history.iter())
                .all(|v| {
                    refact_review_core::rich_blocks::tour_version_candidate_id(v)
                        .ok()
                        .flatten()
                        == pinned
                        && (v.anchors.is_empty() || check_anchors(vec![&v.anchors]))
                        && pinned.is_some_and(|p| blocks_readable_at(review, &v.blocks, p))
                });
            (json!(item), bound)
        }
        "annotation" => {
            let item = review
                .annotations
                .iter()
                .find(|item| item.id == id)
                .ok_or("Unknown review object")?;
            (json!(item), check_anchors(vec![&item.anchors]))
        }
        "thread" => {
            let item = review
                .threads
                .iter()
                .find(|item| item.id == id)
                .ok_or("Unknown review object")?;
            let contexts_match = item.contributions.iter().all(|contribution| {
                contribution
                    .context
                    .as_ref()
                    .and_then(|context| context.candidate_id.as_deref())
                    == pinned
            });
            (
                json!(item),
                (item.anchors.is_empty() || check_anchors(vec![&item.anchors]))
                    && item
                        .target
                        .as_ref()
                        .map_or(!item.anchors.is_empty(), |target| {
                            Some(target.candidate_id.as_str()) == pinned
                        })
                    && contexts_match
                    && item
                        .contributions
                        .iter()
                        .all(|c| pinned.is_some_and(|p| blocks_readable_at(review, &c.blocks, p))),
            )
        }
        "evidence" => {
            let item = review
                .evidence
                .iter()
                .find(|item| item.id == id)
                .ok_or("Unknown review object")?;
            let source_matches = match &item.source {
                refact_review_core::EvidenceSource::Source {
                    anchor: Some(anchor),
                    ..
                } => check_anchors(vec![std::slice::from_ref(anchor)]),
                _ => true,
            };
            (
                json!(item),
                Some(item.candidate_id.as_str()) == pinned && source_matches,
            )
        }
        "material" => {
            let item = review
                .materials
                .iter()
                .find(|item| item.id == id)
                .ok_or("Unknown review object")?;
            (json!(item), Some(item.candidate_id.as_str()) == pinned)
        }
        "run" => {
            let item = review
                .runs
                .iter()
                .find(|item| item.id == id)
                .ok_or("Unknown review object")?;
            (json!(item), Some(item.candidate_id.as_str()) == pinned)
        }
        _ => return Err("Unknown review object kind".into()),
    };
    if pinned.is_some() && !allowed {
        return Err("Review object is not provably bound to the inspected candidate".into());
    }
    Ok(value)
}

fn material_summary(material: &refact_review_core::Material) -> Value {
    let inputs: std::collections::BTreeMap<_, _> = material
        .input_digests
        .iter()
        .filter(|(key, _)| key.as_str() != "derived_origin_labels")
        .take(100)
        .collect();
    let input_total = material.input_digests.len()
        - usize::from(material.input_digests.contains_key("derived_origin_labels"));
    let (description, description_truncated) = bounded_text(&material.description);
    let environment = material.environment.as_deref().map(bounded_text);
    json!({
        "id": material.id, "kind": material.kind, "candidate_id": material.candidate_id,
        "agent_id": material.agent_id, "description": description, "description_truncated": description_truncated,
        "content_digest": material.content_digest, "input_digests": inputs,
        "input_digests_total": input_total, "input_digests_truncated": input_total > 100,
        "derived_origin_labels_omitted": material.input_digests.contains_key("derived_origin_labels"),
        "environment": environment.as_ref().map(|(text, _)| text),
        "environment_truncated": environment.as_ref().map_or(false, |(_, truncated)| *truncated),
        "worktree_id": material.worktree_id, "worktree_root": material.worktree_root,
        "artifacts": material.artifacts.iter().take(100).collect::<Vec<_>>(),
        "artifacts_total": material.artifacts.len(), "artifacts_truncated": material.artifacts.len() > 100,
    })
}

pub fn summary(review: &Review) -> Value {
    json!({
        "projection": "summary",
        "exact_object_lookup": "Use review_get with object_kind and object_id to read a complete object before updating it; summaries are not mutation payloads.",
        "id": review.id,
        "owner_chat_id": review.owner_chat_id,
        "revision": review.revision,
        "title": review.title,
        "target_reference": review.target_reference,
        "current_candidate_id": review.current_candidate_id,
        "context_intake": review.context_intake.as_ref().map(crate::review::intake_runtime::summary),
        "pending_tour_revision_actions": crate::review::proposals::pending(review),
        "candidates": review.candidates.iter().map(|candidate| json!({
            "id": candidate.id,
            "base_oid": candidate.base_oid,
            "head_oid": candidate.head_oid,
            "content_digest": candidate.content_digest,
            "repository": candidate.repository,
            "policy": candidate.policy,
            "files_total": candidate.files.len(),
            "changed_files": candidate.files.iter().filter(|file| file.base != file.head || file.base_mode != file.head_mode).take(200).map(|file| &file.path).collect::<Vec<_>>(),
            "changed_files_total": candidate.files.iter().filter(|file| file.base != file.head || file.base_mode != file.head_mode).count(),
        })).collect::<Vec<_>>(),
        "tour": review.tour.iter().rev().take(50).map(|step| {
            let (title, title_truncated) = bounded_text(&step.current.title);
            let (body, body_truncated) = bounded_text(&step.current.body);
            json!({"id":step.id,"current":{"version":step.current.version,"candidate_id":step.current.candidate_id,"blocks":block_inventory(&step.current.blocks),"title":title,"title_truncated":title_truncated,"body":body,"body_truncated":body_truncated,"anchors":anchors(&step.current.anchors),"anchors_total":step.current.anchors.len(),"anchors_truncated":step.current.anchors.len()>50},"history_total":step.history.len(),"history_truncated":!step.history.is_empty()})
        }).collect::<Vec<_>>(),
        "tour_total": review.tour.len(),
        "annotations": review.annotations.iter().rev().take(50).map(|item| {
            let (title, title_truncated) = bounded_text(&item.title);
            let (body, body_truncated) = bounded_text(&item.body);
            json!({"id":item.id,"kind":item.kind,"title":title,"title_truncated":title_truncated,"body":body,"body_truncated":body_truncated,"anchors":anchors(&item.anchors),"anchors_total":item.anchors.len(),"anchors_truncated":item.anchors.len()>50,"severity":item.severity,"confidence":item.confidence,"assessment":item.assessment,"disposition":item.disposition,"applicability":item.applicability,"evidence_ids":item.evidence_ids.iter().take(100).collect::<Vec<_>>(),"evidence_ids_total":item.evidence_ids.len(),"evidence_ids_truncated":item.evidence_ids.len()>100,"run_id":item.run_id})
        }).collect::<Vec<_>>(),
        "annotations_total": review.annotations.len(),
        "threads": review.threads.iter().rev().take(20).map(|thread| json!({
            "id":thread.id,"target":thread.target,"anchors":anchors(&thread.anchors),"anchors_total":thread.anchors.len(),"anchors_truncated":thread.anchors.len()>50,"annotation_id":thread.annotation_id,"resolved":thread.resolved,
            "contributions":thread.contributions.iter().rev().take(20).map(|item| {
                let (body, body_truncated) = bounded_text(&item.body);
                json!({"id":item.id,"thread_id":item.thread_id,"actor":item.actor,"blocks":block_inventory(&item.blocks),"body":body,"body_truncated":body_truncated,"created_at_ms":item.created_at_ms,"chat_message_id":item.chat_message_id,"context":item.context.as_ref().map(|context| json!({"candidate_id":context.candidate_id,"run_id":context.run_id,"applicability":context.applicability,"privacy_total":context.privacy.len(),"privacy_omitted":!context.privacy.is_empty()}))})
            }).collect::<Vec<_>>(),"contributions_total":thread.contributions.len(),"contributions_truncated":thread.contributions.len()>20
        })).collect::<Vec<_>>(),
        "threads_total": review.threads.len(),
        "evidence": review.evidence.iter().rev().take(50).map(|item| {
            let (text, truncated) = bounded_text(&item.summary);
            let source = match &item.source {
                refact_review_core::EvidenceSource::Source { path, anchor } => json!({"kind":"source","path":path,"anchor":anchor.as_ref().map(|a| anchors(std::slice::from_ref(a)).remove(0))}),
                refact_review_core::EvidenceSource::External { url } => { let (url, truncated) = bounded_text(url); json!({"kind":"external","url":url,"url_truncated":truncated}) },
                refact_review_core::EvidenceSource::Executed { command, workdir, exit_code, stdout, stderr } => {
                    let args = command.iter().take(50).map(|arg| bounded_text(arg).0).collect::<Vec<_>>();
                    let args_truncated = command.iter().take(50).any(|arg| bounded_text(arg).1);
                    let (workdir, workdir_truncated) = bounded_text(workdir);
                    json!({"kind":"executed","command":args,"command_total":command.len(),"command_truncated":command.len()>50 || args_truncated,"workdir":workdir,"workdir_truncated":workdir_truncated,"exit_code":exit_code,"stdout":stdout,"stderr":stderr})
                }
            };
            json!({"id":item.id,"candidate_id":item.candidate_id,"material_id":item.material_id,"run_id":item.run_id,"source":source,"summary":text,"summary_truncated":truncated,"applicability":item.applicability,"created_at_ms":item.created_at_ms})
        }).collect::<Vec<_>>(),
        "evidence_total": review.evidence.len(),
        "runs": review.runs.iter().rev().take(30).map(|run| {
            let (purpose, purpose_truncated) = bounded_text(&run.purpose);
            let error = run.error.as_deref().map(bounded_text);
            json!({"id":run.id,"candidate_id":run.candidate_id,"agent_id":run.agent_id,"purpose":purpose,"purpose_truncated":purpose_truncated,"model":run.model,"domain":run.domain,"status":run.status,"started_at_ms":run.started_at_ms,"finished_at_ms":run.finished_at_ms,"error":error.as_ref().map(|(text, _)| text),"error_truncated":error.as_ref().map_or(false, |(_, truncated)| *truncated)})
        }).collect::<Vec<_>>(),
        "runs_total": review.runs.len(), "runs_truncated": review.runs.len()>30,
        "materials": review.materials.iter().rev().take(30).map(material_summary).collect::<Vec<_>>(),
        "materials_total": review.materials.len(), "materials_truncated": review.materials.len()>30,
    })
}

fn pinned_summary(review: &Review, pinned: &str) -> Value {
    let mut projection = review.clone();
    projection.current_candidate_id = Some(pinned.to_owned());
    projection
        .tour_revision_proposals
        .retain(|p| p.target.candidate_id == pinned);
    projection.candidates.retain(|item| item.id == pinned);
    projection
        .tour
        .retain(|item| exact_object(review, "tour_step", &item.id, Some(pinned)).is_ok());
    projection
        .annotations
        .retain(|item| exact_object(review, "annotation", &item.id, Some(pinned)).is_ok());
    projection
        .threads
        .retain(|item| exact_object(review, "thread", &item.id, Some(pinned)).is_ok());
    projection
        .evidence
        .retain(|item| exact_object(review, "evidence", &item.id, Some(pinned)).is_ok());
    projection
        .materials
        .retain(|item| item.candidate_id == pinned);
    projection.runs.retain(|item| item.candidate_id == pinned);
    let mut value = summary(&projection);
    value["projection"] = json!("pinned_summary");
    value["inspected_candidate_id"] = json!(pinned);
    value["scope_notice"] = json!("Only objects wholly attributable to the inspected candidate are included. Mixed-history objects are withheld; this is not the owner's current-candidate view.");
    value
}

async fn result_message(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    call_id: &str,
    review: &Review,
    mut value: Value,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let gcx = ccx.lock().await.app.gcx.clone();
    let pinned = if ccx.lock().await.background_agent_id.is_some() {
        Some(crate::review::runs::inspected_candidate(ccx.clone(), review).await?)
    } else {
        None
    };
    if value.get("projection").and_then(Value::as_str) == Some("summary") {
        if let Some(pinned) = &pinned {
            value = pinned_summary(review, pinned);
        }
    }
    let paths = review
        .candidates
        .iter()
        .filter(|candidate| pinned.as_ref().is_none_or(|id| &candidate.id == id))
        .flat_map(|candidate| {
            let repository = PathBuf::from(&candidate.repository);
            candidate
                .files
                .iter()
                .map(move |file| repository.join(&file.path))
        })
        .collect::<Vec<_>>();
    let mut records = crate::privacy::records::declared_file_records(&gcx, paths)?;
    records.extend(crate::review::intake_runtime::privacy(review));
    let mut message = ChatMessage {
        role: "tool".into(),
        tool_call_id: call_id.into(),
        content: ChatContent::SimpleText(value.to_string()),
        ..Default::default()
    };
    message.extra.insert("review".into(), json!({"id":review.id,"revision":review.revision,"candidate_id":review.current_candidate_id}));
    crate::privacy::records::merge_records(&mut message, records);
    Ok((false, vec![ContextEnum::ChatMessage(message)]))
}

#[async_trait]
impl Tool for ToolReview {
    fn tool_description(&self) -> ToolDesc {
        let mut properties = json!({"review_id":{"type":"string","description":"Review owned by this chat or this runtime-bound reviewer. Omit for the current review."}});
        let (display_name, description, required) = match self.0 {
            ReviewToolKind::Context => {
                properties = json!({
                    "repository":{"type":"string","description":"Prepared authorized local Git repository. Acquire remote targets first with existing tools."},
                    "base_ref":{"type":"string","description":"Explicit comparison base; use a resolved merge-base when reviewing branch changes."},
                    "candidate_ref":{"type":"string","description":"Local candidate commit or branch to pin."},
                    "policy":{"type":"object","properties":{"staged":{"type":"boolean"},"working":{"type":"boolean"},"untracked":{"type":"boolean"}},"required":["staged","working","untracked"],"additionalProperties":false},
                    "target_reference":{"type":"string","description":"Original target URL or branch reference for display and explicit later refresh."},
                    "title":{"type":"string"},
                    "review_id":{"type":"string","description":"Existing review to update, preserving all prior discussions and material."},
                    "expected_revision":{"type":"integer","minimum":0},
                    "request_key":{"type":"string","description":"Stable idempotency key. Reuse only for retries of the identical request."}
                });
                properties["action"] = json!({"type":"string","enum":["stack_prepare","stack_import","stack_inspect","stack_member","stack_capture","stack_cumulative","stack_refresh"]});
                properties["manifest"] = json!({"type":"object","description":"Explicit acquired StackManifest with local refs and source provenance; prepare validates and returns the exact ask_questions prompt before any capture."});
                properties["member_id"] = json!({"type":"string"});
                properties["scope_receipt_tool_call_id"] = json!({"type":"string","description":"Actual answered ask_questions call in this owning chat. Booleans or model assertions are not consent."});
                ("Set Review Context", "After context searches and before capture or Tour mutation, call action context_sources with phase before_review or before_tour (optional review_id for Tour) to discover actual completed retrieval IDs in this owning chat. Cite returned tool_call_id values in context_intake; metadata is not consent or admission. For known Graphite stacks FIRST call stack_prepare with discovery (title, source, selected_member, members and comparison_mode), present its exact question using ask_questions and stop. After USER answers, stack_import with acquisition (preparation_token, actual scope_receipt_tool_call_id, request_key, comparisons, optional root_base_ref); only then stack_capture with member_id and request_key activates one member. stack_inspect, stack_member and stack_cumulative are read-only and never activate execution. No automatic expansion/download or Graphite mutations. Without action: attach a prepared local comparison to this same chat and Review Canvas. Captures immutable base/candidate material and creates a registered isolated investigation checkout. Does not download code, overwrite the user's checkout, publish remotely, or grant authority. Updates preserve history and stall uncertain anchors. Runtime-root only; existing reviewers remain pinned to the material they inspected.", vec![])
            }
            ReviewToolKind::Get => {
                properties["candidate_id"] = json!({"type":"string","description":"Immutable candidate to read. A child may read only its pinned candidate."});
                properties["object_id"] = json!({"type":"string","description":"ID of one exact object; requires object_kind and cannot be combined with path, range, or candidate_id."});
                properties["object_kind"] = json!({"type":"string","enum":["tour_step","tour_revision_proposal","annotation","thread","evidence","material","run"],"description":"Kind of object to read with object_id."});
                properties["path"] = json!({"type":"string","description":"Optional candidate-relative file to read from the immutable snapshot, not the mutable investigation checkout."});
                properties["side"] = json!({"type":"string","enum":["base","head"]});
                properties["start_line"] = json!({"type":"integer","minimum":1});
                properties["end_line"] = json!({"type":"integer","minimum":1,"description":"Inclusive range, at most 500 lines and 64000 characters per read."});
                ("Read Review", "Read a bounded review summary or an exact immutable file range. Summary objects are projections, not mutation payloads: before updating an existing object, read it with object_kind and object_id to preserve protected history and contributions. Exact object reads may be large and child reads require provable pinned-candidate attribution. Investigation cat results may have experimental edits: use this tool for original candidate source. Use review_anchor for validated anchors.", vec![])
            }
            ReviewToolKind::Anchor => {
                properties["candidate_id"] = json!({"type":"string","description":"Candidate inspected. Defaults to this runtime's pinned candidate, not a newer candidate for a late reviewer."});
                properties["path"] = json!({"type":"string"});
                properties["side"] = json!({"type":"string","enum":["base","head"]});
                properties["start_line"] = json!({"type":"integer","minimum":1});
                properties["end_line"] = json!({"type":"integer","minimum":1});
                ("Anchor Review Range", "Create a server-validated immutable source anchor with exact candidate, path, side, line range and content identity. Use returned anchors in tour steps, highlights, findings and sources; never invent hashes.", vec!["path","side","start_line","end_line"])
            }
            ReviewToolKind::Update => {
                properties["expected_revision"] = json!({"type":"integer","minimum":0});
                properties["request_key"] = json!({"type":"string"});
                properties["operation"] = json!({"type":"object","description":"Typed review operation. op: put_tour_step {step:{id,current:{version,title,body,anchors},history:[]}}, put_annotation {annotation:{id,kind:explanation|highlight|finding,title,body,anchors,severity:info|low|medium|high|critical,confidence:unknown|low|medium|high,assessment:unassessed|supported|disputed|refuted,disposition:open|repair_proposed|fixed|risk_accepted|dismissed,applicability:current|needs_revalidation|historical|no_longer_applicable|unknown,evidence_ids,run_id}}, put_thread {thread}, rename {title}, or add_evidence {evidence} for source/external assertions only. Preserve existing object histories. Runtime attribution is automatic; execution evidence must use review_record_evidence. Candidate activation, fabricated runtime results, and direct contribution insertion are refused."});
                ("Update Review", "Add durable Tour explanations, findings and source evidence. Chat-led Tour revisions use propose_tour_revision {target,thread_id,change_request,body,blocks?}, storing one canonical assistant contribution; do not repeat it in chat or review_reply. Acceptance is exclusively an explicit USER UI action, never model self-consent. After acceptance use apply_tour_revision {proposal_id,current} for that exact scope. Pending intent never automatically dispatches authoring. Both operations require the owning foreground active review. Source interpretation is not execution. Conflicts require rereading and reconciliation.", vec!["expected_revision","request_key","operation"])
            }
            ReviewToolKind::Reply => {
                properties["expected_revision"] = json!({"type":"integer","minimum":0});
                properties["request_key"] = json!({"type":"string"});
                properties["thread_id"] = json!({"type":"string"});
                properties["body"] = json!({"type":"string"});
                ("Reply in Review", "Answer a discussion with one canonical linked contribution. Ordinary replies have no obligation to revise the Tour. For an explanation improvement use review_update propose_tour_revision instead, wait for explicit user acceptance, then apply_tour_revision; a reply is not consent. Source-only review does not permit repository edits or execution. Do not separately post the same reply in chat. Attribution remains runtime-bound.", vec!["expected_revision","request_key","thread_id","body"])
            }
            ReviewToolKind::PrepareExecution => {
                properties = json!({"request_key":{"type":"string","description":"Stable preparation identity for this deliberate execution."}});
                ("Prepare Review Execution", "Retain the current saved investigation inputs before starting a verification process with existing execution tools. Returns a material identity for review_record_evidence. This does not execute anything, grant permission, capture unsaved editor buffers, or establish a complete effective environment.", vec!["request_key"])
            }
            ReviewToolKind::RecordEvidence => {
                properties["process_id"] = json!({"type":"string","description":"Actual completed process owned by this runtime chat."});
                properties["summary"] = json!({"type":"string"});
                properties["retained_kind"] =
                    json!({"type":"string","enum":["investigation","repair"]});
                properties["prepared_material_id"] = json!({"type":"string","description":"Input material returned by review_prepare_execution before this process began. Omit for explicitly unknown post-hoc attribution."});
                ("Record Review Execution Evidence", "Record an actual owned execution-registry result and retain saved investigation or repair material. Command, workdir, status and output come from the process registry, not model assertions. Input drift and unknown environment remain explicit; a passing modified experiment never proves the imported candidate passed.", vec!["process_id","summary","retained_kind"])
            }
        };
        ToolDesc {
            name: self.name().into(),
            display_name: display_name.into(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: matches!(self.0, ReviewToolKind::Get | ReviewToolKind::Anchor),
            description: description.into(),
            input_schema: schema::advertised(
                self.0,
                json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            ),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let advertised = self.tool_description().input_schema;
        let validation = if matches!(self.0, ReviewToolKind::Context) {
            schema::schema(self.0, advertised)
        } else {
            advertised
        };
        schema::validate(&validation, &json!(args))?;
        if matches!(self.0, ReviewToolKind::Context) {
            if let Some(action) = args.get("action").and_then(Value::as_str) {
                let (app, chat) = {
                    let context = ccx.lock().await;
                    if context.background_agent_id.is_some()
                        || context.subchat_depth != 0
                        || context.task_meta.is_some()
                    {
                        return Err("Stack orchestration requires the owning root chat".into());
                    }
                    (context.app.clone(), context.chat_id.clone())
                };
                if action == "context_sources" {
                    let phase = args
                        .get("phase")
                        .and_then(Value::as_str)
                        .ok_or("phase required")?;
                    let candidate = if phase == "before_tour" {
                        let (_, review) = crate::review::runs::bound_review(
                            ccx.clone(),
                            args.get("review_id").and_then(Value::as_str),
                        )
                        .await?;
                        Some(review.current_candidate_id.ok_or("No active candidate")?)
                    } else {
                        if args.contains_key("review_id") {
                            return Err(
                                "review_id is only supported for before_tour context sources"
                                    .into(),
                            );
                        }
                        None
                    };
                    let (value, records) = crate::review::intake_runtime::context_sources(
                        &app,
                        &chat,
                        phase,
                        candidate.as_deref(),
                    )
                    .await?;
                    let mut message = ChatMessage {
                        role: "tool".into(),
                        tool_call_id: tool_call_id.clone(),
                        content: ChatContent::SimpleText(value.to_string()),
                        ..Default::default()
                    };
                    crate::privacy::records::merge_records(&mut message, records);
                    return Ok((false, vec![ContextEnum::ChatMessage(message)]));
                }
                let member = || {
                    args.get("member_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or("member_id required".to_owned())
                };
                let key = || {
                    args.get("request_key")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or("request_key required".to_owned())
                };
                if action == "stack_capture" {
                    let receipt =
                        crate::review::stacks::capture(ccx.clone(), member()?, key()?).await?;
                    return result_message(&ccx, tool_call_id, &receipt.review, json!({"review":summary(&receipt.review),"worktree":receipt.worktree,"candidate_id":receipt.candidate_id})).await;
                }
                let value = match action {
                    "context_question" => {
                        let phase = args
                            .get("phase")
                            .and_then(Value::as_str)
                            .ok_or("phase required")?;
                        let (repository, base, head, candidate) = if phase == "before_tour" {
                            let (_, review) = crate::review::runs::bound_review(
                                ccx.clone(),
                                args.get("review_id").and_then(Value::as_str),
                            )
                            .await?;
                            let candidate = review
                                .candidates
                                .iter()
                                .find(|c| Some(&c.id) == review.current_candidate_id.as_ref())
                                .ok_or("No active candidate")?;
                            (
                                candidate.repository.clone(),
                                candidate.base_oid.clone(),
                                candidate.head_oid.clone(),
                                Some(candidate.id.clone()),
                            )
                        } else {
                            let repository = dunce::canonicalize(
                                args.get("repository")
                                    .and_then(Value::as_str)
                                    .ok_or("repository required")?,
                            )
                            .map_err(|e| e.to_string())?;
                            crate::files_in_workspace::check_file_privacy_for_model_context(
                                app.gcx.clone(),
                                &repository,
                            )
                            .await?;
                            let repo =
                                git2::Repository::open(&repository).map_err(|e| e.to_string())?;
                            let resolve = |name: &str| -> Result<String, String> {
                                Ok(repo
                                    .revparse_single(
                                        args.get(name)
                                            .and_then(Value::as_str)
                                            .ok_or("comparison ref required")?,
                                    )
                                    .and_then(|o| o.peel_to_commit())
                                    .map_err(|e| e.to_string())?
                                    .id()
                                    .to_string())
                            };
                            (
                                repository.to_string_lossy().into_owned(),
                                resolve("base_ref")?,
                                resolve("candidate_ref")?,
                                None,
                            )
                        };
                        let q = crate::review::intake::expected_question(
                            phase,
                            &repository,
                            &base,
                            &head,
                            candidate.as_deref(),
                        );
                        json!({"ask_questions":{"questions":[{"id":q.id,"type":q.question_type,"text":q.text,"options":q.options}]}})
                    }
                    "stack_prepare" => serde_json::to_value(
                        crate::review::stack_entry::prepare(
                            &app,
                            &chat,
                            serde_json::from_value(
                                args.get("discovery").cloned().ok_or("discovery required")?,
                            )
                            .map_err(|_| "Invalid discovery".to_owned())?,
                        )
                        .await?,
                    )
                    .map_err(|e| e.to_string())?,
                    "stack_import" => json!(
                        crate::review::stack_entry::import_with_intake(
                            &app,
                            &chat,
                            serde_json::from_value(
                                args.get("acquisition")
                                    .cloned()
                                    .ok_or("acquisition required")?
                            )
                            .map_err(|_| "Invalid acquisition".to_owned())?,
                            args.get("context_intake")
                                .cloned()
                                .map(serde_json::from_value)
                                .transpose()
                                .map_err(|_| "Invalid context intake")?
                        )
                        .await?
                    ),
                    "stack_inspect" => json!(crate::review::stacks::get(&app, &chat).await?),
                    "stack_member" => {
                        if let Some(review) =
                            crate::review::stacks::member(&app, &chat, member()?).await?
                        {
                            return result_message(&ccx, tool_call_id, &review, summary(&review))
                                .await;
                        }
                        Value::Null
                    }
                    "stack_cumulative" => {
                        let mut value =
                            crate::review::stacks::cumulative(&app, &chat, member()?).await?;
                        let candidate: refact_review_core::Candidate =
                            serde_json::from_value(value["candidate"].clone())
                                .map_err(|_| "Invalid cumulative candidate")?;
                        let paths = candidate
                            .files
                            .iter()
                            .map(|file| PathBuf::from(&candidate.repository).join(&file.path))
                            .collect::<Vec<_>>();
                        let records =
                            crate::privacy::records::declared_file_records(&app.gcx, paths)?;
                        value["candidate"] = json!({"id":candidate.id,"base_oid":candidate.base_oid,"head_oid":candidate.head_oid,"content_digest":candidate.content_digest,"repository":candidate.repository,"files_total":candidate.files.len(),"changed_files":candidate.files.iter().filter(|f| f.base != f.head || f.base_mode != f.head_mode).take(200).map(|f| &f.path).collect::<Vec<_>>(),"changed_files_total":candidate.files.iter().filter(|f| f.base != f.head || f.base_mode != f.head_mode).count(),"read_path":"Use the Review Canvas cumulative comparison to inspect all files."});
                        let mut message = ChatMessage {
                            role: "tool".into(),
                            tool_call_id: tool_call_id.clone(),
                            content: ChatContent::SimpleText(value.to_string()),
                            ..Default::default()
                        };
                        crate::privacy::records::merge_records(&mut message, records);
                        return Ok((false, vec![ContextEnum::ChatMessage(message)]));
                    }
                    "stack_refresh" => json!(
                        crate::review::stacks::refresh(
                            &app,
                            &chat,
                            key()?,
                            args.get("expected_revision")
                                .and_then(Value::as_u64)
                                .ok_or("expected_revision required")?
                        )
                        .await?
                    ),
                    _ => return Err("Unknown stack action".into()),
                };
                let mut message = ChatMessage {
                    role: "tool".into(),
                    tool_call_id: tool_call_id.clone(),
                    content: ChatContent::SimpleText(value.to_string()),
                    ..Default::default()
                };
                if action == "stack_import" {
                    let root = crate::review::service::trusted_root(&app, &chat).await?;
                    let stack: refact_review_core::StackManifest =
                        serde_json::from_value(value.clone())
                            .map_err(|_| "Invalid published stack")?;
                    let mut members = Vec::new();
                    for member in stack.members {
                        if let Some(id) = member.review_id {
                            let review =
                                crate::review::service::load_owned(root.clone(), chat.clone(), id)
                                    .await
                                    .map_err(|e| e.to_string())?;
                            members.push(json!({"review_id":review.id,"candidate_id":review.current_candidate_id}));
                            crate::privacy::records::merge_records(
                                &mut message,
                                crate::review::intake_runtime::privacy(&review),
                            );
                        }
                    }
                    message
                        .extra
                        .insert("review_intake_members".into(), json!(members));
                }
                return Ok((false, vec![ContextEnum::ChatMessage(message)]));
            }
            let intake = serde_json::from_value(
                args.get("context_intake")
                    .cloned()
                    .ok_or("context_intake required before Review creation")?,
            )
            .map_err(|_| "Invalid context intake")?;
            let mut capture = args.clone();
            capture.remove("context_intake");
            let request = parse(&capture)?;
            let receipt =
                crate::review::context::set_context_with_intake(ccx.clone(), request, intake)
                    .await?;
            let value = json!({"review":summary(&receipt.review),"candidate_id":receipt.candidate_id,"execution_commit":receipt.execution_commit,"worktree":receipt.worktree,"imported_material_id":receipt.imported_material_id,"investigation_material_id":receipt.investigation_material_id});
            return result_message(&ccx, tool_call_id, &receipt.review, value).await;
        }
        let review_id = args.get("review_id").and_then(Value::as_str);
        let (root, review) = crate::review::runs::bound_review(ccx.clone(), review_id).await?;
        if matches!(
            self.0,
            ReviewToolKind::PrepareExecution | ReviewToolKind::RecordEvidence
        ) {
            let context = ccx.lock().await;
            if context.background_agent_id.is_none() {
                let active =
                    crate::review::service::get_for_chat(root.clone(), context.chat_id.clone())
                        .await
                        .map_err(|e| e.to_string())?;
                if active.as_ref().map(|r| &r.id) != Some(&review.id) {
                    return Err("Execution evidence requires the active review checkout; browsing does not activate execution".into());
                }
            }
        }
        match self.0 {
            ReviewToolKind::PrepareExecution => {
                let request: PrepareArgs = parse(args)?;
                let material =
                    crate::review::evidence::prepare_execution(ccx.clone(), &request.request_key)
                        .await?;
                result_message(&ccx, tool_call_id, &review, json!(material)).await
            }
            ReviewToolKind::RecordEvidence => {
                let request: EvidenceArgs = parse(args)?;
                let _ = request.review_id;
                let changed = crate::review::evidence::record_prepared_process_evidence(
                    ccx.clone(),
                    &review.id,
                    &request.process_id,
                    &request.summary,
                    request.retained_kind,
                    request.prepared_material_id.as_deref(),
                )
                .await?;
                let app = ccx.lock().await.app.clone();
                crate::review::chat_bridge::invalidate(&app, &changed).await;
                result_message(&ccx, tool_call_id, &changed, summary(&changed)).await
            }
            ReviewToolKind::Get => {
                let request: GetArgs = parse(args)?;
                let _ = request.review_id;
                if request.object_id.is_some() != request.object_kind.is_some() {
                    return Err("object_id and object_kind must be supplied together".into());
                }
                let value = if let (Some(id), Some(kind)) =
                    (&request.object_id, &request.object_kind)
                {
                    if request.path.is_some()
                        || request.candidate_id.is_some()
                        || request.side.is_some()
                        || request.start_line.is_some()
                        || request.end_line.is_some()
                    {
                        return Err("Object lookup cannot be combined with path, candidate or range options".into());
                    }
                    let is_child = ccx.lock().await.background_agent_id.is_some();
                    let pinned = if is_child {
                        Some(crate::review::runs::inspected_candidate(ccx.clone(), &review).await?)
                    } else {
                        None
                    };
                    exact_object(&review, kind, id, pinned.as_deref())?
                } else if let Some(path) = request.path {
                    let inspected =
                        crate::review::runs::inspected_candidate(ccx.clone(), &review).await?;
                    let candidate_id = request.candidate_id.unwrap_or(inspected.clone());
                    if ccx.lock().await.background_agent_id.is_some() && candidate_id != inspected {
                        return Err(
                            "A reviewer may read only its pinned immutable candidate".into()
                        );
                    }
                    let candidate = review
                        .candidates
                        .iter()
                        .find(|candidate| candidate.id == candidate_id)
                        .cloned()
                        .ok_or("Unknown review candidate")?;
                    let side = request.side.unwrap_or(Side::Head);
                    let file = crate::review::service::with_store(root, move |store| {
                        store.file(&candidate, &path)
                    })
                    .await
                    .map_err(|error| error.to_string())?;
                    let bytes = match side {
                        Side::Base => file.base,
                        Side::Head => file.head,
                    }
                    .ok_or("File is absent on the selected side")?;
                    let text = String::from_utf8(bytes).map_err(|_| "Snapshot file is binary or not UTF-8; inspect the binary file notice in the Canvas")?;
                    let total_lines = text.lines().count();
                    let start = request.start_line.unwrap_or(1);
                    let end = request.end_line.unwrap_or(start.saturating_add(199));
                    if start == 0
                        || end < start
                        || end - start >= 500
                        || start as usize > total_lines.max(1)
                    {
                        return Err("Invalid snapshot range; request 1–500 existing lines".into());
                    }
                    let mut characters = 0usize;
                    let lines = text
                        .lines()
                        .enumerate()
                        .skip(start as usize - 1)
                        .take((end - start + 1) as usize)
                        .take_while(|(_, line)| {
                            characters = characters.saturating_add(line.chars().count());
                            characters <= 64_000
                        })
                        .map(|(index, line)| json!({"line":index+1,"text":line}))
                        .collect::<Vec<_>>();
                    json!({"candidate_id":candidate_id,"path":file.path,"side":side,"total_lines":total_lines,"shown_lines":lines.len(),"truncated":lines.len() < total_lines.saturating_sub(start as usize - 1).min((end-start+1) as usize),"lines":lines})
                } else {
                    if request.side.is_some()
                        || request.start_line.is_some()
                        || request.end_line.is_some()
                        || request.candidate_id.is_some()
                    {
                        return Err("A snapshot file path is required for range options".into());
                    }
                    summary(&review)
                };
                result_message(&ccx, tool_call_id, &review, value).await
            }
            ReviewToolKind::Anchor => {
                let request: AnchorArgs = parse(args)?;
                let _ = request.review_id;
                let inspected =
                    crate::review::runs::inspected_candidate(ccx.clone(), &review).await?;
                let candidate_id = request.candidate_id.unwrap_or(inspected.clone());
                let is_child = ccx.lock().await.background_agent_id.is_some();
                if is_child && candidate_id != inspected {
                    return Err("A reviewer can anchor only the candidate bound to its run".into());
                }
                let candidate = review
                    .candidates
                    .iter()
                    .find(|candidate| candidate.id == candidate_id)
                    .cloned()
                    .ok_or("Unknown review candidate")?;
                let anchor = crate::review::service::with_store(root, move |store| {
                    store.anchor(
                        &candidate,
                        &request.path,
                        request.side,
                        request.start_line,
                        request.end_line,
                    )
                })
                .await
                .map_err(|error| error.to_string())?;
                result_message(&ccx, tool_call_id, &review, json!(anchor)).await
            }
            ReviewToolKind::Update => {
                let request: UpdateArgs = parse(args)?;
                let _ = request.review_id;
                if matches!(
                    &request.operation,
                    Operation::PutTourStep { .. }
                        | Operation::ProposeTourRevision { .. }
                        | Operation::ApplyTourRevision { .. }
                ) {
                    let c = ccx.lock().await;
                    if c.background_agent_id.is_some()
                        || c.subchat_depth != 0
                        || c.task_meta.is_some()
                    {
                        return Err("Tour authoring requires the owning foreground chat".into());
                    }
                }
                let (app, agent_id) = {
                    let context = ccx.lock().await;
                    (
                        context.app.clone(),
                        context
                            .background_agent_id
                            .clone()
                            .unwrap_or_else(|| context.chat_id.clone()),
                    )
                };
                let mutation = Mutation {
                    expected_revision: request.expected_revision,
                    request_key: request.request_key,
                    actor: Actor {
                        kind: ActorKind::Agent,
                        id: agent_id.clone(),
                    },
                    at_ms: chrono::Utc::now().timestamp_millis(),
                    operation: request.operation,
                };
                let intake = if matches!(
                    &mutation.operation,
                    Operation::PutTourStep { .. } | Operation::ApplyTourRevision { .. }
                ) {
                    let scope = review.id.clone();
                    let key = mutation.request_key.clone();
                    let saved = crate::review::service::with_store(root.clone(), move |s| {
                        s.intake_for_request(&scope, &key)
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                    if let Some(saved) = saved {
                        crate::review::intake_runtime::verify_request(
                            request
                                .context_intake
                                .as_ref()
                                .ok_or("context_intake required")?,
                            &saved,
                        )?;
                        Some(saved)
                    } else {
                        let candidate = review
                            .current_candidate_id
                            .as_ref()
                            .and_then(|id| review.candidates.iter().find(|c| &c.id == id))
                            .ok_or("No active candidate")?;
                        Some(
                            crate::review::intake_runtime::validate(
                                &app,
                                &review.owner_chat_id,
                                request
                                    .context_intake
                                    .as_ref()
                                    .ok_or("context_intake required before Tour authoring")?,
                                "before_tour",
                                &candidate.repository,
                                &candidate.base_oid,
                                &candidate.head_oid,
                                Some(&candidate.id),
                            )
                            .await?,
                        )
                    }
                } else {
                    None
                };
                let mut provenance = vec![];
                if let Operation::ProposeTourRevision {
                    target,
                    blocks,
                    ..
                } = &mutation.operation
                {
                    let (chat_id, mut sources) = {
                        let context = ccx.lock().await;
                        (context.chat_id.clone(), context.messages.clone())
                    };
                    if let Some(live) = app.chat.sessions.read().await.get(&chat_id).cloned() {
                        sources.extend(live.lock().await.messages.iter().cloned());
                    }
                    provenance =
                        crate::privacy::records::records_to_carry(&sources).map_err(|error| {
                            format!("Proposal source provenance unavailable: {error}")
                        })?;
                    let mut paths = crate::review::service::rich_paths(&root, &review, blocks);
                    // Resolve retained immutable context, not mutable eligibility.
                    let (version, _) =
                        refact_review_core::rich_blocks::resolve_tour_target(&review, target)
                            .map_err(|e| e.to_string())?;
                    paths.extend(crate::review::service::rich_paths(
                        &root,
                        &review,
                        &version.blocks,
                    ));
                    for anchor in &version.anchors {
                        paths.push(root.join(&anchor.position.path));
                        if let Some(candidate) = review
                            .candidates
                            .iter()
                            .find(|c| c.id == anchor.position.candidate_id)
                        {
                            paths.push(
                                PathBuf::from(&candidate.repository).join(&anchor.position.path),
                            );
                        }
                    }
                    provenance.extend(crate::privacy::records::declared_file_records(
                        &app.gcx, paths,
                    )?);
                }
                let proposing =
                    matches!(&mutation.operation, Operation::ProposeTourRevision { .. });
                let captured = if proposing {
                    crate::review::turn_completion::capture(
                        &app,
                        &review.owner_chat_id,
                        tool_call_id,
                    )
                    .await
                } else {
                    None
                };
                let changed = crate::review::service::model_mutate_with_intake(
                    root.clone(),
                    review.owner_chat_id.clone(),
                    review.id.clone(),
                    agent_id,
                    mutation,
                    provenance,
                    captured.as_ref().map(|(intent, _, _)| intent.clone()),
                    intake,
                )
                .await
                .map_err(|error| error.to_string())?;
                crate::review::chat_bridge::invalidate(&app, &changed).await;
                if let Some((intent, session, origin)) = captured {
                    if let Some(proposal) = changed.tour_revision_proposals.iter().find(|p| {
                        !review
                            .tour_revision_proposals
                            .iter()
                            .any(|old| old.id == p.id)
                    }) {
                        crate::review::turn_completion::register(
                            root.clone(),
                            &changed,
                            &intent,
                            &proposal.id,
                            session,
                            origin,
                        )
                        .await?;
                    }
                }
                if let Err(error) = crate::review::chat_bridge::flush(&app, root, &changed).await {
                    tracing::warn!(%error, "review contribution delivery pending");
                }
                result_message(&ccx, tool_call_id, &changed, summary(&changed)).await
            }
            ReviewToolKind::Reply => {
                let request: ReplyArgs = parse(args)?;
                let _ = request.review_id;
                let (app, agent_id) = {
                    let context = ccx.lock().await;
                    (
                        context.app.clone(),
                        context
                            .background_agent_id
                            .clone()
                            .unwrap_or_else(|| context.chat_id.clone()),
                    )
                };
                let request = crate::review::service::CommentRequest {
                    expected_revision: request.expected_revision,
                    request_key: request.request_key,
                    thread_id: Some(request.thread_id),
                    annotation_id: None,
                    anchor: None,
                    target: None,
                    blocks: request.blocks,
                    body: request.body,
                };
                let (chat_id, mut sources) = {
                    let context = ccx.lock().await;
                    (context.chat_id.clone(), context.messages.clone())
                };
                let live = app.chat.sessions.read().await.get(&chat_id).cloned();
                if let Some(live) = live {
                    sources.extend(live.lock().await.messages.iter().cloned());
                }
                let provenance = crate::privacy::records::records_to_carry(&sources)
                    .map_err(|error| format!("Reply source provenance is unavailable: {error}"))?;
                let changed = crate::review::service::reply(
                    app,
                    root,
                    review.owner_chat_id.clone(),
                    review.id.clone(),
                    agent_id,
                    request,
                    provenance,
                )
                .await
                .map_err(|error| error.to_string())?;
                result_message(&ccx, tool_call_id, &changed, summary(&changed)).await
            }
            ReviewToolKind::Context => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_schema_preserves_variants_and_canonical_operations_parse() {
        let desc = ToolReview(ReviewToolKind::Update).tool_description();
        let expected = desc.input_schema.clone();
        let wire = desc.into_openai_style(true);
        assert_eq!(wire["function"]["parameters"], expected);
        let samples = [
            json!({"op":"rename","title":"t"}),
            json!({"op":"set_target_reference","target_reference":"main"}),
            json!({"op":"put_tour_step","step":{"id":"t","current":{"version":1,"title":"t","body":"b","anchors":[]},"history":[]}}),
            json!({"op":"put_annotation","annotation":{"id":"a","kind":"finding","title":"t","body":"b","anchors":[],"severity":"low","confidence":"low","assessment":"unassessed","disposition":"open","applicability":"current","evidence_ids":[],"run_id":null}}),
            json!({"op":"put_thread","thread":{"id":"t","anchors":[],"annotation_id":null,"resolved":false,"contributions":[]}}),
            json!({"op":"add_evidence","evidence":{"id":"e","candidate_id":"c","material_id":null,"run_id":null,"source":{"kind":"source","path":"a","anchor":null},"summary":"s","applicability":"current","created_at_ms":-1}}),
            json!({"op":"add_evidence","evidence":{"id":"e","candidate_id":"c","material_id":null,"run_id":null,"source":{"kind":"external","url":"https://example.invalid"},"summary":"s","applicability":"current","created_at_ms":1}}),
        ];
        for operation in samples {
            let input = json!({"expected_revision":0,"request_key":"k","operation":operation});
            schema::validate(&expected, &input).unwrap();
            let _: UpdateArgs = serde_json::from_value(input).unwrap();
        }
        let context = ToolReview(ReviewToolKind::Context).tool_description();
        for kind in [
            ReviewToolKind::Context,
            ReviewToolKind::Get,
            ReviewToolKind::Anchor,
            ReviewToolKind::Update,
            ReviewToolKind::Reply,
            ReviewToolKind::PrepareExecution,
            ReviewToolKind::RecordEvidence,
        ] {
            let schema = ToolReview(kind).tool_description().input_schema;
            assert_eq!(schema["type"], "object");
            for unsupported in ["oneOf", "anyOf", "allOf"] {
                assert!(schema.get(unsupported).is_none());
            }
        }
        assert_eq!(
            context.clone().into_openai_style(true)["function"]["parameters"],
            context.input_schema
        );
    }

    #[test]
    fn source_evidence_schema_reserves_runtime_provenance() {
        let advertised = ToolReview(ReviewToolKind::Update)
            .tool_description()
            .input_schema;
        let evidence_schema = &advertised["properties"]["operation"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|variant| variant["properties"]["op"]["enum"][0] == "add_evidence")
            .unwrap()["properties"]["evidence"];
        for field in ["material_id", "run_id"] {
            assert_eq!(evidence_schema["properties"][field]["type"], "null");
            assert!(!evidence_schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!(field)));
        }
        for source in [
            json!({"kind":"source","path":"a","anchor":null}),
            json!({"kind":"external","url":"https://example.invalid"}),
        ] {
            let input = json!({"expected_revision":0,"request_key":"k","operation":{
                "op":"add_evidence","evidence":{"id":"e","candidate_id":"c",
                "source":source,"summary":"s","applicability":"current","created_at_ms":0}
            }});
            for explicit_nulls in [false, true] {
                let mut valid = input.clone();
                if explicit_nulls {
                    valid["operation"]["evidence"]["material_id"] = Value::Null;
                    valid["operation"]["evidence"]["run_id"] = Value::Null;
                }
                // This is the same validator and advertised schema used by tool_execute.
                schema::validate(&advertised, &valid).unwrap();
                let _: UpdateArgs = serde_json::from_value(valid.clone()).unwrap();
                for field in ["material_id", "run_id"] {
                    let mut forged = valid.clone();
                    forged["operation"]["evidence"][field] = json!("claimed-provenance");
                    let error = schema::validate(&advertised, &forged).unwrap_err();
                    assert!(
                        error.contains(&format!("$.operation.evidence.{field}: expected null")),
                        "{error}"
                    );
                    assert!(!error.contains("claimed-provenance"));
                }
            }
        }
    }

    #[tokio::test]
    async fn execute_aggregates_schema_errors_before_binding() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let context = AtCommandsContext::new_with_abort(
            app,
            4096,
            10,
            false,
            vec![],
            "synthetic".into(),
            None,
            String::new(),
            None,
            None,
            None,
        )
        .await;
        let ccx = Arc::new(Mutex::new(context));
        let args: HashMap<String, Value> = serde_json::from_value(json!({"expected_revision":"SECRET", "operation":{"op":"put_annotation", "annotation":{"kind":"invalid", "body":42}}})).unwrap();
        let error = ToolReview(ReviewToolKind::Update)
            .tool_execute(ccx.clone(), &"call".into(), &args)
            .await
            .err()
            .unwrap();
        assert!(
            error.contains("$.request_key")
                && error.contains("$.expected_revision")
                && error.contains("$.operation.annotation.id")
                && error.contains("$.operation.annotation.kind")
                && error.contains("$.operation.annotation.body"),
            "{error}"
        );
        assert!(!error.contains("SECRET"));
        let forbidden: HashMap<String, Value> = serde_json::from_value(json!({"request_key":"k","expected_revision":0,"operation":{"op":"activate_candidate","candidate_id":"x"}})).unwrap();
        let error = ToolReview(ReviewToolKind::Update)
            .tool_execute(ccx, &"call".into(), &forbidden)
            .await
            .err()
            .unwrap();
        assert!(error.contains("$.operation.op"));
    }

    #[test]
    fn summary_omits_large_provenance_without_losing_current_identity() {
        let labels = (0..2187)
            .map(|i| (format!("secret-label-{i}"), json!("private")))
            .collect::<serde_json::Map<_, _>>();
        let privacy = (0..2187).map(|i| json!({"path":format!("private-path-{i}"),"zone":"private","attribution":"declared"})).collect::<Vec<_>>();
        let position = json!({"candidate_id":"candidate-1","path":"src/main.rs","side":"head","start_line":3,"end_line":4,"content_hash":"hash","context_before":[],"context_after":[]});
        let anchor = json!({"position":position,"status":"current","stalled_reason":null,"history":[position]});
        let review: Review = serde_json::from_value(json!({
            "schema_version":1,"id":"review-1","owner_chat_id":"chat-1","revision":7,"current_candidate_id":"candidate-1","title":"title","origin":"local","target_reference":"main","candidates":[],
            "tour":[{"id":"tour-1","current":{"version":2,"title":"step","body":"body","anchors":[anchor]},"history":[{"version":1,"title":"old","body":"old body","anchors":[anchor]}]}],
            "annotations":[],"threads":[{"id":"thread-1","anchors":[anchor],"annotation_id":null,"resolved":false,"contributions":[{"id":"contribution-1","thread_id":"thread-1","actor":{"kind":"agent","id":"agent-1"},"body":"hello","created_at_ms":12,"chat_message_id":null,"context":{"candidate_id":"candidate-1","run_id":"run-1","applicability":"current","privacy":privacy}}]}],
            "evidence":[],"runs":[],"materials":[{"id":"material-1","kind":"investigation","candidate_id":"candidate-1","agent_id":null,"description":"material","content_digest":"digest","input_digests":{"derived_origin_labels":Value::Object(labels).to_string(),"tree":"tree-digest"},"environment":null,"worktree_id":null,"worktree_root":null,"artifacts":[]}],"activity":[]
        })).unwrap();
        let raw = serde_json::to_string(&review).unwrap();
        let projected = summary(&review);
        let text = projected.to_string();
        assert!(!text.contains("secret-label-"));
        assert!(!text.contains("private-path-"));
        assert!(text.len() * 10 < raw.len());
        assert_eq!(
            projected["materials"][0]["input_digests"]["tree"],
            "tree-digest"
        );
        assert!(projected["materials"][0]["input_digests"]
            .get("derived_origin_labels")
            .is_none());
        assert_eq!(
            projected["materials"][0]["derived_origin_labels_omitted"],
            true
        );
        assert_eq!(
            projected["threads"][0]["contributions"][0]["id"],
            "contribution-1"
        );
        assert_eq!(
            projected["threads"][0]["contributions"][0]["context"]["privacy_total"],
            2187
        );
        assert_eq!(
            projected["tour"][0]["current"]["anchors"][0]["position"],
            position
        );
        assert_eq!(
            projected["tour"][0]["current"]["anchors"][0]["history_total"],
            1
        );
        assert!(
            review.materials[0].input_digests["derived_origin_labels"].contains("secret-label-")
        );
        assert_eq!(
            review.threads[0].contributions[0]
                .context
                .as_ref()
                .unwrap()
                .privacy
                .len(),
            2187
        );
        let tour = exact_object(&review, "tour_step", "tour-1", None).unwrap();
        assert_eq!(tour, serde_json::to_value(&review.tour[0]).unwrap());
        assert_eq!(tour["history"][0]["anchors"][0]["history"][0], position);
        let thread = exact_object(&review, "thread", "thread-1", None).unwrap();
        assert_eq!(thread, serde_json::to_value(&review.threads[0]).unwrap());
        assert_eq!(
            thread["contributions"][0]["context"]["privacy"]
                .as_array()
                .unwrap()
                .len(),
            2187
        );
        assert_eq!(
            exact_object(&review, "thread", "thread-1", Some("candidate-1")).unwrap(),
            thread
        );
        assert!(exact_object(&review, "thread", "thread-1", Some("foreign")).is_err());
        assert!(exact_object(&review, "tour_step", "tour-1", Some("foreign")).is_err());
        let mut mixed = review.clone();
        mixed.tour[0].history[0].anchors[0].history[0].candidate_id = "foreign".into();
        assert!(exact_object(&mixed, "tour_step", "tour-1", Some("candidate-1")).is_err());
        assert!(exact_object(&mixed, "tour_step", "tour-1", None).is_ok());
        let mut explicit = review.clone();
        explicit.tour[0].current.candidate_id = Some("candidate-2".into());
        explicit.tour[0].current.anchors.clear();
        explicit.tour[0].current.body = "FOREIGN_CANDIDATE_BODY".into();
        assert!(exact_object(&explicit, "tour_step", "tour-1", Some("candidate-1")).is_err());
        let mut foreign_thread = explicit.threads[0].clone();
        foreign_thread.id = "foreign-thread".into();
        foreign_thread.contributions[0].body = "FOREIGN_CONTRIBUTION_BODY".into();
        foreign_thread.contributions[0]
            .context
            .as_mut()
            .unwrap()
            .candidate_id = Some("candidate-2".into());
        explicit.threads.push(foreign_thread);
        let child = pinned_summary(&explicit, "candidate-1");
        let child_text = child.to_string();
        assert!(!child_text.contains("FOREIGN_CANDIDATE_BODY"));
        assert!(!child_text.contains("FOREIGN_CONTRIBUTION_BODY"));
        assert_eq!(child["projection"], "pinned_summary");
        assert_eq!(child["inspected_candidate_id"], "candidate-1");
        assert_eq!(child["threads"][0]["contributions"][0]["body"], "hello");
        let mut retained = review.clone();
        retained.tour[0].current.candidate_id = Some("candidate-1".into());
        retained.tour[0].current.blocks = vec![refact_review_core::RichBlock {
            id: "retained-code".into(),
            revision: 1,
            content: refact_review_core::RichBlockContent::Code {
                anchor: retained.tour[0].current.anchors[0].clone(),
            },
        }];
        if let refact_review_core::RichBlockContent::Code {
            anchor,
        } = &mut retained.tour[0].current.blocks[0].content
        {
            anchor.history[0].candidate_id = "candidate-2".into();
        }
        assert!(exact_object(&retained, "tour_step", "tour-1", Some("candidate-1")).is_err());
        retained.threads[0].target = Some(refact_review_core::TourDiscussionTarget {
            candidate_id: "candidate-1".into(),
            step_id: "tour-1".into(),
            version: 2,
            block_id: None,
            block_revision: None,
        });
        retained.threads[0].anchors[0].history[0].candidate_id = "candidate-2".into();
        assert!(exact_object(&retained, "thread", "thread-1", Some("candidate-1")).is_err());
    }
}
