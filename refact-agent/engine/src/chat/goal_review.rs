//! Agentic review as an evidence source for the goal path.
//!
//! The engine already owns one multi-stage review pipeline in
//! `tools::tool_review::run_review`. This module adds a second *caller* — the
//! goal path — and never a second pipeline. What comes back is a
//! `ReviewReport`, which is turned into a typed `EvidenceRecord` that V-1's
//! gates consume: a review is evidence the goal path weighs, never a verdict it
//! obeys. `Inconclusive` and `Partial` are recorded as *unavailable* evidence
//! and are therefore not a pass; only findings the pipeline could support as
//! facts contribute to a blocking outcome, and repair rounds are bounded so a
//! goal escalates explicitly instead of looping.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::sync::Mutex as AMutex;

use crate::chat::evidence::{
    evaluate_goal_gates, unexecuted_record, EvidenceDefaults, EvidenceRecord, EvidenceVerdict,
    SpecAuthority, VerificationSpec,
};
use crate::chat::evidence_cache::EvidenceCacheKey;
use crate::chat::goal_verifier::GoalEvidence;
use crate::chat::review_reuse::{
    review_policy_fingerprint, review_reuse_limits, review_reuse_store, review_revision,
    ReusableReview, ReviewIdentity, ReviewReuseInputs, ReviewReuseStore,
};
use crate::chat::types::{ChatSession, TaskMeta};
use crate::global_context::GlobalContext;
use crate::tasks::types::VerificationOutcome;
use crate::tools::review_agents::config::{load_review_config, slot_model_id, ReviewConfig};
use crate::tools::review_agents::runner::StageCtx;
use crate::tools::review_agents::stages::{load_stage_catalog, select_stages};
use crate::tools::review_types::{
    ReviewFinding, ReviewOutcome, ReviewReport, ReviewSeverity, ScopeMode,
};
use crate::tools::tool_review::{run_review, ReviewArgs};
use crate::worktrees::types::WorktreeMeta;

pub const GOAL_REVIEW_SPEC_ID: &str = "goal_review";
pub const GOAL_REVIEW_AUTHOR: &str = "chat.goal_review";

/// A review is an agentic investigation, not a command. The spec carries prose
/// on purpose: the evidence service refuses to turn an agent proposal that is
/// prose into argv, so this record can never be executed by accident.
pub const GOAL_REVIEW_COMMAND: &str = "review pipeline (agentic, not an executable command)";

pub const GOAL_REVIEW_DISABLED: &str =
    "goal review is off by default; enable goal_review_enabled to spend model calls on it";

/// A reviewer running on another model slot is additional review. It is not a
/// second opinion drawn from an independent source, and the record says so.
const REVIEWER_DISCLAIMER: &str =
    "a different model slot is additional review, not proof of independence";

/// Below this severity a supported finding is reported but never blocks a goal.
const BLOCKING_SEVERITY: ReviewSeverity = ReviewSeverity::High;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalReviewSettings {
    pub enabled: bool,
    pub max_repair_attempts: usize,
    pub spot_check_percent: usize,
}

