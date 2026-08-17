use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Deserialize;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::subchat::{
    resolve_subchat_config_with_explicit_params, run_subchat,
    run_subchat_once_with_explicit_params, ExplicitSubchatSpec,
};
use crate::tools::review_agents::{now_ms, AgentCtx, AgentOutcome};
use crate::tools::review_candidates::{extract_last_json_block, validate_candidate, CandidateFinding};
use crate::tools::review_evidence::collect_evidence;
use crate::tools::review_scope::ReviewScope;
use crate::tools::review_types::{evidence_kinds, ReviewEvidence, ReviewFinding, VerificationStatus};
use crate::tools::review_verify::merge_metering;
use crate::tools::subagent_phases::get_last_assistant_content;

pub const STATIC_ENRICH_DEFAULT_TOOLS: &[&str] = &[
    "cat",
    "tree",
    "glob",
    "regex_search",
    "symbol_def",
    "semantic_search",
];

pub const DEFAULT_STATIC_ENRICHMENT_PROMPT: &str = r#"You are a static-analysis triage investigator. A deterministic analyzer produced raw findings; your job is to prove, refute, or enrich each one by searching the actual codebase with your tools. Never trust the raw finding text alone: read the flagged lines with cat, trace usage with regex_search and symbol_def, and check the wider context. Be decisive: refute false positives with concrete proof, confirm true positives with the strongest evidence you found, and adjust severity to the real impact. Investigate every raw finding within your budget, highest severity first. When your investigation is complete, end your FINAL answer with the envelope exactly as specified in the task message."#;

const MAX_NEW_FINDINGS: usize = 5;
const MAX_RAW_IN_PROMPT: usize = 30;

const STATIC_RETRY_PROMPT: &str = r#"Your response did not end with a valid envelope. Return your conclusions again and end with exactly one fenced json block matching this schema:

```json
{"summary":"<2-4 sentences>","refuted":[{"index":1,"reason":"<why, citing the file:line proof you found>"}],"candidates":[{"file":"<path as given>","line1":1,"line2":10,"category":"correctness|consistency|security|tests|maintainability|performance|spec_compliance","severity":"low|medium|high|critical","confidence":0.0,"claim":"<one falsifiable sentence>","rationale":"<short why>","confirms":1}]}
```

Use "confirms" with the 1-based raw finding index for findings you confirmed, omit it for new discoveries, and list disproved raw findings in "refuted". Do not put another fenced json block before or after the envelope."#;

pub fn investigation_guidance(agent_id: &str) -> &'static str {
    match agent_id {
        "s1_security" => "Each raw hit is a pattern match, not a proven vulnerability. Investigate exploitability: is the flagged line in production code or in tests/fixtures/examples/docs; does attacker-controlled input actually reach the sink, or is it sanitized or constant upstream; is the flagged secret real (used by production paths) or a placeholder; is weak crypto used for security or only for checksums/ids; is disabled TLS scoped to local development. Trace how the flagged value flows before deciding.",
        "s2_dead_code" => "The static graph misses dynamic usage. Before confirming a symbol as dead, search for: reflection or dynamic dispatch, macro-generated references, public API surface of a library, FFI or extern usage, CLI/route/plugin registrations, config-driven lookups by string name, test-only usage, feature-gated call sites, and trait implementations required by a trait. Refute when any real usage exists; confirm only with a usage-free search as proof.",
        "s3_duplication" => "Decide whether each clone pair is a true DRY violation worth fixing: is one side newly added by this change (duplicating existing code is worse); is there an existing shared helper both sides could call; or is the duplication justified (generated code, test fixtures, platform-specific variants, intentional decoupling). When confirming, name the concrete extraction target or the helper to reuse.",
        "s4_test_integrity" => "Raw hits are diff-pattern matches. Verify intent: did deleted or renamed tests move elsewhere (search the test names across the repository); are skips or ignores justified by linked issues or platform guards; did tolerance or matcher changes follow a legitimate behavior change visible in the conversation; does literal overlap between implementation and test indicate a hardcoded expectation or just a shared constant.",
        "s5_dependencies" => "Confirm each import is truly unresolvable: search every manifest in the workspace (root and member Cargo.toml, package.json, requirements, go.mod), lockfiles, vendored or patched sources, path and workspace dependencies, optional features, dev-only declarations, and language builtins. Refute when a declaration exists anywhere; confirm hallucinated dependencies and list where you searched.",
        _ => "Investigate each raw finding in the actual codebase: read the flagged lines, trace usage, and decide whether the finding is real.",
    }
}

