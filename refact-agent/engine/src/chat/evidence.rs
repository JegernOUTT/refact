//! Executable verification contracts and the single evidence service that runs them.
//!
//! A `VerificationSpec` is a structured contract: what to execute, where, with which
//! environment, what exit codes and artifacts prove it, and which authority accepted it.
//! Prose (a `verify_hint`, an acceptance-criteria sentence) is never a spec and is never
//! executed. Execution itself is delegated to `chat::verifier::run_verification_argv_impl`,
//! the single confined exec path already shared by the board verifier and the post-merge
//! runner, so this service adds no second way to run a command.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::chat::evidence_cache::CacheDeclaration;
use crate::chat::evidence_revision::ContentRevision;
use crate::chat::types::{CriterionEvidence, CriterionVerdict, GoalCriterion, VerdictSource};
use crate::chat::verify_cmd::{
    instruction_commands, parse_verification_argv, unparseable_proposal_prose_reason,
    validate_command_hint, validate_read_only_proposal, VerifyCommandPolicy,
};
use crate::global_context::GlobalContext;
use crate::tasks::types::{BoardCard, VerificationOutcome, VerificationResult};

pub const EVIDENCE_POLICY_AUTHORITY: &str = "chat.evidence.policy";

/// The verdict of one gate. These six states are deliberately distinct: only
/// `Passed` ever means the evidence was obtained and satisfied the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceVerdict {
    #[default]
    NotRun,
    Passed,
    Failed,
    Blocked,
    Unavailable,
    Superseded,
}

impl EvidenceVerdict {
    /// Every verdict the engine can report, in declaration order. Surfaces that
    /// carry the vocabulary are generated from or pinned to this list so a
    /// seventh verdict cannot reach one surface and be silently absent on another.
    pub const ALL: [EvidenceVerdict; 6] = [
        EvidenceVerdict::NotRun,
        EvidenceVerdict::Passed,
        EvidenceVerdict::Failed,
        EvidenceVerdict::Blocked,
        EvidenceVerdict::Unavailable,
        EvidenceVerdict::Superseded,
    ];

    pub fn is_passed(self) -> bool {
        matches!(self, EvidenceVerdict::Passed)
    }

    pub fn label(self) -> &'static str {
        match self {
            EvidenceVerdict::NotRun => "not_run",
            EvidenceVerdict::Passed => "passed",
            EvidenceVerdict::Failed => "failed",
            EvidenceVerdict::Blocked => "blocked",
            EvidenceVerdict::Unavailable => "unavailable",
            EvidenceVerdict::Superseded => "superseded",
        }
    }

    /// A verdict that proves the contract was violated, as opposed to one that
    /// only proves the evidence could not be obtained.
    pub fn is_contract_violation(self) -> bool {
        matches!(self, EvidenceVerdict::Failed)
    }
}

/// Who authored the spec. Authorship is not authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "authority")]
pub enum SpecAuthority {
    CardInstructions,
    AgentProposal { agent: String },
    Operator { operator: String },
}

impl SpecAuthority {
    pub fn is_untrusted_proposal(&self) -> bool {
        matches!(self, SpecAuthority::AgentProposal { .. })
    }

