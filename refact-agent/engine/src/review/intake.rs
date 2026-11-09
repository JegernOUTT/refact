use std::collections::HashSet;

use refact_chat_api::{PendingQuestionSet, PendingQuestionStatus, QuestionResponseValue};
use refact_review_core::{
    ContextConsultation, ContextIntakeGap, ContextInputReference, ContextIntakeReceipt,
};
use serde::{Deserialize, Serialize};
use crate::call_validation::ChatMessage;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextIntakeRequest {
    pub user_message_id: Option<String>,
    pub question_tool_call_id: Option<String>,
    pub retrieval_tool_call_ids: Vec<String>,
    pub gaps: Vec<ContextIntakeGap>,
    pub summary: String,
}

const CATEGORIES: &[&str] = &[
    "docs",
    "mcp",
    "web",
    "messenger",
    "knowledge",
    "trajectories",
    "tasks",
    "memory",
];

pub fn expected_question(
    phase: &str,
    repository: &str,
    base_oid: &str,
    head_oid: &str,
    candidate_id: Option<&str>,
) -> refact_chat_api::PendingQuestion {
    refact_chat_api::PendingQuestion {
        id: format!("review-context-{phase}"),
        question_type: "free_text".into(),
        text: format!("For {phase} of repository {repository}, base {base_oid}, head {head_oid}, candidate {}: what relevant intent, constraints and sources should be considered? Consider docs, MCP, web, messenger, knowledge, trajectories, tasks and memory; identify unavailable or declined sources.", candidate_id.unwrap_or("not yet captured")),
        options: None,
    }
}

fn categories(name: &str) -> &'static [&'static str] {
    match name {
        "cat" | "tree" | "glob" | "search_pattern" | "search_symbol_definition" | "review_get" => {
            &["docs"]
        }
        "web" | "web_search" | "chrome" => &["web"],
        "knowledge" => &["knowledge"],
        "search_trajectories" | "get_trajectory_context" => &["trajectories"],
        "work_overview" | "work_inspect" | "board_discover" => &["tasks"],
        _ => &[],
    }
}

struct Retrieval<'a> {
    index: usize,
    result: &'a ChatMessage,
    invocation: &'a ChatMessage,
    call_id: &'a str,
    name: &'a str,
    arguments: &'a str,
    cats: &'static [&'static str],
}

fn resolve_retrieval<'a>(messages: &'a [ChatMessage], id: &str) -> Result<Retrieval<'a>, String> {
    let (index, result) = messages
        .iter()
        .enumerate()
        .find(|(_, m)| {
            (m.role == "tool" && m.tool_call_id == id && !m.extra.contains_key("tool_job"))
                || (m.role == "event"
                    && m.extra
                        .get("tool_job_result")
                        .is_some_and(|v| v["tool_call_id"].as_str() == Some(id)))
        })
        .ok_or("Retrieval result not found")?;
    if result.message_id.is_empty() {
        return Err("Retrieval result has no durable identity".into());
    }
    if result.tool_failed != Some(false) {
        return Err("Retrieval did not complete successfully".into());
    }
    let (call_id, event_name) = if result.role == "tool" && !result.extra.contains_key("tool_job") {
        (result.tool_call_id.as_str(), None)
    } else if result.role == "event" {
        let v = result
            .extra
            .get("tool_job_result")
            .ok_or("Not a completed tool result")?;
        if v["actor"] != "tool_completion" || v["original_role"] != "tool" {
            return Err("Not a completed tool result".into());
        }
        (
            v["tool_call_id"].as_str().ok_or("Missing call identity")?,
            v["tool_name"].as_str(),
        )
    } else {
        return Err("Not a completed tool result".into());
    };
    let (call_message, call) = messages[..index]
        .iter()
        .filter(|m| m.role == "assistant")
        .flat_map(|m| m.tool_calls.iter().flatten().map(move |c| (m, c)))
        .find(|(_, c)| !call_id.is_empty() && c.id == call_id)
        .ok_or("Retrieval invocation not found")?;
    let name = call.function.name.as_str();
    if event_name.is_some_and(|n| n != name) {
        return Err("Retrieval tool identity mismatch".into());
    }
    let cats = categories(name);
    if cats.is_empty() {
        return Err("Unsupported retrieval tool; record an explicit gap instead".into());
    }
    Ok(Retrieval {
        index,
        result,
        invocation: call_message,
        call_id,
        name,
        arguments: &call.function.arguments,
        cats,
    })
}

