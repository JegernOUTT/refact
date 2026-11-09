use crate::*;

pub fn validate_context_intake(receipt: &ContextIntakeReceipt) -> Result<()> {
    let bounded = |s: &str, max: usize| !s.trim().is_empty() && s.len() <= max && !s.contains('\0');
    let hash = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if !bounded(&receipt.id, 256)
        || !matches!(receipt.phase.as_str(), "before_review" | "before_tour")
        || !bounded(&receipt.owner_chat_id, 256)
        || !bounded(&receipt.repository, 4096)
        || !bounded(&receipt.base_oid, 128)
        || !bounded(&receipt.head_oid, 128)
        || receipt
            .candidate_id
            .as_ref()
            .is_some_and(|s| !bounded(s, 256))
        || receipt.captured_at_ms <= 0
        || !bounded(&receipt.summary, 16_384)
        || receipt.consultations.len() > 64
        || receipt.gaps.len() > 64
        || receipt.privacy.len() > MAX_FILES
        || serde_json::to_vec(receipt)?.len() > MAX_BLOB_BYTES
    {
        return Err(invalid("invalid context intake receipt"));
    }
    let input = &receipt.input_reference;
    if !matches!(input.kind.as_str(), "user_directive" | "question")
        || !matches!(
            input.status.as_str(),
            "cited_user_intent" | "answered" | "answered_with_skips"
        )
        || !hash(&input.content_hash)
        || input.message_id.as_ref().is_some_and(|s| !bounded(s, 256))
        || (input.kind == "user_directive" && input.message_id.is_none())
        || (input.kind == "user_directive"
            && (input.status != "cited_user_intent" || input.tool_call_id.is_some()))
        || (input.kind == "question" && input.status == "cited_user_intent")
        || input
            .tool_call_id
            .as_ref()
            .is_some_and(|s| !bounded(s, 256))
        || (input.kind == "question" && input.tool_call_id.is_none())
    {
        return Err(invalid("invalid context input reference"));
    }
    for consultation in &receipt.consultations {
        if !bounded(&consultation.message_id, 256)
            || !bounded(&consultation.tool_call_id, 256)
            || !bounded(&consultation.tool_name, 256)
            || consultation.categories.is_empty()
            || consultation.categories.len() > 8
            || consultation.categories.iter().any(|s| !category(s))
            || !hash(&consultation.result_hash)
            || consultation
                .observed_at_ms
                .is_some_and(|t| t <= 0 || t > receipt.captured_at_ms)
        {
            return Err(invalid("invalid context consultation"));
        }
    }
    for gap in &receipt.gaps {
        if !category(&gap.category)
            || !matches!(gap.status.as_str(), "unavailable" | "declined" | "unknown")
            || !bounded(&gap.reason, 4096)
        {
            return Err(invalid("invalid context gap"));
        }
    }
    Ok(())
}
fn category(s: &str) -> bool {
    matches!(
        s,
        "docs" | "mcp" | "web" | "messenger" | "knowledge" | "trajectories" | "tasks" | "memory"
    )
}
pub(crate) fn intake_semantic(receipt: &ContextIntakeReceipt) -> ContextIntakeReceipt {
    let mut receipt = receipt.clone();
    receipt.captured_at_ms = 0;
    receipt
}
pub(crate) fn bind_intake(
    receipt: &ContextIntakeReceipt,
    phase: &str,
    owner: &str,
    candidate: Option<&Candidate>,
) -> Result<()> {
    validate_context_intake(receipt)?;
    if receipt.phase != phase || receipt.owner_chat_id != owner {
        return Err(invalid("context intake identity mismatch"));
    }
    if let Some(candidate) = candidate {
        if receipt.repository != candidate.repository
            || receipt.base_oid != candidate.base_oid
            || receipt.head_oid != candidate.head_oid
            || (phase == "before_tour"
                && receipt.candidate_id.as_deref() != Some(candidate.id.as_str()))
            || (phase == "before_review" && receipt.candidate_id.is_some())
        {
            return Err(invalid("context intake candidate mismatch"));
        }
    } else if phase == "before_tour" || receipt.candidate_id.is_some() {
        return Err(invalid("context intake candidate required or premature"));
    }
    Ok(())
}