pub struct StaticEnrichmentInput {
    pub agent_id: String,
    pub slot_label: String,
    pub spec: ExplicitSubchatSpec,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub max_steps: usize,
    pub raw: AgentOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StaticRefutation {
    pub index: usize,
    pub reason: String,
}

#[derive(Debug, PartialEq)]
pub struct StaticParsed {
    pub summary: String,
    pub candidates: Vec<(CandidateFinding, Option<usize>)>,
    pub refuted: Vec<StaticRefutation>,
}

#[derive(Deserialize)]
struct StaticEnvelope {
    summary: String,
    candidates: Vec<serde_json::Value>,
    #[serde(default)]
    refuted: Vec<serde_json::Value>,
}

pub fn parse_static_envelope(text: &str, raw_count: usize) -> Result<StaticParsed, String> {
    let json = extract_last_json_block(text).map_err(|e| e.to_string())?;
    let envelope: StaticEnvelope =
        serde_json::from_str(json).map_err(|e| format!("invalid envelope json: {e}"))?;
    let mut candidates = Vec::with_capacity(envelope.candidates.len());
    for (index, value) in envelope.candidates.into_iter().enumerate() {
        let confirms = value
            .get("confirms")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize)
            .filter(|v| (1..=raw_count).contains(v));
        match validate_candidate(index, value) {
            Ok(candidate) => candidates.push((candidate, confirms)),
            Err(reason) => tracing::warn!("static enrichment: {reason}"),
        }
    }
    let mut refuted = Vec::new();
    for value in envelope.refuted {
        let Some(index) = value
            .get("index")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize)
            .filter(|v| (1..=raw_count).contains(v))
        else {
            continue;
        };
        let reason = value
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("no reason given")
            .to_string();
        if !refuted.iter().any(|r: &StaticRefutation| r.index == index) {
            refuted.push(StaticRefutation { index, reason });
        }
    }
    Ok(StaticParsed {
        summary: envelope.summary.trim().to_string(),
        candidates,
        refuted,
    })
}

fn raw_findings_json(findings: &[ReviewFinding]) -> String {
    let entries: Vec<serde_json::Value> = findings
        .iter()
        .take(MAX_RAW_IN_PROMPT)
        .enumerate()
        .map(|(i, finding)| {
            serde_json::json!({
                "index": i + 1,
                "file": finding.file,
                "line1": finding.line1,
                "line2": finding.line2,
                "category": finding.category,
                "severity": finding.severity,
                "claim": finding.claim,
                "fact": finding.evidence.first().map(|e| e.content.as_str()).unwrap_or(""),
            })
        })
        .collect();
    serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
}