fn consultation(source: &Retrieval<'_>) -> ContextConsultation {
    ContextConsultation {
        message_id: source.result.message_id.clone(),
        tool_call_id: source.call_id.into(),
        tool_name: source.name.into(),
        categories: source.cats.iter().map(|s| (*s).into()).collect(),
        result_hash: refact_review_core::content_hash(
            source.result.content.content_text_only().as_bytes(),
        ),
        observed_at_ms: None,
    }
}

pub(crate) fn context_sources(
    messages: &[ChatMessage],
    after: usize,
) -> Result<
    (
        Vec<ContextConsultation>,
        Vec<refact_privacy::FileRecord>,
        bool,
    ),
    String,
> {
    let mut refs = Vec::new();
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    let mut truncated = false;
    for (index, message) in messages
        .iter()
        .enumerate()
        .rev()
        .take_while(|(i, _)| *i > after)
    {
        let id = if message.role == "tool" {
            Some(message.tool_call_id.as_str())
        } else if message.role == "event" {
            message
                .extra
                .get("tool_job_result")
                .and_then(|v| v["tool_call_id"].as_str())
        } else {
            None
        };
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            continue;
        };
        let Ok(source) = resolve_retrieval(messages, id) else {
            continue;
        };
        if source.index != index
            || !messages[after + 1..index]
                .iter()
                .any(|m| std::ptr::eq(m, source.invocation))
            || !seen.insert(id)
        {
            continue;
        }
        if refs.len() == 64 {
            truncated = true;
            break;
        }
        refs.push(consultation(&source));
        sources.push(source.invocation.clone());
        sources.push(source.result.clone());
    }
    let privacy = crate::privacy::records::records_to_carry(&sources)
        .map_err(|_| "Invalid referenced privacy records")?;
    Ok((refs, privacy, truncated))
}