impl GoalReviewSettings {
    pub fn from_settings() -> Self {
        let settings = crate::runtime_settings::current();
        Self {
            enabled: settings.goal_review_enabled,
            max_repair_attempts: settings.goal_review_max_repair_attempts,
            spot_check_percent: settings.goal_review_spot_check_percent,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoalReviewRequest {
    pub what_to_check: Option<String>,
    pub plan: Option<String>,
    pub base: Option<String>,
    pub files: Vec<String>,
}

impl GoalReviewRequest {
    fn into_review_args(self) -> ReviewArgs {
        let scope_mode = if self.files.is_empty() && self.base.is_none() {
            ScopeMode::Broad
        } else {
            ScopeMode::Strict
        };
        ReviewArgs {
            what_to_check: self.what_to_check,
            files: self.files,
            base: self.base,
            plan: self.plan,
            scope_mode,
            stages: None,
            depth: None,
            parallel_depth: None,
            variants: None,
            browser_scenario: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalReviewOutcome {
    Skipped {
        reason: String,
    },
    Unavailable {
        reason: String,
    },
    Clean,
    Repair {
        attempt: usize,
        remaining: usize,
        findings: Vec<ReviewFinding>,
    },
    Escalated {
        attempts: usize,
        findings: Vec<ReviewFinding>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalReviewResult {
    pub outcome: GoalReviewOutcome,
    pub report: Option<ReviewReport>,
    pub evidence: Option<(VerificationSpec, EvidenceRecord)>,
}

/// Findings that may stand in the way of a goal: facts only. A hypothesis — no
/// reproduction and no verified evidence, or disputed as unsupported by the
/// adversarial stage — is reported but never blocks.
pub fn blocking_findings(report: &ReviewReport) -> Vec<ReviewFinding> {
    report
        .facts()
        .into_iter()
        .filter(|finding| finding.severity.rank() >= BLOCKING_SEVERITY.rank())
        .cloned()
        .collect()
}

/// The models that actually produced this review, in stage order.
pub fn reviewer_models(report: &ReviewReport) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    for stage in &report.stages {
        if let Some(model) = stage.model.as_deref() {
            if !model.is_empty() && !models.iter().any(|known| known == model) {
                models.push(model.to_string());
            }
        }
    }
    models
}

fn reviewer_label(report: &ReviewReport) -> String {
    let models = reviewer_models(report);
    if models.is_empty() {
        return format!("no reviewer model reported; {REVIEWER_DISCLAIMER}");
    }
    format!("reviewed by {}; {REVIEWER_DISCLAIMER}", models.join(", "))
}

/// How a review reads as evidence. A review that could not run is not a pass,
/// and `Partial` is kept distinct from a clean `Reviewed`.
pub fn review_verdict(report: &ReviewReport) -> (EvidenceVerdict, VerificationOutcome, String) {
    let blocking = blocking_findings(report);
    let label = reviewer_label(report);
    if !blocking.is_empty() {
        let titles = blocking
            .iter()
            .map(|finding| {
                format!(
                    "{} [{}] {}:{}",
                    finding.id,
                    finding.severity.as_str(),
                    finding.file,
                    finding.line_start
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        return (
            EvidenceVerdict::Failed,
            VerificationOutcome::CommandFailed,
            format!(
                "review [{}] found {} supported finding(s): {titles} — {label}",
                report.outcome.as_str(),
                blocking.len()
            ),
        );
    }
    match report.outcome {
        ReviewOutcome::Reviewed => (
            EvidenceVerdict::Passed,
            VerificationOutcome::Passed,
            format!("review [reviewed] found no supported blocking findings — {label}"),
        ),
        ReviewOutcome::Partial => (
            EvidenceVerdict::Unavailable,
            VerificationOutcome::InfrastructureFailed,
            format!(
                "review [partial] completed {} of {} stage(s); the rest of the change was not reviewed, which is not a pass — {label}",
                report.completed_stages(),
                report.stages.len()
            ),
        ),
        ReviewOutcome::Inconclusive => (
            EvidenceVerdict::Unavailable,
            VerificationOutcome::InfrastructureFailed,
            format!(
                "review [inconclusive] produced no stage result; nothing was checked, which is not a pass — {label}"
            ),
        ),
    }
}

pub fn review_spec(defaults: &EvidenceDefaults) -> VerificationSpec {
    let mut spec = VerificationSpec::propose(
        GOAL_REVIEW_SPEC_ID,
        GOAL_REVIEW_COMMAND,
        SpecAuthority::AgentProposal {
            agent: GOAL_REVIEW_AUTHOR.to_string(),
        },
        defaults,
    );
    spec.criterion_ids = vec![GOAL_REVIEW_SPEC_ID.to_string()];
    spec
}

pub fn review_evidence_record(report: &ReviewReport, spec: &VerificationSpec) -> EvidenceRecord {
    let (verdict, outcome, reason) = review_verdict(report);
    unexecuted_record(spec, verdict, outcome, reason)
}

/// Feed a review into the goal's evidence set and re-decide the gates from it.
pub fn attach_review_evidence(
    evidence: &mut GoalEvidence,
    spec: VerificationSpec,
    record: EvidenceRecord,
) {
    evidence.specs.push(spec);
    evidence.records.push(record);
    evidence.gates = evaluate_goal_gates(&evidence.specs, &evidence.records);
}

/// Bounded repair: at most `max_attempts` repair rounds, then an explicit
/// escalation carrying the findings that are still outstanding.
pub fn decide_repair(
    report: &ReviewReport,
    attempts_used: usize,
    max_attempts: usize,
) -> GoalReviewOutcome {
    let findings = blocking_findings(report);
    if findings.is_empty() {
        return match report.outcome {
            ReviewOutcome::Reviewed => GoalReviewOutcome::Clean,
            _ => GoalReviewOutcome::Unavailable {
                reason: review_verdict(report).2,
            },
        };
    }
    if attempts_used >= max_attempts {
        return GoalReviewOutcome::Escalated {
            attempts: attempts_used,
            findings,
        };
    }
    GoalReviewOutcome::Repair {
        attempt: attempts_used + 1,
        remaining: max_attempts - attempts_used - 1,
        findings,
    }
}

/// Whether a parent samples this child's claimed-complete work for review.
pub fn spot_check_selected(percent: usize, roll: u64) -> bool {
    percent > 0 && roll % 100 < percent as u64
}

pub fn spot_check_child(settings: &GoalReviewSettings) -> bool {
    settings.enabled && spot_check_selected(settings.spot_check_percent, rand::random::<u64>())
}

#[async_trait]
pub trait GoalReviewer: Send {
    async fn review(&mut self, request: GoalReviewRequest) -> Result<ReviewReport, String>;

    /// What this reviewer is about to read and with what. A reviewer that cannot
    /// describe itself returns `None`, and then nothing it produces is ever
    /// reused: an unknown identity is not an unchanged one.
    async fn identity(&self, request: &GoalReviewRequest) -> Option<ReviewIdentity>;
}

async fn reuse_key<R: GoalReviewer>(
    reviewer: &R,
    request: &GoalReviewRequest,
) -> Option<EvidenceCacheKey> {
    let identity = reviewer.identity(request).await?;
    Some(
        ReviewReuseInputs {
            identity,
            what_to_check: request.what_to_check.clone(),
            plan: request.plan.clone(),
            base: request.base.clone(),
            files: request.files.clone(),
        }
        .key(),
    )
}

/// The goal path's review entry: obtain a report, record it as evidence, and
/// decide whether a bounded repair round is still available.
///
/// A goal that fails verification several times must not pay for the same review
/// of the same diff each time, so a report still describing the tree under review
/// is reused — visibly, never disguised as a fresh reading.
pub async fn review_goal_work<R: GoalReviewer>(
    settings: &GoalReviewSettings,
    reviewer: &mut R,
    request: GoalReviewRequest,
    attempts_used: usize,
) -> GoalReviewResult {
    let store = review_reuse_store();
    store.lock().await.set_limits(review_reuse_limits());
    review_goal_work_in(
        settings,
        reviewer,
        request,
        attempts_used,
        store,
        crate::chat::evidence_cache::now_ms(),
    )
    .await
}

pub async fn review_goal_work_in<R: GoalReviewer>(
    settings: &GoalReviewSettings,
    reviewer: &mut R,
    request: GoalReviewRequest,
    attempts_used: usize,
    store: &AMutex<ReviewReuseStore>,
    now_ms: i64,
) -> GoalReviewResult {
    if !settings.enabled {
        return GoalReviewResult {
            outcome: GoalReviewOutcome::Skipped {
                reason: GOAL_REVIEW_DISABLED.to_string(),
            },
            report: None,
            evidence: None,
        };
    }
    let spec = review_spec(&EvidenceDefaults::from_settings());
    let key = reuse_key(reviewer, &request).await;
    if let Some(key) = key.as_ref() {
        if let Some(reused) = store.lock().await.lookup(key, now_ms) {
            let outcome =
                decide_repair(&reused.report, attempts_used, settings.max_repair_attempts);
            return GoalReviewResult {
                outcome,
                report: Some(reused.report),
                evidence: Some((spec, reused.record)),
            };
        }
    }
    match reviewer.review(request).await {
        Ok(report) => {
            let record = review_evidence_record(&report, &spec);
            if let Some(key) = key {
                let reusable = ReusableReview {
                    report: report.clone(),
                    record: record.clone(),
                };
                store.lock().await.store(key, &reusable, now_ms);
            }
            let outcome = decide_repair(&report, attempts_used, settings.max_repair_attempts);
            GoalReviewResult {
                outcome,
                report: Some(report),
                evidence: Some((spec, record)),
            }
        }
        Err(error) => {
            let reason = format!("review could not run: {error}");
            let record = unexecuted_record(
                &spec,
                EvidenceVerdict::Unavailable,
                VerificationOutcome::InfrastructureFailed,
                reason.clone(),
            );
            GoalReviewResult {
                outcome: GoalReviewOutcome::Unavailable { reason },
                report: None,
                evidence: Some((spec, record)),
            }
        }
    }
}

/// The production reviewer: the same `run_review` the `review` tool calls, with
/// no tool-call surface in between.
pub struct PipelineReviewer {
    gcx: Arc<GlobalContext>,
    cfg: ReviewConfig,
    ctx: StageCtx,
    stage_drain: tokio::task::JoinHandle<()>,
    stage_events_seen: Arc<AtomicU64>,
    pub metering: serde_json::Map<String, serde_json::Value>,
}

/// Stage progress is watched, never collected. A deep review emits progress for
/// as long as it runs, so the receiver is consumed as events arrive and only a
/// count survives; nothing is buffered for a caller that never reads it.
fn drain_stage_progress(
    mut events: UnboundedReceiver<serde_json::Value>,
    seen: Arc<AtomicU64>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while events.recv().await.is_some() {
            seen.fetch_add(1, Ordering::Relaxed);
        }
    })
}

pub struct GoalReviewScope {
    pub chat_id: String,
    pub root_chat_id: String,
    pub worktree: Option<WorktreeMeta>,
    pub task_meta: Option<TaskMeta>,
    pub abort_flag: Arc<AtomicBool>,
}

impl PipelineReviewer {
    pub async fn new(gcx: Arc<GlobalContext>, scope: GoalReviewScope) -> Result<Self, String> {
        let cfg = load_review_config(gcx.clone()).await?;
        let review_id = format!("gr-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let (stage_tx, stage_events) = unbounded_channel();
        let ctx = StageCtx {
            tool_call_id: review_id.clone(),
            review_id,
            subchat_tx: Arc::new(AMutex::new(stage_tx)),
            abort_flag: scope.abort_flag,
            depth: 0,
            task_meta: scope.task_meta,
            worktree: scope.worktree,
            chat_id: scope.chat_id,
            root_chat_id: scope.root_chat_id,
            // Stamped per stage by `monitor_ctx`; the pipeline-level ctx is
            // not itself the liveness source.
            activity: None,
            review_scope: None,
        };
        let stage_events_seen = Arc::new(AtomicU64::new(0));
        let stage_drain = drain_stage_progress(stage_events, stage_events_seen.clone());
        Ok(Self {
            gcx,
            cfg,
            ctx,
            stage_drain,
            stage_events_seen,
            metering: serde_json::Map::new(),
        })
    }

    pub fn stage_events_seen(&self) -> u64 {
        self.stage_events_seen.load(Ordering::Relaxed)
    }

    /// The tree this reviewer would read. Without one there is no revision, and
    /// without a revision nothing is reused.
    async fn review_root(&self) -> Option<PathBuf> {
        if let Some(worktree) = self.ctx.worktree.as_ref() {
            return Some(worktree.root.clone());
        }
        crate::files_correction::get_project_dirs(self.gcx.clone())
            .await
            .first()
            .cloned()
    }
}

impl Drop for PipelineReviewer {
    fn drop(&mut self) {
        self.stage_drain.abort();
    }
}

#[async_trait]
impl GoalReviewer for PipelineReviewer {
    async fn review(&mut self, request: GoalReviewRequest) -> Result<ReviewReport, String> {
        run_review(
            self.gcx.clone(),
            self.ctx.clone(),
            self.cfg.clone(),
            request.into_review_args(),
            &mut self.metering,
            None,
        )
        .await
    }

    async fn identity(&self, request: &GoalReviewRequest) -> Option<ReviewIdentity> {
        let root = self.review_root().await?;
        let depth = self.cfg.settings.depth_or_default(None);
        let selection = select_stages(
            load_stage_catalog(self.gcx.clone()).await,
            depth,
            None,
            &request.files,
        )
        .ok()?;
        let mut reviewer_models = Vec::new();
        for slot in self
            .cfg
            .settings
            .slots_for_variants(self.cfg.settings.variants)
        {
            reviewer_models.push(slot_model_id(self.gcx.clone(), slot).await.ok()?);
        }
        Some(ReviewIdentity {
            revision: review_revision(&root, request.base.as_deref()).await,
            reviewer_models,
            stages: selection
                .scheduled
                .into_iter()
                .map(|spec| spec.id)
                .collect(),
            policy: review_policy_fingerprint(&self.cfg),
        })
    }
}

/// Builds a reviewer from a live session. The session lock is taken only to
/// snapshot the scope and is dropped before any model call is made, so a review
/// never runs inside a held session lock.
pub async fn reviewer_for_session(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<AMutex<ChatSession>>,
) -> Result<PipelineReviewer, String> {
    let scope = {
        let session = session_arc.lock().await;
        GoalReviewScope {
            chat_id: session.chat_id.clone(),
            root_chat_id: session
                .thread
                .root_chat_id
                .clone()
                .unwrap_or_else(|| session.chat_id.clone()),
            worktree: session.thread.worktree.clone(),
            task_meta: session.thread.task_meta.clone(),
            abort_flag: session.abort_flag.clone(),
        }
    };
    PipelineReviewer::new(gcx, scope).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::evidence::EvidenceService;
    use crate::chat::verify_cmd::VerifyCommandPolicy;
    use crate::tools::review_types::{Dispute, ReviewDiffSummary, ReviewScopeSummary, StageRun};

    const REVIEWER_MODEL: &str = "reviewer-model-b";

    fn defaults() -> EvidenceDefaults {
        EvidenceDefaults {
            timeout_secs: 60,
            output_cap_bytes: 4_000,
        }
    }

    fn fact(id: &str, severity: ReviewSeverity) -> ReviewFinding {
        ReviewFinding {
            id: id.to_string(),
            stage: "diff".to_string(),
            model: Some(REVIEWER_MODEL.to_string()),
            title: "Error dropped".to_string(),
            severity,
            file: "src/lib.rs".to_string(),
            line_start: 10,
            line_end: 14,
            claim: "The error arm returns success.".to_string(),
            evidence: "return Ok(());".to_string(),
            evidence_present: true,
            reproduction: Some("cargo test -p thing".to_string()),
            fix: Some("Propagate the error.".to_string()),
            introduced_by_diff: true,
            out_of_scope: false,
            reported_by: vec!["diff".to_string()],
            locations: vec![],
            disputed: None,
        }
    }

    fn hypothesis(id: &str, severity: ReviewSeverity) -> ReviewFinding {
        let mut finding = fact(id, severity);
        finding.reproduction = None;
        finding.evidence_present = false;
        finding
    }

    fn report(findings: Vec<ReviewFinding>, stages: Vec<StageRun>) -> ReviewReport {
        let mut report = ReviewReport {
            depth: "normal".to_string(),
            outcome: ReviewOutcome::Inconclusive,
            scope: ReviewScopeSummary::default(),
            diff: ReviewDiffSummary::default(),
            stages,
            findings,
            duration_ms: 1_000,
            duplicates_merged: 0,
            scratch_dir: None,
        };
        report.outcome = report.derive_outcome();
        report
    }

    fn reviewed(findings: Vec<ReviewFinding>) -> ReviewReport {
        report(
            findings,
            vec![
                StageRun::ok("mechanical", Some(REVIEWER_MODEL.to_string()), 10),
                StageRun::ok("spec", Some(REVIEWER_MODEL.to_string()), 20),
            ],
        )
    }

    fn inconclusive() -> ReviewReport {
        report(
            vec![],
            vec![
                StageRun::timed_out("mechanical", None, 10, "no activity for 360s"),
                StageRun::failed("spec", None, 20, "output_contract"),
            ],
        )
    }

    fn partial() -> ReviewReport {
        report(
            vec![],
            vec![
                StageRun::ok("mechanical", Some(REVIEWER_MODEL.to_string()), 10),
                StageRun::timed_out("spec", None, 20, "no activity for 360s"),
            ],
        )
    }

    struct FakeReviewer {
        report: Result<ReviewReport, String>,
        identity: Option<ReviewIdentity>,
        requests: Vec<GoalReviewRequest>,
    }

    #[async_trait]
    impl GoalReviewer for FakeReviewer {
        async fn review(&mut self, request: GoalReviewRequest) -> Result<ReviewReport, String> {
            self.requests.push(request);
            self.report.clone()
        }

        async fn identity(&self, _request: &GoalReviewRequest) -> Option<ReviewIdentity> {
            self.identity.clone()
        }
    }

    /// A reviewer that cannot describe what it is about to read, so nothing it
    /// produces is ever reused.

    fn identity() -> ReviewIdentity {
        ReviewIdentity {
            revision: "rev-1".to_string(),
            reviewer_models: vec![REVIEWER_MODEL.to_string()],
            stages: vec!["mechanical".to_string(), "spec".to_string()],
            policy: "policy-1".to_string(),
        }
    }

    #[test]
    fn a_hypothesis_never_blocks_but_a_fact_can() {
        let hypotheses = reviewed(vec![
            hypothesis("rf-1", ReviewSeverity::Blocker),
            hypothesis("rf-2", ReviewSeverity::High),
        ]);
        assert!(blocking_findings(&hypotheses).is_empty());
        assert_eq!(review_verdict(&hypotheses).0, EvidenceVerdict::Passed);
        assert_eq!(decide_repair(&hypotheses, 0, 2), GoalReviewOutcome::Clean);

        let mut disputed = fact("rf-3", ReviewSeverity::Blocker);
        disputed.disputed = Some(Dispute {
            stage: "adversarial".to_string(),
            reason: "guarded by an earlier check".to_string(),
        });
        let disputed = reviewed(vec![disputed]);
        assert!(blocking_findings(&disputed).is_empty());
        assert_eq!(review_verdict(&disputed).0, EvidenceVerdict::Passed);

        let facts = reviewed(vec![
            fact("rf-4", ReviewSeverity::Blocker),
            fact("rf-5", ReviewSeverity::Note),
        ]);
        let blocking = blocking_findings(&facts);
        assert_eq!(blocking.len(), 1);
        assert_eq!(blocking[0].id, "rf-4");
        let (verdict, outcome, reason) = review_verdict(&facts);
        assert_eq!(verdict, EvidenceVerdict::Failed);
        assert_eq!(outcome, VerificationOutcome::CommandFailed);
        assert!(reason.contains("rf-4"), "{reason}");
        assert!(!reason.contains("rf-5"), "{reason}");
    }

    #[tokio::test]
    async fn the_review_spec_is_evidence_and_is_never_executed_as_a_command() {
        let temp = tempfile::tempdir().unwrap();
        let spec = review_spec(&defaults());
        assert!(!spec.is_accepted());
        assert!(spec.argv.is_empty());
        assert!(spec.required);

        let rejection = spec
            .clone()
            .accept(&VerifyCommandPolicy::permissive(), "test")
            .unwrap_err();

        assert_eq!(rejection.verdict, EvidenceVerdict::Blocked);
        assert!(rejection.reason.contains("prose"), "{}", rejection.reason);

        let service = EvidenceService::new(
            VerifyCommandPolicy::permissive(),
            crate::chat::evidence::EvidenceContext {
                worktree: temp.path().to_path_buf(),
                commit: None,
                scope: "chat".to_string(),
            },
        );
        let mut runner = crate::chat::evidence::SystemEvidenceRunner {
            gcx: crate::global_context::tests::make_test_gcx().await,
        };
        let blocked = service.accept_and_execute(spec, &mut runner).await;
        assert_eq!(blocked.verdict, EvidenceVerdict::Blocked);
    }
}