pub fn build_static_task_prompt(
    agent_id: &str,
    scope: &ReviewScope,
    raw_findings: &[ReviewFinding],
) -> String {
    let mut prompt = String::new();
    prompt.push_str(&format!(
        "# Check under investigation\nDeterministic check `{agent_id}` flagged the raw findings below.\n{}\n\n",
        investigation_guidance(agent_id)
    ));
    if let Some(focus) = scope.focus.as_deref() {
        prompt.push_str(&format!("# Focus — what to check\n{focus}\n\n"));
    }
    prompt.push_str("# Raw findings (1-based index)\n```json\n");
    prompt.push_str(&raw_findings_json(raw_findings));
    prompt.push_str("\n```\n");
    if raw_findings.len() > MAX_RAW_IN_PROMPT {
        prompt.push_str(&format!(
            "… and {} more raw findings not shown; they will be reported unverified.\n",
            raw_findings.len() - MAX_RAW_IN_PROMPT
        ));
    }
    if !scope.changed_files.is_empty() {
        prompt.push_str("\n# Changed files\n");
        for path in scope.changed_files.iter().take(40) {
            prompt.push_str(&format!("- {}\n", path.to_string_lossy()));
        }
    }
    prompt.push_str(&format!(
        r#"
# Output contract
When your investigation is complete, end your FINAL answer with exactly one fenced json block:

```json
{{"summary":"<2-4 sentences>","refuted":[{{"index":2,"reason":"<why, citing the file:line proof you found>"}}],"candidates":[{{"file":"<path as given>","line1":1,"line2":10,"category":"correctness|consistency|security|tests|maintainability|performance|spec_compliance","severity":"low|medium|high|critical","confidence":0.0,"claim":"<one falsifiable sentence>","rationale":"<short why>","confirms":1}}]}}
```

- "confirms": the 1-based index of the raw finding this candidate proves. Emit one candidate per confirmed finding with your improved claim, exact lines, and severity adjusted to real impact.
- "refuted": raw findings you disproved with concrete evidence found in the codebase.
- Raw findings you neither confirm nor refute are reported unchanged as unverified; leave a finding untouched only when you run out of budget.
- Candidates without "confirms" are new discoveries directly related to this check; report at most {MAX_NEW_FINDINGS} of them.
"#
    ));
    prompt
}

enum Origin {
    Confirmed(usize),
    New,
}

fn reconcile(
    raw_findings: &[ReviewFinding],
    parsed: StaticParsed,
    label: &str,
) -> (
    Vec<ReviewFinding>,
    Vec<Origin>,
    Vec<ReviewFinding>,
    Vec<String>,
) {
    let mut checks = Vec::new();
    let refuted_set: HashSet<usize> = parsed.refuted.iter().map(|r| r.index).collect();
    if !refuted_set.is_empty() {
        checks.push(format!("static_refuted:{}", refuted_set.len()));
        for refutation in &parsed.refuted {
            tracing::info!(
                "{label}: refuted raw finding {}: {}",
                refutation.index,
                refutation.reason
            );
        }
    }

    let mut confirmed_by_raw: HashMap<usize, CandidateFinding> = HashMap::new();
    let mut new_candidates: Vec<CandidateFinding> = Vec::new();
    for (candidate, confirms) in parsed.candidates {
        match confirms {
            Some(raw_index) if !refuted_set.contains(&raw_index) => {
                confirmed_by_raw.entry(raw_index).or_insert(candidate);
            }
            Some(_) => {}
            None => {
                if new_candidates.len() < MAX_NEW_FINDINGS {
                    new_candidates.push(candidate);
                }
            }
        }
    }

    let mut confirmed_sorted: Vec<(usize, CandidateFinding)> =
        confirmed_by_raw.into_iter().collect();
    confirmed_sorted.sort_by_key(|(raw_index, _)| *raw_index);
    let mut enriched: Vec<ReviewFinding> = Vec::new();
    let mut origins: Vec<Origin> = Vec::new();
    for (raw_index, candidate) in confirmed_sorted {
        let raw = &raw_findings[raw_index - 1];
        let (mut finding, _rationale) = candidate.into_review_parts();
        finding.verification_status = VerificationStatus::Verified;
        finding.sources = vec![label.to_string()];
        finding
            .checks_performed
            .push("static_confirmed".to_string());
        for evidence in raw
            .evidence
            .iter()
            .filter(|e| e.kind == evidence_kinds::STATIC_FACT)
        {
            finding.evidence.push(ReviewEvidence {
                kind: evidence.kind.clone(),
                path: evidence.path.clone(),
                line1: evidence.line1,
                line2: evidence.line2,
                content: evidence.content.clone(),
            });
        }
        enriched.push(finding);
        origins.push(Origin::Confirmed(raw_index));
    }
    for candidate in new_candidates {
        let (mut finding, _rationale) = candidate.into_review_parts();
        finding.sources = vec![label.to_string()];
        enriched.push(finding);
        origins.push(Origin::New);
    }

    let kept_raw: Vec<ReviewFinding> = raw_findings
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let index = i + 1;
            !refuted_set.contains(&index) && !confirmed_by_raw_contains(&origins, index)
        })
        .map(|(_, finding)| finding.clone())
        .collect();

    (enriched, origins, kept_raw, checks)
}

