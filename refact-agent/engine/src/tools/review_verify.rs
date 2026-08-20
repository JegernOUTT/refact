use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex as AMutex;

use crate::call_validation::ChatMessage;
use crate::chat::types::TaskMeta;
use crate::global_context::GlobalContext;
use crate::subchat::{run_subchat_once_with_explicit_params, ExplicitSubchatSpec, TraceParent};
use crate::tools::review_types::{ReviewEvidence, ReviewFinding, ReviewSeverity, VerificationStatus};
use crate::tools::subagent_phases::get_last_assistant_content;
use crate::worktrees::types::WorktreeMeta;

const MAX_BATCH_SIZE: usize = 10;
const MAX_VERIFIER_CALLS: usize = 3;
const VERIFIER_PARSE_FAILED: &str = "verifier_parse_failed";
const VERIFIER_CALL_FAILED: &str = "verifier_call_failed";
const VERIFIER_BUDGET_EXHAUSTED: &str = "verifier_budget_exhausted";
const VERIFIER_INCOMPLETE: &str = "verifier_incomplete";
pub(crate) const VERIFIER_ATTRIBUTION_ID: &str = "review_verifier";

const VERIFIER_RETRY_PROMPT: &str = r#"Your response was malformed. Return exactly one fenced json block and no other text:

```json
{"verdicts":[{"index":0,"status":"verified|downgraded|rejected|needs_human_validation","confidence":0.0,"note":"<=200 chars"}]}
```