    pub fn label(&self) -> String {
        match self {
            SpecAuthority::CardInstructions => "card_instructions".to_string(),
            SpecAuthority::AgentProposal { agent } => format!("agent_proposal:{agent}"),
            SpecAuthority::Operator { operator } => format!("operator:{operator}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecAcceptance {
    pub accepted_by: String,
    pub accepted_at: String,
    pub authority: SpecAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecRejection {
    pub verdict: EvidenceVerdict,
    pub outcome: VerificationOutcome,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "assertion", content = "value")]
pub enum ArtifactAssertion {
    Exists,
    NonEmpty,
    Contains(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedArtifact {
    pub path: String,
    pub assertion: ArtifactAssertion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCheck {
    pub path: String,
    pub assertion: ArtifactAssertion,
    pub satisfied: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceDefaults {
    pub timeout_secs: u64,
    pub output_cap_bytes: usize,
}

impl EvidenceDefaults {
    pub fn from_settings() -> Self {
        let settings = crate::runtime_settings::current();
        Self {
            timeout_secs: settings.verification_evidence_timeout_secs,
            output_cap_bytes: settings.verification_evidence_output_cap_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationSpec {
    pub spec_id: String,
    pub revision: u64,
    pub criterion_ids: Vec<String>,
    pub required: bool,
    pub command: String,
    pub proposed_by: SpecAuthority,
    pub accepted: Option<SpecAcceptance>,
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub env_allowlist: Vec<String>,
    pub timeout_secs: u64,
    pub output_cap_bytes: usize,
    pub expected_exit_codes: Vec<i32>,
    pub expected_artifacts: Vec<ExpectedArtifact>,
    pub toolchain_inputs: Vec<String>,
    pub cache: CacheDeclaration,
}

impl VerificationSpec {
    pub fn propose(
        spec_id: impl Into<String>,
        command: impl Into<String>,
        proposed_by: SpecAuthority,
        defaults: &EvidenceDefaults,
    ) -> Self {
        Self {
            spec_id: spec_id.into(),
            revision: 1,
            criterion_ids: Vec::new(),
            required: true,
            command: command.into(),
            proposed_by,
            accepted: None,
            argv: Vec::new(),
            cwd: None,
            env: Vec::new(),
            env_allowlist: Vec::new(),
            timeout_secs: defaults.timeout_secs,
            output_cap_bytes: defaults.output_cap_bytes,
            expected_exit_codes: vec![0],
            expected_artifacts: Vec::new(),
            toolchain_inputs: Vec::new(),
            cache: CacheDeclaration::default(),
        }
    }

    pub fn is_accepted(&self) -> bool {
        self.accepted.is_some() && !self.argv.is_empty()
    }

    /// Turn a proposal into an executable spec, or refuse it. An agent-authored
    /// proposal must additionally survive the prose check, so a sentence never
    /// becomes argv just because a model wrote it into a report.
    pub(crate) fn accept(
        mut self,
        policy: &VerifyCommandPolicy,
        accepted_by: &str,
    ) -> Result<Self, SpecRejection> {
        let parsed = parse_verification_argv(&self.command).map_err(|reason| SpecRejection {
            verdict: EvidenceVerdict::Blocked,
            outcome: VerificationOutcome::Rejected,
            reason: self
                .proposed_by
                .is_untrusted_proposal()
                .then(|| unparseable_proposal_prose_reason(&self.command))
                .flatten()
                .unwrap_or_else(|| format!("Cannot run as a command: {}", reason)),
        })?;
        if self.proposed_by.is_untrusted_proposal() {
            validate_command_hint(&parsed.argv).map_err(|reason| SpecRejection {
                verdict: EvidenceVerdict::Blocked,
                outcome: VerificationOutcome::Rejected,
                reason,
            })?;
            validate_read_only_proposal(&parsed.argv).map_err(|reason| SpecRejection {
                verdict: EvidenceVerdict::Blocked,
                outcome: VerificationOutcome::Rejected,
                reason,
            })?;
        }
        policy
            .check(&self.command)
            .map_err(|reason| SpecRejection {
                verdict: EvidenceVerdict::Blocked,
                outcome: VerificationOutcome::PolicyDenied,
                reason: format!("Denied by shell policy: {}", reason),
            })?;
        if self.env_allowlist.is_empty() {
            self.env_allowlist = parsed.env.iter().map(|(name, _)| name.clone()).collect();
        } else if let Some(name) = parsed
            .env
            .iter()
            .map(|(name, _)| name)
            .find(|name| !self.env_allowlist.contains(name))
        {
            return Err(SpecRejection {
                verdict: EvidenceVerdict::Blocked,
                outcome: VerificationOutcome::PolicyDenied,
                reason: format!(
                    "Environment variable '{}' is outside the spec allowlist",
                    name
                ),
            });
        }
        self.cwd = parsed.cwd;
        self.env = parsed.env;
        self.argv = parsed.argv;
        self.accepted = Some(SpecAcceptance {
            accepted_by: accepted_by.to_string(),
            accepted_at: Utc::now().to_rfc3339(),
            authority: self.proposed_by.clone(),
        });
        Ok(self)
    }
}

/// Where the evidence comes from. `scope` names *what* the evidence is about —
/// a card id, a chat — and is deliberately not a revision: whether the evidence
/// still describes the current content is decided by
/// [`crate::chat::evidence_revision::ContentRevision`], which moves only when
/// the evidence's own inputs move.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EvidenceContext {
    pub worktree: PathBuf,
    pub commit: Option<String>,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExecutionFingerprint {
    pub worktree: String,
    pub commit: Option<String>,
    pub content_revision: String,
    pub os: String,
    pub arch: String,
    pub program: String,
    pub env: BTreeMap<String, String>,
    pub toolchain: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub spec_id: String,
    pub spec_revision: u64,
    pub criterion_ids: Vec<String>,
    pub required: bool,
    pub verdict: EvidenceVerdict,
    pub outcome: VerificationOutcome,
    pub command: String,
    pub argv: Vec<String>,
    pub exit_code: Option<i32>,
    pub output_tail: String,
    pub artifacts: Vec<ArtifactCheck>,
    pub fingerprint: ExecutionFingerprint,
    /// The content this evidence depended on when it was produced. A later
    /// reader recomputes it and supersedes the record when it has moved.
    #[serde(default)]
    pub content_revision: ContentRevision,
    /// When the command actually ran. A replayed record keeps the original
    /// execution time here, never the replay time.
    pub started_at: String,
    pub finished_at: String,
    pub accepted_by: Option<String>,
    pub reason: Option<String>,
    /// True when this record was replayed from the evidence cache instead of
    /// executed. A consumer that ignores this reads a replay as a fresh run.
    pub from_cache: bool,
    /// When the replay happened, set only on a cache hit.
    pub replayed_at: Option<String>,
}

impl EvidenceRecord {
    pub fn to_verification_result(&self) -> VerificationResult {
        VerificationResult {
            command: self.command.clone(),
            exit_code: self.exit_code,
            passed: self.verdict.is_passed(),
            output_tail: self.output_tail.clone(),
            outcome: match self.verdict {
                EvidenceVerdict::Passed => VerificationOutcome::Passed,
                EvidenceVerdict::Failed => VerificationOutcome::CommandFailed,
                EvidenceVerdict::NotRun => VerificationOutcome::NoCommands,
                EvidenceVerdict::Unavailable | EvidenceVerdict::Superseded => {
                    VerificationOutcome::InfrastructureFailed
                }
                _ => self.outcome,
            },
            from_cache: self.from_cache,
            executed_at: Some(self.started_at.clone()),
        }
    }
}

/// Record for a spec that was never handed to the runner.
pub(crate) fn unexecuted_record(
    spec: &VerificationSpec,
    verdict: EvidenceVerdict,
    outcome: VerificationOutcome,
    reason: String,
) -> EvidenceRecord {
    let now = Utc::now().to_rfc3339();
    EvidenceRecord {
        spec_id: spec.spec_id.clone(),
        spec_revision: spec.revision,
        criterion_ids: spec.criterion_ids.clone(),
        required: spec.required,
        verdict,
        outcome,
        command: spec.command.clone(),
        argv: Vec::new(),
        exit_code: None,
        output_tail: reason.clone(),
        artifacts: Vec::new(),
        fingerprint: ExecutionFingerprint::default(),
        content_revision: ContentRevision::declared(spec),
        started_at: now.clone(),
        finished_at: now,
        accepted_by: spec
            .accepted
            .as_ref()
            .map(|acceptance| acceptance.accepted_by.clone()),
        reason: Some(reason),
        from_cache: false,
        replayed_at: None,
    }
}

#[async_trait]
pub trait EvidenceRunner: Send {
    async fn run(&mut self, worktree: &Path, spec: &VerificationSpec) -> VerificationResult;
}

/// Delegates to the one confined exec path the engine already uses.
pub struct SystemEvidenceRunner {
    pub gcx: Arc<GlobalContext>,
}

#[async_trait]
impl EvidenceRunner for SystemEvidenceRunner {
    async fn run(&mut self, worktree: &Path, spec: &VerificationSpec) -> VerificationResult {
        crate::chat::verifier::run_verification_argv_impl(
            self.gcx.clone(),
            worktree,
            &spec.command,
            spec.cwd.clone(),
            spec.env.clone(),
            spec.argv.clone(),
            Duration::from_secs(spec.timeout_secs),
            crate::chat::verifier::DRAIN_TIMEOUT,
        )
        .await
    }
}

pub struct EvidenceService {
    policy: VerifyCommandPolicy,
    context: EvidenceContext,
}

impl EvidenceService {
    pub fn new(policy: VerifyCommandPolicy, context: EvidenceContext) -> Self {
        Self { policy, context }
    }

    /// Execute an already-accepted spec. An unaccepted proposal is blocked here and
    /// never reaches the runner.
    pub async fn execute<R: EvidenceRunner>(
        &self,
        spec: &VerificationSpec,
        runner: &mut R,
    ) -> EvidenceRecord {
        if !spec.is_accepted() {
            return unexecuted_record(
                spec,
                EvidenceVerdict::Blocked,
                VerificationOutcome::Rejected,
                format!(
                    "Spec proposed by {} has not been accepted under policy",
                    spec.proposed_by.label()
                ),
            );
        }
        let started_at = Utc::now().to_rfc3339();
        let result = runner.run(&self.context.worktree, spec).await;
        let finished_at = Utc::now().to_rfc3339();
        let artifacts = self.check_artifacts(spec).await;
        let (verdict, reason) = verdict_for(spec, &result, &artifacts);
        let content_revision =
            ContentRevision::observe(&self.context.worktree, self.context.commit.as_deref(), spec)
                .await;
        EvidenceRecord {
            spec_id: spec.spec_id.clone(),
            spec_revision: spec.revision,
            criterion_ids: spec.criterion_ids.clone(),
            required: spec.required,
            verdict,
            outcome: result.outcome,
            command: result.command.clone(),
            argv: spec.argv.clone(),
            exit_code: result.exit_code,
            output_tail: tail_chars(&result.output_tail, spec.output_cap_bytes),
            artifacts,
            fingerprint: self.fingerprint(spec, &content_revision),
            content_revision,
            started_at,
            finished_at,
            accepted_by: spec
                .accepted
                .as_ref()
                .map(|acceptance| acceptance.accepted_by.clone()),
            reason,
            from_cache: false,
            replayed_at: None,
        }
    }

    /// The tree this service's evidence is about.
    pub fn worktree(&self) -> &Path {
        &self.context.worktree
    }

    /// The verification policy in force, as a value a cache key can carry.
    pub fn policy_fingerprint(&self) -> String {
        self.policy.fingerprint()
    }

    pub fn accept(&self, spec: VerificationSpec) -> Result<VerificationSpec, SpecRejection> {
        spec.accept(&self.policy, EVIDENCE_POLICY_AUTHORITY)
    }

    /// Accept a proposal under policy, then execute it. Refusal produces a blocked
    /// record instead of an execution.
    pub async fn accept_and_execute<R: EvidenceRunner>(
        &self,
        spec: VerificationSpec,
        runner: &mut R,
    ) -> EvidenceRecord {
        if spec.is_accepted() {
            return self.execute(&spec, runner).await;
        }
        let proposal = spec.clone();
        match self.accept(spec) {
            Ok(accepted) => self.execute(&accepted, runner).await,
            Err(rejection) => unexecuted_record(
                &proposal,
                rejection.verdict,
                rejection.outcome,
                rejection.reason,
            ),
        }
    }

    pub async fn accept_and_execute_all<R: EvidenceRunner>(
        &self,
        specs: Vec<VerificationSpec>,
        runner: &mut R,
    ) -> Vec<EvidenceRecord> {
        let mut records = Vec::with_capacity(specs.len());
        for spec in specs {
            records.push(self.accept_and_execute(spec, runner).await);
        }
        records
    }

    fn fingerprint(
        &self,
        spec: &VerificationSpec,
        revision: &ContentRevision,
    ) -> ExecutionFingerprint {
        ExecutionFingerprint {
            worktree: self.context.worktree.to_string_lossy().to_string(),
            commit: self.context.commit.clone(),
            content_revision: revision.digest(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            program: spec.argv.first().cloned().unwrap_or_default(),
            env: spec
                .env
                .iter()
                .filter(|(name, _)| spec.env_allowlist.contains(name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            toolchain: spec.toolchain_inputs.clone(),
        }
    }

    async fn check_artifacts(&self, spec: &VerificationSpec) -> Vec<ArtifactCheck> {
        let mut checks = Vec::with_capacity(spec.expected_artifacts.len());
        for artifact in &spec.expected_artifacts {
            checks.push(self.check_artifact(spec, artifact).await);
        }
        checks
    }

    async fn check_artifact(
        &self,
        spec: &VerificationSpec,
        artifact: &ExpectedArtifact,
    ) -> ArtifactCheck {
        let unsatisfied = |detail: String| ArtifactCheck {
            path: artifact.path.clone(),
            assertion: artifact.assertion.clone(),
            satisfied: false,
            detail,
        };
        let path = match resolve_artifact_path(&self.context.worktree, spec, &artifact.path) {
            Ok(path) => path,
            Err(reason) => return unsatisfied(reason),
        };
        match &artifact.assertion {
            ArtifactAssertion::Exists => match tokio::fs::metadata(&path).await {
                Ok(_) => ArtifactCheck {
                    path: artifact.path.clone(),
                    assertion: artifact.assertion.clone(),
                    satisfied: true,
                    detail: "artifact exists".to_string(),
                },
                Err(error) => unsatisfied(format!("artifact is missing: {}", error)),
            },
            ArtifactAssertion::NonEmpty => match tokio::fs::metadata(&path).await {
                Ok(metadata) if metadata.len() > 0 => ArtifactCheck {
                    path: artifact.path.clone(),
                    assertion: artifact.assertion.clone(),
                    satisfied: true,
                    detail: format!("artifact holds {} bytes", metadata.len()),
                },
                Ok(_) => unsatisfied("artifact is empty".to_string()),
                Err(error) => unsatisfied(format!("artifact is missing: {}", error)),
            },
            ArtifactAssertion::Contains(needle) => match tokio::fs::read_to_string(&path).await {
                Ok(contents) if contents.contains(needle.as_str()) => ArtifactCheck {
                    path: artifact.path.clone(),
                    assertion: artifact.assertion.clone(),
                    satisfied: true,
                    detail: format!("artifact contains '{}'", needle),
                },
                Ok(_) => unsatisfied(format!("artifact does not contain '{}'", needle)),
                Err(error) => unsatisfied(format!("artifact is unreadable: {}", error)),
            },
        }
    }
}

pub(crate) fn resolve_artifact_path(
    worktree: &Path,
    spec: &VerificationSpec,
    artifact: &str,
) -> Result<PathBuf, String> {
    let relative = Path::new(artifact);
    if artifact.trim().is_empty() {
        return Err("artifact path is empty".to_string());
    }
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err("artifact path must stay within the worktree".to_string());
    }
    let mut path = worktree.to_path_buf();
    if let Some(cwd) = spec.cwd.as_ref() {
        path.push(cwd);
    }
    path.push(relative);
    Ok(path)
}

fn verdict_for(
    spec: &VerificationSpec,
    result: &VerificationResult,
    artifacts: &[ArtifactCheck],
) -> (EvidenceVerdict, Option<String>) {
    match result.outcome {
        VerificationOutcome::Rejected | VerificationOutcome::PolicyDenied => (
            EvidenceVerdict::Blocked,
            Some(result.output_tail.trim().to_string()),
        ),
        VerificationOutcome::InfrastructureFailed => (
            EvidenceVerdict::Unavailable,
            Some(result.output_tail.trim().to_string()),
        ),
        VerificationOutcome::NoCommands => (
            EvidenceVerdict::NotRun,
            Some("no command was executed".to_string()),
        ),
        VerificationOutcome::Passed
        | VerificationOutcome::CommandFailed
        | VerificationOutcome::Unknown => match result.exit_code {
            None => (
                EvidenceVerdict::Unavailable,
                Some("command produced no exit status".to_string()),
            ),
            Some(code) if !spec.expected_exit_codes.contains(&code) => (
                EvidenceVerdict::Failed,
                Some(format!(
                    "exit code {} is not one of the expected {:?}",
                    code, spec.expected_exit_codes
                )),
            ),
            Some(_) => match artifacts.iter().find(|check| !check.satisfied) {
                Some(check) => (
                    EvidenceVerdict::Failed,
                    Some(format!("artifact '{}': {}", check.path, check.detail)),
                ),
                None => (EvidenceVerdict::Passed, None),
            },
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateBlock {
    pub spec_id: String,
    pub command: String,
    pub verdict: EvidenceVerdict,
    pub exit_code: Option<i32>,
    pub reason: String,
    /// True when the blocking evidence was replayed rather than executed.
    pub from_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GateOutcome {
    pub required_total: usize,
    pub required_passed: usize,
    pub blocks: Vec<GateBlock>,
}

impl GateOutcome {
    pub fn completion_allowed(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The semantic judge only runs once every required gate has already passed.
    pub fn judge_allowed(&self) -> bool {
        self.completion_allowed()
    }

    pub fn reasons(&self) -> Vec<String> {
        self.blocks
            .iter()
            .map(|block| block.reason.clone())
            .collect()
    }

    pub fn has_contract_violation(&self) -> bool {
        self.blocks
            .iter()
            .any(|block| block.verdict.is_contract_violation())
    }
}

/// Decide, from evidence alone, whether a required gate stands in the way.
pub fn evaluate_gates(specs: &[VerificationSpec], evidence: &[EvidenceRecord]) -> GateOutcome {
    let mut outcome = GateOutcome::default();
    for spec in specs.iter().filter(|spec| spec.required) {
        outcome.required_total += 1;
        let record = evidence
            .iter()
            .find(|record| record.spec_id == spec.spec_id && record.required);
        let Some(record) = record else {
            crate::coordination::metrics::record_verdict(EvidenceVerdict::NotRun);
            outcome.blocks.push(GateBlock {
                spec_id: spec.spec_id.clone(),
                command: spec.command.clone(),
                verdict: EvidenceVerdict::NotRun,
                exit_code: None,
                reason: format!(
                    "not run: Verification command never ran; no verifier evidence for {}",
                    spec.command
                ),
                from_cache: false,
            });
            continue;
        };
        crate::coordination::metrics::record_evidence_cache(record.from_cache);
        if record.spec_revision != spec.revision {
            crate::coordination::metrics::record_verdict(EvidenceVerdict::Superseded);
            outcome.blocks.push(GateBlock {
                spec_id: spec.spec_id.clone(),
                command: spec.command.clone(),
                verdict: EvidenceVerdict::Superseded,
                exit_code: record.exit_code,
                reason: format!(
                    "Verification evidence superseded: {} was verified at revision {}, spec is at {}",
                    spec.command, record.spec_revision, spec.revision
                ),
                from_cache: record.from_cache,
            });
            continue;
        }
        crate::coordination::metrics::record_verdict(record.verdict);
        match record.verdict {
            EvidenceVerdict::Passed => outcome.required_passed += 1,
            verdict => outcome.blocks.push(GateBlock {
                spec_id: spec.spec_id.clone(),
                command: spec.command.clone(),
                verdict,
                exit_code: record.exit_code,
                reason: gate_reason(
                    verdict,
                    &spec.command,
                    record.exit_code,
                    record.reason.as_deref(),
                    record,
                ),
                from_cache: record.from_cache,
            }),
        }
    }
    if outcome.required_total == 0 {
        outcome.blocks.push(GateBlock {
            spec_id: String::new(),
            command: String::new(),
            verdict: EvidenceVerdict::NotRun,
            exit_code: None,
            reason: "not run: No verification commands found in planner instructions".to_string(),
            from_cache: false,
        });
    }
    outcome
}

/// A replayed verdict says what an earlier run found, never that a command just ran.
pub fn replay_note(record: &EvidenceRecord) -> Option<String> {
    match record.from_cache {
        true => Some(format!(
            "replayed from cache, not re-executed; originally executed at {}",
            record.started_at
        )),
        false => None,
    }
}

fn gate_reason(
    verdict: EvidenceVerdict,
    command: &str,
    exit_code: Option<i32>,
    detail: Option<&str>,
    record: &EvidenceRecord,
) -> String {
    let detail = detail.map(str::trim).filter(|detail| !detail.is_empty());
    let head = match verdict {
        EvidenceVerdict::Failed => format!("Verification command failed: {}", command),
        EvidenceVerdict::Blocked => format!("not run: Verification command blocked: {}", command),
        EvidenceVerdict::Unavailable => {
            format!("not run: Verification evidence unavailable: {}", command)
        }
        EvidenceVerdict::Superseded => {
            format!("Verification evidence superseded: {}", command)
        }
        EvidenceVerdict::NotRun => format!("not run: verification command {}", command),
        EvidenceVerdict::Passed => format!("Verification command passed: {}", command),
    };
    let exit = match exit_code {
        Some(code) => format!(" (exit code {})", code),
        None => " (no exit code)".to_string(),
    };
    let detail = match (detail, replay_note(record)) {
        (Some(detail), Some(replay)) => Some(format!("{} — {}", detail, replay)),
        (Some(detail), None) => Some(detail.to_string()),
        (None, replay) => replay,
    };
    match detail {
        Some(detail) => format!("{}{} [{}] — {}", head, exit, verdict.label(), detail),
        None => format!("{}{} [{}]", head, exit, verdict.label()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionDecision {
    pub allowed: bool,
    pub reported_success: bool,
    pub reasons: Vec<String>,
}

/// What an agent reported about itself has no weight here: only gates decide.
pub fn completion_decision(gates: &GateOutcome, reported_success: bool) -> CompletionDecision {
    let decision = CompletionDecision {
        allowed: gates.completion_allowed(),
        reported_success,
        reasons: gates.reasons(),
    };
    explain_completion(gates, &decision);
    decision
}

/// The explanation behind a completion or a refusal: the gate tally that
/// produced it, the first blocking verdict, and the rule that fired.
fn explain_completion(gates: &GateOutcome, decision: &CompletionDecision) {
    use crate::coordination::metrics::{explain, Decision, Explanation};
    let mut inputs = vec![
        (
            "reason".to_string(),
            format!(
                "{} of {} required gates passed",
                gates.required_passed, gates.required_total
            ),
        ),
        (
            "source".to_string(),
            format!("agent reported success={}", decision.reported_success),
        ),
    ];
    if let Some(block) = gates.blocks.first() {
        inputs.push(("verdict".to_string(), block.verdict.label().to_string()));
        inputs.push(("outcome".to_string(), block.reason.clone()));
    }
    let (kind, outcome, rule) = match decision.allowed {
        true => (
            Decision::GoalCompletion,
            "allowed",
            "evidence.gates.all_required_passed",
        ),
        false => (
            Decision::GoalRefusal,
            "refused",
            "evidence.gates.required_gate_blocks",
        ),
    };
    explain(Explanation::new(kind, outcome, rule, inputs));
}

pub(crate) fn validate_session_goal_verify_commands(
    session: &crate::chat::types::ChatSession,
    policy: &VerifyCommandPolicy,
) -> Result<(), String> {
    let projection = session.try_accepted_control_projection()?;
    if let Some(goal) =
        crate::chat::goal_snapshot_from_messages(&projection.messages, projection.goal.as_ref())
    {
        validate_goal_verify_commands(&goal.criteria, policy)?;
    }
    Ok(())
}

pub(crate) fn validate_goal_verify_commands(
    criteria: &[GoalCriterion],
    policy: &VerifyCommandPolicy,
) -> Result<(), String> {
    let defaults = EvidenceDefaults::from_settings();
    for criterion in criteria {
        let Some(command) = &criterion.verify_command else {
            continue;
        };
        VerificationSpec::propose(
            format!("criterion:{}", criterion.id),
            command,
            SpecAuthority::AgentProposal {
                agent: "goal author".to_string(),
            },
            &defaults,
        )
        .accept(policy, EVIDENCE_POLICY_AUTHORITY)
        .map_err(|rejection| {
            format!(
                "criterion `{}` verify_command: {}",
                criterion.id, rejection.reason
            )
        })?;
    }
    Ok(())
}

/// Specs the goal's criteria carry. `verify_hint` is prose for the judge and is
/// deliberately not read here, so a hint can never become argv.
pub fn specs_from_criteria(
    criteria: &[GoalCriterion],
    author: &str,
    defaults: &EvidenceDefaults,
) -> Vec<VerificationSpec> {
    criteria
        .iter()
        .filter_map(|criterion| {
            let command = criterion
                .verify_command
                .as_deref()
                .map(str::trim)
                .filter(|command| !command.is_empty())?;
            let mut spec = VerificationSpec::propose(
                format!("criterion:{}", criterion.id),
                command,
                SpecAuthority::AgentProposal {
                    agent: author.to_string(),
                },
                defaults,
            );
            spec.criterion_ids = vec![criterion.id.clone()];
            Some(spec)
        })
        .collect()
}

/// Gate evaluation for a goal. A goal that declares no executable contract keeps its
/// previous semantics — the judge decides it — so declaring nothing never blocks a goal
/// the way it blocks a card.
pub fn evaluate_goal_gates(specs: &[VerificationSpec], evidence: &[EvidenceRecord]) -> GateOutcome {
    if !specs.iter().any(|spec| spec.required) {
        return GateOutcome::default();
    }
    evaluate_gates(specs, evidence)
}

/// Criterion verdicts that executed evidence decided. Everything absent from this list
/// is left to the judge and stays labelled `judged`.
pub fn measured_criterion_verdicts(
    specs: &[VerificationSpec],
    evidence: &[EvidenceRecord],
) -> Vec<CriterionVerdict> {
    let mut verdicts = Vec::new();
    for record in evidence {
        let Some(spec) = specs
            .iter()
            .find(|spec| spec.spec_id == record.spec_id && spec.revision == record.spec_revision)
        else {
            continue;
        };
        let reference = CriterionEvidence {
            spec_id: record.spec_id.clone(),
            command: record.command.clone(),
            exit_code: record.exit_code,
            verdict: record.verdict.label().to_string(),
        };
        for criterion_id in &spec.criterion_ids {
            verdicts.push(CriterionVerdict {
                id: criterion_id.clone(),
                met: record.verdict.is_passed(),
                note: evidence_note(record),
                source: VerdictSource::Measured,
                evidence: Some(reference.clone()),
            });
        }
    }
    verdicts
}

pub fn evidence_note(record: &EvidenceRecord) -> String {
    match record.exit_code {
        Some(code) => format!(
            "{} exited {} [{}]",
            record.command,
            code,
            record.verdict.label()
        ),
        None => format!(
            "{} produced no exit code [{}]",
            record.command,
            record.verdict.label()
        ),
    }
}

/// A card's verification contracts. The card's declared `target_files` are the only
/// inputs its evidence may be cached on: a card that declares none is re-run every
/// time rather than keyed on inputs nobody named.
pub fn specs_from_card(card: &BoardCard, defaults: &EvidenceDefaults) -> Vec<VerificationSpec> {
    let mut specs = instruction_specs(&card.id, &card.instructions, defaults);
    for spec in specs.iter_mut() {
        spec.cache.deterministic_inputs = card.target_files.clone();
    }
    specs
}

/// Planner-authored contracts recovered from instruction prose.
pub(crate) fn instruction_specs(
    card_id: &str,
    instructions: &str,
    defaults: &EvidenceDefaults,
) -> Vec<VerificationSpec> {
    let mut specs: Vec<VerificationSpec> = Vec::new();
    for command in instruction_commands(instructions) {
        let spec_id = format!("{}#{}", card_id, specs.len());
        specs.push(VerificationSpec::propose(
            spec_id,
            command,
            SpecAuthority::CardInstructions,
            defaults,
        ));
    }
    specs
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }
    text.chars().skip(len - max_chars).collect()
}

#[cfg(test)]
pub(crate) fn gate_outcome_for_verdicts(verdicts: &[(bool, EvidenceVerdict)]) -> GateOutcome {
    let defaults = EvidenceDefaults {
        timeout_secs: 60,
        output_cap_bytes: 4_000,
    };
    let mut specs = Vec::new();
    let mut records = Vec::new();
    for (index, (required, verdict)) in verdicts.iter().enumerate() {
        let mut spec = VerificationSpec::propose(
            format!("S-{index}"),
            "cargo test",
            SpecAuthority::CardInstructions,
            &defaults,
        );
        spec.required = *required;
        let mut record = unexecuted_record(
            &spec,
            *verdict,
            VerificationOutcome::Unknown,
            format!("verdict {}", verdict.label()),
        );
        record.exit_code = Some(if verdict.is_passed() { 0 } else { 1 });
        records.push(record);
        specs.push(spec);
    }
    evaluate_gates(&specs, &records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{FinalReport, ScopeGuardMode};

    #[derive(Default)]
    struct RecordingRunner {
        calls: Vec<(PathBuf, VerificationSpec)>,
        exit_code: Option<i32>,
        outcome: Option<VerificationOutcome>,
    }

    #[async_trait]
    impl EvidenceRunner for RecordingRunner {
        async fn run(&mut self, worktree: &Path, spec: &VerificationSpec) -> VerificationResult {
            self.calls.push((worktree.to_path_buf(), spec.clone()));
            let exit_code = self.exit_code.unwrap_or(0);
            VerificationResult {
                command: spec.command.clone(),
                exit_code: Some(exit_code),
                passed: exit_code == 0,
                output_tail: "runner output".to_string(),
                outcome: self.outcome.unwrap_or(VerificationOutcome::Unknown),
                from_cache: false,
                executed_at: None,
            }
        }
    }

    fn defaults() -> EvidenceDefaults {
        EvidenceDefaults {
            timeout_secs: 60,
            output_cap_bytes: 4_000,
        }
    }

    fn service(worktree: &Path) -> EvidenceService {
        EvidenceService::new(
            VerifyCommandPolicy::permissive(),
            EvidenceContext {
                worktree: worktree.to_path_buf(),
                commit: Some("abc1234".to_string()),
                scope: "T-1".to_string(),
            },
        )
    }

    fn proposal(command: &str, authority: SpecAuthority) -> VerificationSpec {
        VerificationSpec::propose("S-1", command, authority, &defaults())
    }

    fn card(instructions: &str) -> BoardCard {
        BoardCard {
            id: "T-1".to_string(),
            title: "Evidence card".to_string(),
            column: "done".to_string(),
            priority: "P1".to_string(),
            depends_on: Vec::new(),
            instructions: instructions.to_string(),
            assignee: None,
            agent_chat_id: Some("agent-1".to_string()),
            status_updates: Vec::new(),
            comments: Vec::new(),
            final_report: Some("done".to_string()),
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: Vec::new(),
            target_files: Vec::new(),
            scope_guard_mode: ScopeGuardMode::Off,
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_failing_required_gate_blocks_completion_even_when_the_report_claims_success() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner {
            exit_code: Some(1),
            outcome: Some(VerificationOutcome::CommandFailed),
            ..RecordingRunner::default()
        };
        let spec = proposal("cargo test --lib", SpecAuthority::CardInstructions);

        let record = service(temp.path())
            .accept_and_execute(spec.clone(), &mut runner)
            .await;
        let gates = evaluate_gates(&[spec], &[record.clone()]);
        let decision = completion_decision(&gates, true);

        assert_eq!(record.verdict, EvidenceVerdict::Failed);
        assert!(!gates.completion_allowed());
        assert!(!gates.judge_allowed());
        assert!(
            !decision.allowed,
            "a model claiming success must not defeat a failing gate"
        );
        assert!(decision.reported_success);
        assert!(decision.reasons[0].contains("cargo test --lib"));
    }

    #[tokio::test]
    async fn a_required_gate_that_never_ran_blocks_completion() {
        let spec = proposal("cargo test --lib", SpecAuthority::CardInstructions);

        let gates = evaluate_gates(&[spec], &[]);

        assert_eq!(gates.required_total, 1);
        assert_eq!(gates.required_passed, 0);
        assert_eq!(gates.blocks[0].verdict, EvidenceVerdict::NotRun);
        assert!(!gates.completion_allowed());
        assert!(!completion_decision(&gates, true).allowed);
    }

    #[test]
    fn declaring_no_required_spec_is_not_a_pass() {
        let gates = evaluate_gates(&[], &[]);

        assert!(!gates.completion_allowed());
        assert!(!gates.has_contract_violation());
        assert!(gates.reasons()[0].contains("No verification commands found"));
    }

    const ALL_VERDICTS: [EvidenceVerdict; 6] = [
        EvidenceVerdict::NotRun,
        EvidenceVerdict::Passed,
        EvidenceVerdict::Failed,
        EvidenceVerdict::Blocked,
        EvidenceVerdict::Unavailable,
        EvidenceVerdict::Superseded,
    ];

    #[test]
    fn all_six_verdicts_are_representable_and_distinguishable() {
        let labels: Vec<&str> = ALL_VERDICTS.iter().map(|verdict| verdict.label()).collect();
        assert_eq!(
            labels,
            vec![
                "not_run",
                "passed",
                "failed",
                "blocked",
                "unavailable",
                "superseded"
            ]
        );
        let unique: std::collections::BTreeSet<&str> = labels.iter().copied().collect();
        assert_eq!(unique.len(), 6);
        for verdict in ALL_VERDICTS {
            let json = serde_json::to_string(&verdict).unwrap();
            assert_eq!(
                serde_json::from_str::<EvidenceVerdict>(&json).unwrap(),
                verdict
            );
            assert_eq!(verdict.is_passed(), verdict == EvidenceVerdict::Passed);
        }
        assert_eq!(EvidenceVerdict::default(), EvidenceVerdict::NotRun);
    }

    #[test]
    fn not_run_blocked_and_unavailable_never_read_as_passed() {
        for verdict in [
            EvidenceVerdict::NotRun,
            EvidenceVerdict::Blocked,
            EvidenceVerdict::Unavailable,
            EvidenceVerdict::Superseded,
            EvidenceVerdict::Failed,
        ] {
            assert!(!verdict.is_passed(), "{verdict:?} must not read as passed");
            let gates = gate_outcome_for_verdicts(&[(true, verdict)]);
            assert!(
                !gates.completion_allowed(),
                "{verdict:?} must block completion"
            );
            assert!(!completion_decision(&gates, true).allowed);
        }
        let passed = gate_outcome_for_verdicts(&[(true, EvidenceVerdict::Passed)]);
        assert!(passed.completion_allowed());
    }

    #[tokio::test]
    async fn executed_evidence_records_command_exit_fingerprint_and_content_revision() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let mut spec = proposal(
            "cd engine && RUST_BACKTRACE=1 cargo test --lib",
            SpecAuthority::CardInstructions,
        );
        spec.toolchain_inputs = vec!["cargo 1.79.0".to_string()];

        let record = service(temp.path())
            .accept_and_execute(spec, &mut runner)
            .await;

        assert_eq!(record.verdict, EvidenceVerdict::Passed);
        assert_eq!(
            record.command,
            "cd engine && RUST_BACKTRACE=1 cargo test --lib"
        );
        assert_eq!(record.argv, vec!["cargo", "test", "--lib"]);
        assert_eq!(record.exit_code, Some(0));
        assert_eq!(
            record.fingerprint.content_revision,
            record.content_revision.digest()
        );
        assert_eq!(record.content_revision.spec_id, "S-1");
        assert_eq!(record.content_revision.commit.as_deref(), Some("abc1234"));
        assert_eq!(record.fingerprint.commit.as_deref(), Some("abc1234"));
        assert_eq!(
            record.fingerprint.worktree,
            temp.path().to_string_lossy().to_string()
        );
        assert_eq!(record.fingerprint.program, "cargo");
        assert_eq!(
            record
                .fingerprint
                .env
                .get("RUST_BACKTRACE")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            record.fingerprint.toolchain,
            vec!["cargo 1.79.0".to_string()]
        );
        assert_eq!(record.fingerprint.os, std::env::consts::OS);
        assert_eq!(
            record.accepted_by.as_deref(),
            Some(EVIDENCE_POLICY_AUTHORITY)
        );
        assert!(!record.started_at.is_empty());
        assert!(!record.finished_at.is_empty());
    }

    #[tokio::test]
    async fn an_expected_artifact_fails_when_absent_and_passes_when_present() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let mut spec = proposal("cargo test --lib", SpecAuthority::CardInstructions);
        spec.expected_artifacts = vec![ExpectedArtifact {
            path: "target/report.json".to_string(),
            assertion: ArtifactAssertion::Contains("\"passed\"".to_string()),
        }];

        let absent = service(temp.path())
            .accept_and_execute(spec.clone(), &mut runner)
            .await;
        assert_eq!(absent.verdict, EvidenceVerdict::Failed);
        assert!(!absent.artifacts[0].satisfied);
        assert_eq!(absent.exit_code, Some(0), "the command itself succeeded");

        tokio::fs::create_dir_all(temp.path().join("target"))
            .await
            .unwrap();
        tokio::fs::write(
            temp.path().join("target").join("report.json"),
            "{\"passed\": 12}",
        )
        .await
        .unwrap();

        let present = service(temp.path())
            .accept_and_execute(spec, &mut runner)
            .await;
        assert_eq!(present.verdict, EvidenceVerdict::Passed);
        assert!(present.artifacts[0].satisfied);
    }

    #[tokio::test]
    async fn an_artifact_outside_the_worktree_is_never_read() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let mut spec = proposal("cargo test --lib", SpecAuthority::CardInstructions);
        spec.expected_artifacts = vec![ExpectedArtifact {
            path: "../escape.txt".to_string(),
            assertion: ArtifactAssertion::Exists,
        }];

        let record = service(temp.path())
            .accept_and_execute(spec, &mut runner)
            .await;

        assert_eq!(record.verdict, EvidenceVerdict::Failed);
        assert!(record.artifacts[0]
            .detail
            .contains("must stay within the worktree"));
    }

    #[tokio::test]
    async fn an_agent_proposed_command_is_not_executed_until_accepted_under_policy() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let proposal = proposal(
            "cargo test --lib",
            SpecAuthority::AgentProposal {
                agent: "agent-1".to_string(),
            },
        );
        assert!(!proposal.is_accepted());
        assert!(proposal.argv.is_empty(), "a proposal carries no argv");

        let blocked = service(temp.path()).execute(&proposal, &mut runner).await;

        assert_eq!(blocked.verdict, EvidenceVerdict::Blocked);
        assert!(blocked
            .reason
            .as_deref()
            .unwrap()
            .contains("has not been accepted under policy"));
        assert!(runner.calls.is_empty(), "runner must never see a proposal");

        let accepted = proposal
            .accept(
                &VerifyCommandPolicy::permissive(),
                EVIDENCE_POLICY_AUTHORITY,
            )
            .unwrap();
        assert_eq!(
            accepted.accepted.as_ref().unwrap().accepted_by,
            EVIDENCE_POLICY_AUTHORITY
        );
        assert_eq!(
            accepted.accepted.as_ref().unwrap().authority,
            SpecAuthority::AgentProposal {
                agent: "agent-1".to_string()
            }
        );
        let record = service(temp.path()).execute(&accepted, &mut runner).await;
        assert_eq!(record.verdict, EvidenceVerdict::Passed);
        assert_eq!(runner.calls.len(), 1);
    }

    #[tokio::test]
    async fn verify_hint_prose_is_advisory_and_is_never_executed() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let hints = [
            "Run the lifecycle suite",
            "make sure the tests pass",
            "go through the migration list",
        ];
        let criteria: Vec<crate::chat::types::GoalCriterion> = hints
            .iter()
            .enumerate()
            .map(|(index, hint)| crate::chat::types::GoalCriterion {
                id: format!("C{index}"),
                text: "criterion".to_string(),
                verify_hint: Some(hint.to_string()),
                verify_command: None,
            })
            .collect();
        let prompt = crate::chat::goal_verifier::goal_verification_message("goal", &criteria);

        for hint in hints {
            assert!(
                prompt.content.content_text_only().contains(hint),
                "hints stay advisory in the judge prompt"
            );
            let spec = proposal(
                hint,
                SpecAuthority::AgentProposal {
                    agent: "agent-1".to_string(),
                },
            );
            let rejection = spec
                .clone()
                .accept(
                    &VerifyCommandPolicy::permissive(),
                    EVIDENCE_POLICY_AUTHORITY,
                )
                .unwrap_err();
            assert_eq!(rejection.verdict, EvidenceVerdict::Blocked);
            assert!(rejection.reason.contains("prose"), "{}", rejection.reason);

            let record = service(temp.path())
                .accept_and_execute(spec, &mut runner)
                .await;
            assert_eq!(record.verdict, EvidenceVerdict::Blocked);
        }
        assert!(
            runner.calls.is_empty(),
            "prose must never reach the runner: {:?}",
            runner.calls
        );
    }

    #[tokio::test]
    async fn evidence_for_an_older_revision_is_superseded_not_passed() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let spec = proposal("cargo test --lib", SpecAuthority::CardInstructions);
        let record = service(temp.path())
            .accept_and_execute(spec.clone(), &mut runner)
            .await;
        assert_eq!(record.verdict, EvidenceVerdict::Passed);

        let mut revised = spec;
        revised.revision = 2;
        let gates = evaluate_gates(&[revised], &[record]);

        assert_eq!(gates.blocks[0].verdict, EvidenceVerdict::Superseded);
        assert!(!gates.completion_allowed());
        assert!(!gates.has_contract_violation());
    }

    #[tokio::test]
    async fn the_service_runs_argv_and_never_a_shell() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();

        let record = service(temp.path())
            .accept_and_execute(
                proposal(
                    "cd refact-agent/engine && cargo check",
                    SpecAuthority::CardInstructions,
                ),
                &mut runner,
            )
            .await;

        assert_eq!(record.verdict, EvidenceVerdict::Passed);
        assert_eq!(runner.calls.len(), 1);
        assert_eq!(runner.calls[0].0, temp.path());
        assert_eq!(
            runner.calls[0].1.cwd,
            Some(PathBuf::from("refact-agent/engine"))
        );
        assert_eq!(runner.calls[0].1.argv, vec!["cargo", "check"]);
    }

    #[tokio::test]
    async fn the_service_threads_an_env_prefix_through_to_the_runner() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();

        let record = service(temp.path())
            .accept_and_execute(
                proposal(
                    "cd ui && FLEXUS_PYTHON=/abs/py pnpm typecheck",
                    SpecAuthority::CardInstructions,
                ),
                &mut runner,
            )
            .await;

        assert_eq!(record.verdict, EvidenceVerdict::Passed);
        assert_eq!(runner.calls[0].1.cwd, Some(PathBuf::from("ui")));
        assert_eq!(
            runner.calls[0].1.env,
            vec![("FLEXUS_PYTHON".to_string(), "/abs/py".to_string())]
        );
        assert_eq!(runner.calls[0].1.argv, vec!["pnpm", "typecheck"]);
    }

    #[tokio::test]
    async fn an_env_variable_outside_the_spec_allowlist_is_blocked() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let mut spec = proposal("SECRET=1 cargo test", SpecAuthority::CardInstructions);
        spec.env_allowlist = vec!["RUST_BACKTRACE".to_string()];

        let record = service(temp.path())
            .accept_and_execute(spec, &mut runner)
            .await;

        assert_eq!(record.verdict, EvidenceVerdict::Blocked);
        assert_eq!(record.outcome, VerificationOutcome::PolicyDenied);
        assert!(runner.calls.is_empty());
    }

    #[tokio::test]
    async fn the_service_runs_previously_unsupported_binaries() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();

        for command in [
            "dotnet test",
            "go test ./...",
            "make check",
            "gradlew build",
        ] {
            let record = service(temp.path())
                .accept_and_execute(
                    proposal(command, SpecAuthority::CardInstructions),
                    &mut runner,
                )
                .await;
            assert_eq!(record.verdict, EvidenceVerdict::Passed, "{command}");
        }
        assert_eq!(runner.calls.len(), 4);
    }

    #[tokio::test]
    async fn a_command_matching_a_shell_policy_deny_rule_is_blocked() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let service = EvidenceService::new(
            VerifyCommandPolicy::from_shell_policy(
                &crate::tools::shell_gate::ShellGatePolicy::default(),
            ),
            EvidenceContext {
                worktree: temp.path().to_path_buf(),
                commit: None,
                scope: "T-1".to_string(),
            },
        );

        let record = service
            .accept_and_execute(
                proposal("sudo rm -rf /", SpecAuthority::CardInstructions),
                &mut runner,
            )
            .await;

        assert_eq!(record.verdict, EvidenceVerdict::Blocked);
        assert_eq!(record.outcome, VerificationOutcome::PolicyDenied);
        assert!(record.output_tail.starts_with("Denied by shell policy:"));
        assert!(runner.calls.is_empty());
    }

    #[tokio::test]
    async fn yolo_mode_still_accepts_an_arbitrary_planner_authored_binary() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let service = EvidenceService::new(
            VerifyCommandPolicy::from_shell_policy(&crate::tools::shell_gate::ShellGatePolicy {
                mode: crate::tools::shell_gate::ApprovalMode::Yolo,
                ..crate::tools::shell_gate::ShellGatePolicy::default()
            }),
            EvidenceContext {
                worktree: temp.path().to_path_buf(),
                commit: None,
                scope: "T-1".to_string(),
            },
        );

        let record = service
            .accept_and_execute(
                proposal("mycompany-verify --all", SpecAuthority::CardInstructions),
                &mut runner,
            )
            .await;

        assert_eq!(record.verdict, EvidenceVerdict::Passed);
        assert_eq!(runner.calls[0].1.argv, vec!["mycompany-verify", "--all"]);
    }

    #[tokio::test]
    async fn shell_syntax_is_rejected_before_execution() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();

        let record = service(temp.path())
            .accept_and_execute(
                proposal("cargo test | tee f", SpecAuthority::CardInstructions),
                &mut runner,
            )
            .await;

        assert_eq!(record.verdict, EvidenceVerdict::Blocked);
        assert_eq!(record.outcome, VerificationOutcome::Rejected);
        assert!(record.output_tail.starts_with("Cannot run as a command:"));
        assert!(runner.calls.is_empty());
    }

    #[test]
    fn specs_from_a_card_only_include_declared_verification() {
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test --lib`");
        card.final_report_structured = Some(FinalReport {
            verification: vec![VerificationResult {
                command: "npm test".to_string(),
                passed: true,
                ..Default::default()
            }],
            ..Default::default()
        });

        let specs = specs_from_card(&card, &defaults());

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].command, "cargo test --lib");
        assert_eq!(specs[0].proposed_by, SpecAuthority::CardInstructions);
        assert!(specs
            .iter()
            .all(|spec| spec.required && !spec.is_accepted()));
        assert_eq!(specs[0].spec_id, "T-1#0");
    }

    #[test]
    fn a_card_without_commands_yields_no_specs_and_cannot_pass() {
        let specs = specs_from_card(&card("## Acceptance Criteria\n- it works"), &defaults());

        assert!(specs.is_empty());
        assert!(!evaluate_gates(&specs, &[]).completion_allowed());
    }

    #[tokio::test]
    async fn an_unexpected_exit_code_fails_and_a_declared_one_passes() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner {
            exit_code: Some(2),
            outcome: Some(VerificationOutcome::CommandFailed),
            ..RecordingRunner::default()
        };
        let mut spec = proposal("cargo test --lib", SpecAuthority::CardInstructions);

        let failed = service(temp.path())
            .accept_and_execute(spec.clone(), &mut runner)
            .await;
        assert_eq!(failed.verdict, EvidenceVerdict::Failed);
        assert!(failed.reason.unwrap().contains("exit code 2"));

        spec.expected_exit_codes = vec![0, 2];
        let passed = service(temp.path())
            .accept_and_execute(spec, &mut runner)
            .await;
        assert_eq!(passed.verdict, EvidenceVerdict::Passed);
    }

    #[tokio::test]
    async fn an_infrastructure_failure_is_unavailable_not_failed() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner {
            exit_code: None,
            outcome: Some(VerificationOutcome::InfrastructureFailed),
            ..RecordingRunner::default()
        };

        let record = service(temp.path())
            .accept_and_execute(
                proposal("cargo test --lib", SpecAuthority::CardInstructions),
                &mut runner,
            )
            .await;

        assert_eq!(record.verdict, EvidenceVerdict::Unavailable);
        assert!(!record.verdict.is_contract_violation());
        assert!(!record.verdict.is_passed());
    }

    #[tokio::test]
    async fn output_is_capped_by_the_spec() {
        let temp = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::default();
        let mut spec = proposal("cargo test --lib", SpecAuthority::CardInstructions);
        spec.output_cap_bytes = 6;

        let record = service(temp.path())
            .accept_and_execute(spec, &mut runner)
            .await;

        assert_eq!(record.output_tail, "output");
    }
}