fn confirmed_by_raw_contains(origins: &[Origin], raw_index: usize) -> bool {
    origins
        .iter()
        .any(|origin| matches!(origin, Origin::Confirmed(index) if *index == raw_index))
}

pub(crate) async fn run_static_enrichment(
    gcx: Arc<crate::global_context::GlobalContext>,
    ctx: AgentCtx,
    input: StaticEnrichmentInput,
    scope: Arc<ReviewScope>,
) -> AgentOutcome {
    let started = now_ms();
    let raw = input.raw;
    let label = format!("{}@{}", input.agent_id, input.slot_label);
    let attribution = format!("review_{}", input.agent_id);
    let raw_count = raw.findings.len();

    let enrich = enrich_once(
        gcx.clone(),
        &ctx,
        &input.agent_id,
        &attribution,
        &input.spec,
        &input.system_prompt,
        &input.tools,
        input.max_steps,
        &scope,
        &raw.findings,
    )
    .await;

    let (parsed, steps, metering) = match enrich {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!("{label}: enrichment failed, keeping raw findings: {error}");
            let mut outcome = raw;
            outcome.run.reason = Some(format!("enrichment_failed:{error}"));
            outcome.run.duration_ms = now_ms().saturating_sub(started);
            return outcome;
        }
    };

    let (mut enriched, origins, kept_raw, mut checks) = reconcile(&raw.findings, parsed, &label);

    let rejections = collect_evidence(gcx.clone(), &scope, &mut enriched).await;
    let mut surviving = enriched;
    for rejection in &rejections {
        checks.push(rejection.check_name());
        if let Some(Origin::Confirmed(raw_index)) = origins.get(rejection.index - 1) {
            surviving.push(raw.findings[raw_index - 1].clone());
            checks.push(format!("static_enrich_fallback:{raw_index}"));
        }
    }
    surviving.extend(kept_raw);

    let mut outcome = AgentOutcome::ran(
        &label,
        Some(input.spec.model.clone()),
        raw_count,
        surviving,
        started,
    );
    for check in raw.checks {
        if !checks.contains(&check) {
            checks.insert(0, check);
        }
    }
    outcome.checks = checks;
    outcome.metering = metering;
    outcome.run.steps = Some(steps);
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn enrich_once(
    gcx: Arc<crate::global_context::GlobalContext>,
    ctx: &AgentCtx,
    agent_id: &str,
    attribution: &str,
    spec: &ExplicitSubchatSpec,
    system_prompt: &str,
    tools: &[String],
    max_steps: usize,
    scope: &ReviewScope,
    raw_findings: &[ReviewFinding],
) -> Result<
    (
        StaticParsed,
        usize,
        serde_json::Map<String, serde_json::Value>,
    ),
    String,