pub fn validate(
    request: &ContextIntakeRequest,
    phase: &str,
    owner_chat_id: &str,
    repository: &str,
    base_oid: &str,
    head_oid: &str,
    candidate_id: Option<&str>,
    activation_index: Option<usize>,
    messages: &[ChatMessage],
    questions: &[PendingQuestionSet],
    captured_at_ms: i64,
) -> Result<ContextIntakeReceipt, String> {
    if !["before_review", "before_tour"].contains(&phase)
        || owner_chat_id.is_empty()
        || repository.is_empty()
        || base_oid.is_empty()
        || head_oid.is_empty()
        || request.summary.len() > 4000
        || request.retrieval_tool_call_ids.len() > 64
        || request.gaps.len() > 32
    {
        return Err("Invalid context intake bounds or identity".into());
    }
    let mut sources = Vec::new();
    let input_reference = match (&request.user_message_id, &request.question_tool_call_id) {
        (Some(id), None) => {
            let m = messages
                .iter()
                .rev()
                .find(|m| {
                    !m.message_id.is_empty()
                        && (id == "latest" || &m.message_id == id)
                        && m.role == "user"
                        && !m.extra.contains_key("review")
                })
                .ok_or("Context intake requires an ordinary owning-chat user message")?;
            sources.push(m.clone());
            ContextInputReference {
                kind: "user_directive".into(),
                message_id: Some(m.message_id.clone()),
                tool_call_id: None,
                status: "cited_user_intent".into(),
                content_hash: refact_review_core::content_hash(
                    m.content.content_text_only().as_bytes(),
                ),
            }
        }
        (None, Some(id)) => {
            let set = questions
                .iter()
                .find(|q| {
                    &q.tool_call_id == id
                        && q.status == PendingQuestionStatus::Answered
                        && q.resolution_id.as_deref().is_some_and(|s| !s.is_empty())
                        && !q.questions.is_empty()
                })
                .ok_or("Context intake requires a completed question response")?;
            if set.questions
                != vec![expected_question(
                    phase,
                    repository,
                    base_oid,
                    head_oid,
                    candidate_id,
                )]
            {
                return Err("Question does not match this phase and identity".into());
            }
            let response = set
                .response
                .as_ref()
                .ok_or("Structured question response required")?;
            if response.answers.len() != set.questions.len()
                || set
                    .questions
                    .iter()
                    .any(|q| !response.answers.iter().any(|a| a.id == q.id))
            {
                return Err("Incomplete question response".into());
            }
            let call = messages
                .iter()
                .find(|m| {
                    m.role == "assistant"
                        && m.tool_calls.as_ref().is_some_and(|calls| {
                            calls
                                .iter()
                                .any(|c| c.id == *id && c.function.name == "ask_questions")
                        })
                })
                .ok_or("Question invocation not found")?;
            if call.message_id.is_empty() {
                return Err("Question invocation has no durable identity".into());
            }
            sources.push(call.clone());
            if let Some(resolution) = set.resolution_id.as_deref() {
                sources.extend(
                    messages
                        .iter()
                        .filter(|message| {
                            message.role == "user"
                                && message
                                    .extra
                                    .get("client_message_id")
                                    .and_then(serde_json::Value::as_str)
                                    == Some(resolution)
                        })
                        .cloned(),
                );
            }
            let status = if response
                .answers
                .iter()
                .any(|a| matches!(a.value, QuestionResponseValue::Skipped))
            {
                "answered_with_skips"
            } else {
                "answered"
            };
            let bytes = serde_json::to_vec(set).map_err(|_| "Invalid question response")?;
            ContextInputReference {
                kind: "question".into(),
                message_id: Some(call.message_id.clone()),
                tool_call_id: Some(id.clone()),
                status: status.into(),
                content_hash: refact_review_core::content_hash(&bytes),
            }
        }
        _ => return Err("Cite exactly one user message or question response".into()),
    };
    let mut seen = HashSet::new();
    let mut covered = HashSet::new();
    let mut consultations = Vec::new();
    let mut fresh_candidate = false;
    for id in &request.retrieval_tool_call_ids {
        if id.is_empty() || !seen.insert(id) {
            return Err("Duplicate or empty retrieval reference".into());
        }
        let source = resolve_retrieval(messages, id)?;
        let Retrieval {
            index,
            result,
            invocation: call_message,
            name,
            arguments,
            cats,
            ..
        } = &source;
        for cat in *cats {
            covered.insert(*cat);
        }
        if phase == "before_tour"
            && *name == "review_get"
            && activation_index.is_some_and(|start| *index > start)
        {
            let args: serde_json::Value =
                serde_json::from_str(arguments).map_err(|_| "Invalid retrieval arguments")?;
            let payload: serde_json::Value =
                serde_json::from_str(&result.content.content_text_only())
                    .map_err(|_| "Invalid candidate source result")?;
            if args
                .get("path")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
                && args
                    .get("candidate_id")
                    .and_then(|v| v.as_str())
                    .is_none_or(|id| Some(id) == candidate_id)
                && payload.get("candidate_id").and_then(|v| v.as_str()) == candidate_id
                && payload.get("path").and_then(|v| v.as_str())
                    == args.get("path").and_then(|v| v.as_str())
                && payload.get("lines").is_some_and(|v| v.is_array())
                && candidate_id.is_some()
            {
                fresh_candidate = true;
            }
        }
        sources.push((*call_message).clone());
        sources.push((*result).clone());
        consultations.push(consultation(&source));
    }
    for gap in &request.gaps {
        if !CATEGORIES.contains(&gap.category.as_str())
            || !["unavailable", "declined", "unknown"].contains(&gap.status.as_str())
            || gap.reason.trim().is_empty()
            || gap.reason.len() > 1000
        {
            return Err("Invalid context gap".into());
        }
        covered.insert(gap.category.as_str());
    }
    if CATEGORIES.iter().any(|c| !covered.contains(c)) {
        return Err("Every source category requires a consultation or explicit gap".into());
    }
    if phase == "before_tour" && !fresh_candidate {
        return Err("A fresh candidate source inspection is required before Tour authoring".into());
    }
    let privacy = crate::privacy::records::records_to_carry(&sources)
        .map_err(|_| "Invalid referenced privacy records")?;
    let id = refact_review_core::content_hash(
        &serde_json::to_vec(&(
            phase,
            owner_chat_id,
            repository,
            base_oid,
            head_oid,
            candidate_id,
            &input_reference,
            &consultations,
            &request.gaps,
            &request.summary,
        ))
        .map_err(|_| "Invalid intake")?,
    );
    Ok(ContextIntakeReceipt {
        id,
        phase: phase.into(),
        owner_chat_id: owner_chat_id.into(),
        repository: repository.into(),
        base_oid: base_oid.into(),
        head_oid: head_oid.into(),
        candidate_id: candidate_id.map(str::to_owned),
        captured_at_ms,
        input_reference,
        consultations,
        gaps: request.gaps.clone(),
        summary: request.summary.clone(),
        privacy,
    })
}

#[cfg(test)]
#[path = "intake_tests.rs"]
mod tests;