Return exactly one verdict for each candidate index from the original evidence bundle."#;

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BlindCandidate<'a> {
    index: usize,
    #[serde(rename = "id-index")]
    id_index: String,
    #[serde(rename = "file:lines")]
    file_lines: String,
    category: &'a str,
    severity: &'static str,
    claim: &'a str,
    evidence: &'a [ReviewEvidence],
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VerifierBatch {
    pub indices: Vec<usize>,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VerifierVerdictStatus {
    Verified,
    Downgraded,
    Rejected,
    NeedsHumanValidation,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VerifierVerdict {
    pub index: usize,
    pub status: VerifierVerdictStatus,
    pub confidence: f32,
    pub note: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerdictEnvelope {
    verdicts: Vec<RawVerifierVerdict>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVerifierVerdict {
    index: usize,
    status: VerifierVerdictStatus,
    confidence: f32,
    note: String,
}

#[derive(Default)]
struct VerifierCallBudget {
    used: usize,
}

impl VerifierCallBudget {
    fn reserve(&mut self) -> bool {
        if self.used >= MAX_VERIFIER_CALLS {
            return false;
        }
        self.used += 1;
        true
    }
}

#[derive(Clone)]
pub(crate) struct VerifierRunner {
    pub spec: ExplicitSubchatSpec,
    pub system_prompt: String,
}

#[derive(Clone)]
pub(crate) struct VerifyParentCtx {
    pub tool_call_id: String,
    pub subchat_tx: Arc<AMutex<UnboundedSender<Value>>>,
    pub abort_flag: Arc<AtomicBool>,
    pub depth: usize,
    pub task_meta: Option<TaskMeta>,
    pub worktree: Option<WorktreeMeta>,
    pub chat_id: String,
    pub root_chat_id: String,
}

fn severity_label(severity: &ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::Low => "low",
        ReviewSeverity::Medium => "medium",
        ReviewSeverity::High => "high",
        ReviewSeverity::Critical => "critical",
    }
}

fn should_verify(finding: &ReviewFinding, total_candidates: usize) -> bool {
    matches!(
        finding.severity,
        ReviewSeverity::High | ReviewSeverity::Critical
    ) || total_candidates <= 5
        || finding.confidence < 0.7
}

pub(crate) fn build_verifier_batches(findings: &[ReviewFinding]) -> VecDeque<VerifierBatch> {
    let selected = findings
        .iter()
        .enumerate()
        .filter(|(_, finding)| should_verify(finding, findings.len()))
        .collect::<Vec<_>>();
    selected
        .chunks(MAX_BATCH_SIZE)
        .map(|chunk| {
            let candidates = chunk
                .iter()
                .map(|(index, finding)| BlindCandidate {
                    index: *index,
                    id_index: if finding.id.is_empty() {
                        index.to_string()
                    } else {
                        format!("{}-{index}", finding.id)
                    },
                    file_lines: if finding.line1 == finding.line2 {
                        format!("{}:{}", finding.file, finding.line1)
                    } else {
                        format!("{}:{}-{}", finding.file, finding.line1, finding.line2)
                    },
                    category: &finding.category,
                    severity: severity_label(&finding.severity),
                    claim: &finding.claim,
                    evidence: &finding.evidence,
                })
                .collect::<Vec<_>>();
            VerifierBatch {
                indices: chunk.iter().map(|(index, _)| *index).collect(),
                prompt: serde_json::to_string(&candidates)
                    .expect("blind review candidate serialization cannot fail"),
            }
        })
        .collect()
}

fn extract_exact_json_block(text: &str) -> Result<&str, String> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix("```json")
        .ok_or_else(|| "missing opening fenced json block".to_string())?;
    let end = body
        .find("```")
        .ok_or_else(|| "missing closing fenced json block".to_string())?;
    if !body[end + 3..].trim().is_empty() {
        return Err("content after fenced json block".to_string());
    }
    if body[end + 3..].contains("```json") {
        return Err("multiple fenced json blocks".to_string());
    }
    Ok(body[..end].trim())
}

pub(crate) fn parse_verifier_verdicts(
    text: &str,
    allowed_indices: &[usize],
) -> Result<Vec<VerifierVerdict>, String> {
    let json = extract_exact_json_block(text)?;
    let envelope: VerdictEnvelope =
        serde_json::from_str(json).map_err(|error| format!("invalid verdict json: {error}"))?;
    let allowed = allowed_indices.iter().copied().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut verdicts = Vec::new();
    for verdict in envelope.verdicts {
        if !allowed.contains(&verdict.index) {
            tracing::warn!(
                "review verifier ignored verdict for unknown candidate index {}",
                verdict.index
            );
            continue;
        }
        if !seen.insert(verdict.index) {
            return Err(format!(
                "duplicate verdict for candidate index {}",
                verdict.index
            ));
        }
        if !verdict.confidence.is_finite() || !(0.0..=1.0).contains(&verdict.confidence) {
            return Err(format!(
                "verdict confidence for candidate index {} is outside 0..=1",
                verdict.index
            ));
        }
        if verdict.note.chars().count() > 200 {
            return Err(format!(
                "verdict note for candidate index {} exceeds 200 characters",
                verdict.index
            ));
        }
        verdicts.push(VerifierVerdict {
            index: verdict.index,
            status: verdict.status,
            confidence: verdict.confidence,
            note: verdict.note,
        });
    }
    Ok(verdicts)
}

fn downgrade_severity(severity: &ReviewSeverity) -> ReviewSeverity {
    match severity {
        ReviewSeverity::Critical => ReviewSeverity::High,
        ReviewSeverity::High => ReviewSeverity::Medium,
        ReviewSeverity::Medium | ReviewSeverity::Low => ReviewSeverity::Low,
    }
}

fn missing_verdict_indices(allowed_indices: &[usize], verdicts: &[VerifierVerdict]) -> Vec<usize> {
    let covered = verdicts
        .iter()
        .map(|verdict| verdict.index)
        .collect::<HashSet<_>>();
    allowed_indices
        .iter()
        .copied()
        .filter(|index| !covered.contains(index))
        .collect()
}

pub(crate) fn apply_verifier_verdicts(
    findings: &mut Vec<ReviewFinding>,
    checks: &mut Vec<String>,
    verdicts: Vec<VerifierVerdict>,
    incomplete_indices: impl IntoIterator<Item = usize>,
) {
    let verdicts = verdicts
        .into_iter()
        .map(|verdict| (verdict.index, verdict))
        .collect::<HashMap<_, _>>();
    let incomplete_indices = incomplete_indices.into_iter().collect::<HashSet<_>>();
    let mut rejected = 0;
    let mut incomplete = 0;
    *findings = std::mem::take(findings)
        .into_iter()
        .enumerate()
        .filter_map(|(index, mut finding)| {
            let Some(verdict) = verdicts.get(&index) else {
                if incomplete_indices.contains(&index) {
                    incomplete += 1;
                    finding.verification_status = VerificationStatus::NeedsHumanValidation;
                    finding
                        .checks_performed
                        .push(VERIFIER_INCOMPLETE.to_string());
                }
                return Some(finding);
            };
            tracing::debug!("review verifier candidate {}: {}", index, verdict.note);
            match verdict.status {
                VerifierVerdictStatus::Verified => {
                    finding.verification_status = VerificationStatus::Verified;
                    finding.confidence = finding.confidence.min(verdict.confidence);
                    Some(finding)
                }
                VerifierVerdictStatus::Downgraded => {
                    finding.verification_status = VerificationStatus::Downgraded;
                    finding.severity = downgrade_severity(&finding.severity);
                    Some(finding)
                }
                VerifierVerdictStatus::Rejected => {
                    rejected += 1;
                    None
                }
                VerifierVerdictStatus::NeedsHumanValidation => {
                    finding.verification_status = VerificationStatus::NeedsHumanValidation;
                    Some(finding)
                }
            }
        })
        .collect();
    if rejected > 0 {
        checks.push(format!("verifier_rejected:{rejected}"));
    }
    if incomplete > 0 {
        checks.push(format!("{VERIFIER_INCOMPLETE}:{incomplete}"));
    }
}

pub(crate) fn verification_status_label(status: &VerificationStatus) -> &'static str {
    match status {
        VerificationStatus::Unverified => "unverified",
        VerificationStatus::Verified => "verified",
        VerificationStatus::Downgraded => "downgraded",
        VerificationStatus::Rejected => "rejected",
        VerificationStatus::NeedsHumanValidation => "needs human validation",
    }
}

fn mark_candidates(
    findings: &mut [ReviewFinding],
    checks: &mut Vec<String>,
    indices: impl IntoIterator<Item = usize>,
    status: VerificationStatus,
    marker: &str,
) {
    let indices = indices.into_iter().collect::<HashSet<_>>();
    if indices.is_empty() {
        return;
    }
    for (index, finding) in findings.iter_mut().enumerate() {
        if indices.contains(&index) {
            finding.verification_status = status.clone();
            if !finding.checks_performed.iter().any(|check| check == marker) {
                finding.checks_performed.push(marker.to_string());
            }
        }
    }
    if !checks.iter().any(|check| check == marker) {
        checks.push(marker.to_string());
    }
}

pub(crate) fn merge_metering(
    target: &mut serde_json::Map<String, Value>,
    source: serde_json::Map<String, Value>,
) {
    for (key, incoming) in source {
        match (target.get_mut(&key), incoming) {
            (Some(Value::Object(existing)), Value::Object(incoming)) => {
                merge_metering(existing, incoming);
            }
            (Some(Value::Number(existing)), Value::Number(incoming)) => {
                if let (Some(left), Some(right)) = (existing.as_u64(), incoming.as_u64()) {
                    if let Some(sum) = left.checked_add(right) {
                        *existing = sum.into();
                    }
                } else if let (Some(left), Some(right)) = (existing.as_i64(), incoming.as_i64()) {
                    if let Some(sum) = left.checked_add(right) {
                        *existing = sum.into();
                    }
                } else if let Some(sum) = serde_json::Number::from_f64(
                    existing.as_f64().unwrap_or(0.0) + incoming.as_f64().unwrap_or(0.0),
                ) {
                    *existing = sum;
                }
            }
            (_, incoming) => {
                target.insert(key, incoming);
            }
        }
    }
}

pub(crate) async fn verify_findings(
    gcx: Arc<GlobalContext>,
    parent: &VerifyParentCtx,
    runner: &VerifierRunner,
    findings: &mut Vec<ReviewFinding>,
    checks: &mut Vec<String>,
    metering: &mut serde_json::Map<String, Value>,
) {
    let mut batches = build_verifier_batches(findings);
    if batches.is_empty() {
        return;
    }
    let mut budget = VerifierCallBudget::default();
    let mut verdicts = Vec::new();
    let mut incomplete_indices = Vec::new();

    let run_batch = |messages: Vec<ChatMessage>| {
        run_subchat_once_with_explicit_params(
            gcx.clone(),
            VERIFIER_ATTRIBUTION_ID,
            &runner.spec,
            messages,
            parent.tool_call_id.clone(),
            parent.subchat_tx.clone(),
            parent.abort_flag.clone(),
            parent.depth,
            parent.task_meta.clone(),
            parent.worktree.clone(),
            TraceParent::rooted(&parent.chat_id, &parent.root_chat_id),
        )
    };

    while let Some(batch) = batches.pop_front() {
        if !budget.reserve() {
            let exhausted = batch
                .indices
                .into_iter()
                .chain(batches.into_iter().flat_map(|batch| batch.indices));
            mark_candidates(
                findings,
                checks,
                exhausted,
                VerificationStatus::NeedsHumanValidation,
                VERIFIER_BUDGET_EXHAUSTED,
            );
            break;
        }
        let batch_indices = batch.indices;
        let history = vec![
            ChatMessage::new("system".to_string(), runner.system_prompt.clone()),
            ChatMessage::new("user".to_string(), batch.prompt),
        ];
        let first = run_batch(history).await;
        let first = match first {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!("review verifier call failed: {error}");
                mark_candidates(
                    findings,
                    checks,
                    batch_indices,
                    VerificationStatus::Unverified,
                    VERIFIER_CALL_FAILED,
                );
                continue;
            }
        };
        let first_text = get_last_assistant_content(&first.messages);
        match parse_verifier_verdicts(&first_text, &batch_indices) {
            Ok(batch_verdicts) => {
                merge_metering(metering, first.metering);
                incomplete_indices.extend(missing_verdict_indices(&batch_indices, &batch_verdicts));
                verdicts.extend(batch_verdicts);
            }
            Err(first_error) => {
                tracing::info!(
                    "review verifier response malformed ({first_error}), requesting retry"
                );
                if !budget.reserve() {
                    merge_metering(metering, first.metering);
                    mark_candidates(
                        findings,
                        checks,
                        batch_indices,
                        VerificationStatus::Unverified,
                        VERIFIER_PARSE_FAILED,
                    );
                    continue;
                }
                let mut retry_messages = first.messages;
                retry_messages.push(ChatMessage::new(
                    "user".to_string(),
                    VERIFIER_RETRY_PROMPT.to_string(),
                ));
                match run_batch(retry_messages).await {
                    Ok(retry) => {
                        let retry_text = get_last_assistant_content(&retry.messages);
                        merge_metering(metering, retry.metering);
                        match parse_verifier_verdicts(&retry_text, &batch_indices) {
                            Ok(batch_verdicts) => {
                                incomplete_indices.extend(missing_verdict_indices(
                                    &batch_indices,
                                    &batch_verdicts,
                                ));
                                verdicts.extend(batch_verdicts);
                            }
                            Err(second_error) => {
                                tracing::warn!(
                                    "review verifier response malformed after retry: {second_error}"
                                );
                                mark_candidates(
                                    findings,
                                    checks,
                                    batch_indices,
                                    VerificationStatus::Unverified,
                                    VERIFIER_PARSE_FAILED,
                                );
                            }
                        }
                    }
                    Err(error) => {
                        merge_metering(metering, first.metering);
                        tracing::warn!("review verifier retry failed: {error}");
                        mark_candidates(
                            findings,
                            checks,
                            batch_indices,
                            VerificationStatus::Unverified,
                            VERIFIER_CALL_FAILED,
                        );
                    }
                }
            }
        }
    }

    apply_verifier_verdicts(findings, checks, verdicts, incomplete_indices);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::review_types::RankTier;

    fn finding(severity: ReviewSeverity, confidence: f32) -> ReviewFinding {
        ReviewFinding {
            id: String::new(),
            category: "correctness".to_string(),
            severity,
            confidence,
            verification_status: VerificationStatus::Unverified,
            rank_tier: RankTier::Unverified,
            sources: vec![],
            file: "src/lib.rs".to_string(),
            line1: 4,
            line2: 6,
            claim: "The branch drops errors.".to_string(),
            evidence: vec![ReviewEvidence {
                kind: "excerpt".to_string(),
                path: Some("src/lib.rs".to_string()),
                line1: Some(1),
                line2: Some(9),
                content: "4: return Ok(())".to_string(),
            }],
            impact: None,
            remediation: None,
            checks_performed: vec![],
        }
    }

    #[test]
    fn tool_review_verifier_parses_well_formed_verdicts() {
        let parsed = parse_verifier_verdicts(
            r#"```json
{"verdicts":[{"index":0,"status":"verified","confidence":0.6,"note":"Supported."}]}
```"#,
            &[0],
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].status, VerifierVerdictStatus::Verified);
        assert_eq!(parsed[0].confidence, 0.6);
        assert_eq!(parsed[0].note, "Supported.");
    }

    #[test]
    fn tool_review_verifier_rejects_malformed_verdicts() {
        assert!(parse_verifier_verdicts(
            r#"{"verdicts":[{"index":0,"status":"verified","confidence":0.6,"note":"Supported."}]}"#,
            &[0],
        )
        .is_err());
        assert!(parse_verifier_verdicts(
            r#"```json
{"verdicts":[{"index":0,"status":"maybe","confidence":0.6,"note":"Supported."}]}
```"#,
            &[0],
        )
        .is_err());
    }

    #[test]
    fn tool_review_verifier_marks_missing_and_ignores_unknown_indices() {
        let parsed = parse_verifier_verdicts(
            r#"```json
{"verdicts":[{"index":0,"status":"verified","confidence":0.6,"note":"Supported."},{"index":1,"status":"downgraded","confidence":0.5,"note":"Partial."},{"index":99,"status":"rejected","confidence":0.9,"note":"Unknown."}]}
```"#,
            &[0, 1, 2],
        )
        .unwrap();
        let missing = missing_verdict_indices(&[0, 1, 2], &parsed);
        let mut findings = vec![
            finding(ReviewSeverity::High, 0.9),
            finding(ReviewSeverity::High, 0.8),
            finding(ReviewSeverity::High, 0.7),
        ];
        let mut checks = vec![];

        assert_eq!(parsed.len(), 2);
        assert_eq!(missing, [2]);
        apply_verifier_verdicts(&mut findings, &mut checks, parsed, missing);
        assert_eq!(
            findings[0].verification_status,
            VerificationStatus::Verified
        );
        assert_eq!(
            findings[1].verification_status,
            VerificationStatus::Downgraded
        );
        assert_eq!(
            findings[2].verification_status,
            VerificationStatus::NeedsHumanValidation
        );
        assert_eq!(findings[2].checks_performed, [VERIFIER_INCOMPLETE]);
        assert_eq!(checks, ["verifier_incomplete:1"]);
    }

    #[test]
    fn tool_review_verifier_batches_at_ten_and_includes_evidence() {
        let findings = (0..25)
            .map(|_| finding(ReviewSeverity::High, 0.9))
            .collect::<Vec<_>>();
        let batches = build_verifier_batches(&findings)
            .into_iter()
            .collect::<Vec<_>>();

        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].indices.len(), 10);
        assert_eq!(batches[1].indices.len(), 10);
        assert_eq!(batches[2].indices.len(), 5);
        assert!(batches[0].prompt.contains("\"id-index\":\"0\""));
        assert!(batches[0]
            .prompt
            .contains("\"file:lines\":\"src/lib.rs:4-6\""));
        assert!(batches[0].prompt.contains("4: return Ok(())"));
    }

    #[test]
    fn tool_review_verifier_trigger_policy_matches_severity_confidence_and_size() {
        assert!(should_verify(&finding(ReviewSeverity::Critical, 1.0), 20));
        assert!(should_verify(&finding(ReviewSeverity::High, 1.0), 20));
        assert!(should_verify(&finding(ReviewSeverity::Medium, 0.69), 20));
        assert!(should_verify(&finding(ReviewSeverity::Low, 0.69), 20));
        assert!(!should_verify(&finding(ReviewSeverity::Medium, 0.7), 20));
        assert!(!should_verify(&finding(ReviewSeverity::Low, 1.0), 20));
        assert!(should_verify(&finding(ReviewSeverity::Low, 1.0), 5));
    }

    #[test]
    fn tool_review_verifier_budget_exhaustion_marks_overflow_candidates() {
        let mut findings = (0..31)
            .map(|_| finding(ReviewSeverity::High, 0.9))
            .collect::<Vec<_>>();
        let mut checks = vec![];
        let batches = build_verifier_batches(&findings)
            .into_iter()
            .collect::<Vec<_>>();
        let mut budget = VerifierCallBudget::default();

        assert!(budget.reserve());
        assert!(budget.reserve());
        assert!(budget.reserve());
        assert!(!budget.reserve());
        mark_candidates(
            &mut findings,
            &mut checks,
            batches[3].indices.clone(),
            VerificationStatus::NeedsHumanValidation,
            VERIFIER_BUDGET_EXHAUSTED,
        );

        assert_eq!(
            findings[30].verification_status,
            VerificationStatus::NeedsHumanValidation
        );
        assert!(findings[30]
            .checks_performed
            .contains(&VERIFIER_BUDGET_EXHAUSTED.to_string()));
        assert!(checks.contains(&VERIFIER_BUDGET_EXHAUSTED.to_string()));
    }

    #[test]
    fn tool_review_verifier_parse_failure_keeps_candidates_unverified() {
        let mut findings = vec![finding(ReviewSeverity::High, 0.9)];
        let mut checks = vec![];

        mark_candidates(
            &mut findings,
            &mut checks,
            [0],
            VerificationStatus::Unverified,
            VERIFIER_PARSE_FAILED,
        );

        assert_eq!(
            findings[0].verification_status,
            VerificationStatus::Unverified
        );
        assert!(findings[0]
            .checks_performed
            .contains(&VERIFIER_PARSE_FAILED.to_string()));
        assert!(checks.contains(&VERIFIER_PARSE_FAILED.to_string()));
    }

    #[test]
    fn tool_review_verifier_accumulates_nested_metering() {
        let mut metering = serde_json::Map::from_iter([
            ("metering_tokens".to_string(), Value::from(10)),
            (
                "metering_usd".to_string(),
                serde_json::json!({"total_usd": 0.25}),
            ),
        ]);

        merge_metering(
            &mut metering,
            serde_json::Map::from_iter([
                ("metering_tokens".to_string(), Value::from(7)),
                (
                    "metering_usd".to_string(),
                    serde_json::json!({"total_usd": 0.5}),
                ),
            ]),
        );

        assert_eq!(metering["metering_tokens"], 17);
        assert_eq!(metering["metering_usd"]["total_usd"], 0.75);
    }

    #[test]
    fn tool_review_verifier_applies_reject_downgrade_verify_and_human_status() {
        let mut findings = vec![
            finding(ReviewSeverity::Critical, 0.9),
            finding(ReviewSeverity::High, 0.8),
            finding(ReviewSeverity::Medium, 0.7),
            finding(ReviewSeverity::Low, 0.6),
            finding(ReviewSeverity::High, 0.5),
            finding(ReviewSeverity::High, 0.4),
            finding(ReviewSeverity::High, 0.3),
        ];
        let mut checks = vec![];
        apply_verifier_verdicts(
            &mut findings,
            &mut checks,
            vec![
                VerifierVerdict {
                    index: 0,
                    status: VerifierVerdictStatus::Downgraded,
                    confidence: 0.9,
                    note: String::new(),
                },
                VerifierVerdict {
                    index: 1,
                    status: VerifierVerdictStatus::Downgraded,
                    confidence: 0.8,
                    note: String::new(),
                },
                VerifierVerdict {
                    index: 2,
                    status: VerifierVerdictStatus::Downgraded,
                    confidence: 0.7,
                    note: String::new(),
                },
                VerifierVerdict {
                    index: 3,
                    status: VerifierVerdictStatus::Downgraded,
                    confidence: 0.6,
                    note: String::new(),
                },
                VerifierVerdict {
                    index: 4,
                    status: VerifierVerdictStatus::Verified,
                    confidence: 0.2,
                    note: String::new(),
                },
                VerifierVerdict {
                    index: 5,
                    status: VerifierVerdictStatus::NeedsHumanValidation,
                    confidence: 0.4,
                    note: String::new(),
                },
                VerifierVerdict {
                    index: 6,
                    status: VerifierVerdictStatus::Rejected,
                    confidence: 0.9,
                    note: String::new(),
                },
            ],
            [],
        );

        assert_eq!(findings.len(), 6);
        assert_eq!(findings[0].severity, ReviewSeverity::High);
        assert_eq!(findings[1].severity, ReviewSeverity::Medium);
        assert_eq!(findings[2].severity, ReviewSeverity::Low);
        assert_eq!(findings[3].severity, ReviewSeverity::Low);
        assert_eq!(
            findings[4].verification_status,
            VerificationStatus::Verified
        );
        assert_eq!(findings[4].confidence, 0.2);
        assert_eq!(
            findings[5].verification_status,
            VerificationStatus::NeedsHumanValidation
        );
        assert_eq!(checks, vec!["verifier_rejected:1"]);
    }
}