> {
    let mut metering = serde_json::Map::new();
    let config = resolve_subchat_config_with_explicit_params(
        gcx.clone(),
        attribution,
        spec,
        true,
        None,
        Some(format!("Review: {agent_id} enrichment")),
        Some(ctx.chat_id.clone()),
        Some("review_agent".to_string()),
        Some(ctx.root_chat_id.clone()),
        Some(tools.to_vec()),
        max_steps,
        false,
        "agent".to_string(),
        ctx.task_meta.clone(),
        ctx.worktree.clone(),
        Some(ctx.tool_call_id.clone()),
        Some(ctx.subchat_tx.clone()),
        Some(ctx.abort_flag.clone()),
        ctx.depth + 1,
    )
    .await?;

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(system_prompt.to_string()),
            ..Default::default()
        },
        ChatMessage::new(
            "user".to_string(),
            build_static_task_prompt(agent_id, scope, raw_findings),
        ),
    ];

    let result = run_subchat(gcx.clone(), messages, config).await?;
    merge_metering(&mut metering, result.metering.clone());
    let steps = result
        .messages
        .iter()
        .filter(|m| m.role == "assistant")
        .count();
    let text = get_last_assistant_content(&result.messages);
    let raw_count = raw_findings.len().min(MAX_RAW_IN_PROMPT);

    match parse_static_envelope(&text, raw_count) {
        Ok(parsed) => Ok((parsed, steps, metering)),
        Err(first_error) => {
            tracing::info!("{agent_id}: envelope missing ({first_error}), requesting retry");
            let mut retry_messages = result.messages;
            retry_messages.push(ChatMessage::new(
                "user".to_string(),
                STATIC_RETRY_PROMPT.to_string(),
            ));
            let retry = run_subchat_once_with_explicit_params(
                gcx.clone(),
                attribution,
                spec,
                retry_messages,
                ctx.tool_call_id.clone(),
                ctx.subchat_tx.clone(),
                ctx.abort_flag.clone(),
                ctx.depth,
                ctx.task_meta.clone(),
                ctx.worktree.clone(),
            )
            .await?;
            merge_metering(&mut metering, retry.metering.clone());
            let retry_text = get_last_assistant_content(&retry.messages);
            let parsed = parse_static_envelope(&retry_text, raw_count)?;
            Ok((parsed, steps + 1, metering))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::review_types::{RankTier, ReviewSeverity};

    fn raw_finding(file: &str, line: u32, claim: &str) -> ReviewFinding {
        ReviewFinding {
            id: String::new(),
            category: "security".to_string(),
            severity: ReviewSeverity::High,
            confidence: 0.85,
            verification_status: VerificationStatus::Unverified,
            rank_tier: RankTier::Unverified,
            sources: vec!["s1_security".to_string()],
            file: file.to_string(),
            line1: line,
            line2: line,
            claim: claim.to_string(),
            evidence: vec![ReviewEvidence {
                kind: evidence_kinds::STATIC_FACT.to_string(),
                path: Some(file.to_string()),
                line1: Some(line),
                line2: Some(line),
                content: format!("static fact for {claim}"),
            }],
            impact: None,
            remediation: None,
            checks_performed: vec![],
        }
    }

    fn envelope(candidates: &str, refuted: &str) -> String {
        format!(
            "```json\n{{\"summary\":\"Investigated.\",\"refuted\":[{refuted}],\"candidates\":[{candidates}]}}\n```"
        )
    }

    fn candidate(confirms: Option<usize>, file: &str, line: u32) -> String {
        let confirms_part = confirms
            .map(|c| format!(",\"confirms\":{c}"))
            .unwrap_or_default();
        format!(
            r#"{{"file":"{file}","line1":{line},"line2":{line},"category":"security","severity":"medium","confidence":0.9,"claim":"Proven issue at {file}:{line}.","rationale":"traced"{confirms_part}}}"#
        )
    }

    #[test]
    fn static_envelope_parses_confirms_and_refuted() {
        let text = envelope(
            &format!(
                "{},{}",
                candidate(Some(1), "a.rs", 5),
                candidate(None, "b.rs", 9)
            ),
            r#"{"index":2,"reason":"used via CLI registration in main.rs:88"}"#,
        );
        let parsed = parse_static_envelope(&text, 3).unwrap();
        assert_eq!(parsed.candidates.len(), 2);
        assert_eq!(parsed.candidates[0].1, Some(1));
        assert_eq!(parsed.candidates[1].1, None);
        assert_eq!(
            parsed.refuted,
            vec![StaticRefutation {
                index: 2,
                reason: "used via CLI registration in main.rs:88".to_string()
            }]
        );
    }

    #[test]
    fn static_envelope_bound_is_clamped_to_prompt_visible_findings() {
        let visible = MAX_RAW_IN_PROMPT;
        let text = envelope(
            &candidate(Some(visible + 1), "a.rs", 5),
            &format!(r#"{{"index":{},"reason":"never shown"}}"#, visible + 2),
        );
        let parsed = parse_static_envelope(&text, visible).unwrap();
        assert_eq!(parsed.candidates[0].1, None);
        assert!(parsed.refuted.is_empty());
    }

    #[test]
    fn static_envelope_drops_out_of_range_confirms_and_refutations() {
        let text = envelope(
            &candidate(Some(9), "a.rs", 5),
            r#"{"index":0,"reason":"bad"},{"index":7,"reason":"bad"}"#,
        );
        let parsed = parse_static_envelope(&text, 2).unwrap();
        assert_eq!(parsed.candidates[0].1, None);
        assert!(parsed.refuted.is_empty());
    }

    #[test]
    fn static_envelope_requires_json_block() {
        assert!(parse_static_envelope("no block here", 1).is_err());
    }

    #[test]
    fn static_reconcile_confirms_refutes_and_keeps_untouched() {
        let raw = vec![
            raw_finding("a.rs", 5, "Secret in a.rs"),
            raw_finding("b.rs", 10, "Secret in b.rs"),
            raw_finding("c.rs", 15, "Secret in c.rs"),
        ];
        let parsed = parse_static_envelope(
            &envelope(
                &candidate(Some(1), "a.rs", 5),
                r#"{"index":2,"reason":"test fixture only"}"#,
            ),
            raw.len(),
        )
        .unwrap();

        let (enriched, origins, kept_raw, checks) = reconcile(&raw, parsed, "s1_security@light");

        assert_eq!(enriched.len(), 1);
        assert_eq!(
            enriched[0].verification_status,
            VerificationStatus::Verified
        );
        assert_eq!(enriched[0].sources, vec!["s1_security@light".to_string()]);
        assert!(enriched[0]
            .checks_performed
            .contains(&"static_confirmed".to_string()));
        assert!(enriched[0]
            .evidence
            .iter()
            .any(|e| e.kind == evidence_kinds::STATIC_FACT));
        assert!(matches!(origins[0], Origin::Confirmed(1)));
        assert_eq!(kept_raw.len(), 1);
        assert_eq!(kept_raw[0].file, "c.rs");
        assert!(checks.contains(&"static_refuted:1".to_string()));
    }

    #[test]
    fn static_reconcile_caps_new_findings_and_ignores_refuted_confirms() {
        let raw = vec![raw_finding("a.rs", 5, "Secret in a.rs")];
        let candidates = (0..8)
            .map(|i| candidate(None, "new.rs", 10 + i))
            .chain(std::iter::once(candidate(Some(1), "a.rs", 5)))
            .collect::<Vec<_>>()
            .join(",");
        let parsed = parse_static_envelope(
            &envelope(&candidates, r#"{"index":1,"reason":"fixture"}"#),
            raw.len(),
        )
        .unwrap();

        let (enriched, origins, kept_raw, _checks) = reconcile(&raw, parsed, "s1_security@light");

        assert_eq!(enriched.len(), MAX_NEW_FINDINGS);
        assert!(origins.iter().all(|origin| matches!(origin, Origin::New)));
        assert!(enriched
            .iter()
            .all(|f| f.verification_status == VerificationStatus::Unverified));
        assert!(kept_raw.is_empty());
    }

    #[test]
    fn static_task_prompt_contains_guidance_findings_and_contract() {
        let raw = vec![raw_finding("a.rs", 5, "Secret in a.rs")];
        let scope = ReviewScope {
            files: vec![],
            seed_files: vec![],
            focus: Some("check secrets".to_string()),
            diff_base: None,
            changed_files: vec![std::path::PathBuf::from("a.rs")],
            diff_patch: None,
            budgets: crate::tools::review_scope::ReviewBudgets {
                max_files: 10,
                tokens_budget: 10_000,
                max_candidates: 30,
            },
        };
        let prompt = build_static_task_prompt("s1_security", &scope, &raw);
        assert!(prompt.contains("`s1_security`"));
        assert!(prompt.contains("pattern match, not a proven vulnerability"));
        assert!(prompt.contains("check secrets"));
        assert!(prompt.contains("\"index\": 1"));
        assert!(prompt.contains("Secret in a.rs"));
        assert!(prompt.contains("# Changed files"));
        assert!(prompt.contains("\"confirms\":1"));
        assert!(prompt.contains("reported unchanged as unverified"));
    }

    #[test]
    fn static_guidance_covers_all_five_checks() {
        for agent in [
            "s1_security",
            "s2_dead_code",
            "s3_duplication",
            "s4_test_integrity",
            "s5_dependencies",
        ] {
            assert!(investigation_guidance(agent).len() > 100, "{agent}");
        }
        assert!(investigation_guidance("unknown").contains("raw finding"));
    }
}
