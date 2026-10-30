use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMeta {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub status: TaskStatus,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub cards_total: usize,
    #[serde(default)]
    pub cards_done: usize,
    #[serde(default)]
    pub cards_failed: usize,
    #[serde(default)]
    pub agents_active: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_model: Option<String>,
    #[serde(default)]
    pub is_name_generated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_agents_summary_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_session_state: Option<String>,
}

fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Planning,
    Active,
    Paused,
    Completed,
    Abandoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScopeGuardMode {
    #[default]
    Off,
    Warn,
    Reject,
    Strict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBoard {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub rev: u64,
    #[serde(default = "default_columns")]
    pub columns: Vec<BoardColumn>,
    #[serde(default)]
    pub cards: Vec<BoardCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviewer_chat_ids: Vec<String>,
    #[serde(default)]
    pub review_policy: BoardReviewPolicy,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub review_attestations: std::collections::BTreeMap<String, BoardReviewAttestation>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub card_claims: std::collections::BTreeMap<String, CardClaim>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub dependency_overrides: std::collections::BTreeMap<String, DependencyOverride>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub card_outcomes: std::collections::BTreeMap<String, CardOutcome>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub planner_replies:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, PlannerReply>>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub card_attempt_generations: std::collections::BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerReplyAttempt {
    pub assignee: Option<String>,
    pub chat_id: Option<String>,
    pub started_at: Option<String>,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerReplyState {
    Recorded,
    Accepted,
    Delivered,
    Unknown,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerReply {
    pub version: u32,
    pub task_id: String,
    pub card_id: String,
    pub question_id: String,
    pub planner_chat_id: String,
    pub action_id: String,
    pub answered_at: String,
    pub answer: String,
    pub answer_input_hash: String,
    pub attempt: PlannerReplyAttempt,
    pub envelope: serde_json::Value,
    pub envelope_hash: String,
    pub push: String,
    pub ttl_secs: u64,
    pub state: PlannerReplyState,
    pub outcome: String,
    pub reservation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardClaim {
    pub claimant_chat_id: String,
    pub idempotency_key: String,
    pub claimed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyOverride {
    pub overridden_by: String,
    pub reason: String,
    pub overridden_at: String,
    pub unsatisfied: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed,
    AlreadyHeld,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardRuleViolation {
    CardNotFound {
        card_id: String,
    },
    AlreadyClaimed {
        card_id: String,
        holder: String,
    },
    DependenciesUnsatisfied {
        card_id: String,
        unsatisfied: Vec<String>,
    },
    RoleDenied {
        card_id: String,
        reason: String,
    },
}

impl BoardRuleViolation {
    pub fn code(&self) -> &'static str {
        match self {
            BoardRuleViolation::CardNotFound { .. } => "not_found_or_not_visible",
            BoardRuleViolation::AlreadyClaimed { .. } => "already_claimed",
            BoardRuleViolation::DependenciesUnsatisfied { .. } => "dependency_unsatisfied",
            BoardRuleViolation::RoleDenied { .. } => "scope_denied",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, BoardRuleViolation::DependenciesUnsatisfied { .. })
    }

    pub fn message(&self) -> String {
        match self {
            BoardRuleViolation::CardNotFound { card_id } => {
                format!("card {card_id} not found on this board")
            }
            BoardRuleViolation::AlreadyClaimed { card_id, holder } => {
                format!("card {card_id} is already claimed by {holder}")
            }
            BoardRuleViolation::DependenciesUnsatisfied {
                card_id,
                unsatisfied,
            } => format!(
                "card {card_id} is blocked by unsatisfied dependencies: {}",
                unsatisfied.join(", ")
            ),
            BoardRuleViolation::RoleDenied { card_id, reason } => {
                format!("card {card_id}: {reason}")
            }
        }
    }
}

impl std::fmt::Display for BoardRuleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BoardReviewPolicy {
    #[default]
    OwnerAllowed,
    IndependentRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardReviewAttestation {
    pub reviewer_chat_id: String,
    pub reviewed_at: String,
    pub report: VerifierReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardAttention {
    pub reason: String,
    /// "transient" | "stall" | "no_session" | "unknown"
    pub kind: String,
    /// RFC 3339 timestamp of the first flag in the current episode
    pub since: String,
    pub count: u32,
}

pub const OUTCOME_FRAGMENT_SUMMARY: &str = "summary";
pub const OUTCOME_FRAGMENT_FILES_CHANGED: &str = "files_changed";
pub const OUTCOME_FRAGMENT_TESTS: &str = "tests_added_or_updated";
pub const OUTCOME_FRAGMENT_VERIFICATION: &str = "verification";
pub const OUTCOME_FRAGMENT_FOLLOWUP_CARDS: &str = "followup_cards";
pub const OUTCOME_FRAGMENT_RISKS: &str = "risks";
pub const OUTCOME_FRAGMENT_ASSUMPTIONS: &str = "assumptions";

const OUTCOME_PLACEHOLDERS: [&str; 5] = ["_none._", "_none_", "_none provided._", "none", "n/a"];

/// A required fragment is never satisfied by an empty section or by the
/// placeholder text the markdown renderer emits for an absent one.
pub fn fragment_content_is_absent(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return true;
    }
    trimmed.lines().all(|line| {
        let line = line.trim().trim_start_matches('-').trim();
        line.is_empty()
            || OUTCOME_PLACEHOLDERS
                .iter()
                .any(|placeholder| line.eq_ignore_ascii_case(placeholder))
    })
}

pub fn normalize_fragment_name(name: &str) -> String {
    name.trim()
        .trim_matches('`')
        .trim_matches('.')
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

/// The planner declares required sections in the card instructions:
/// `Required outcome: summary, verification` and, for sections that must carry
/// evidence provenance, `Required outcome evidence: verification`.
pub fn outcome_requirements_from_instructions(instructions: &str) -> Vec<OutcomeRequirement> {
    let mut required: Vec<OutcomeRequirement> = Vec::new();
    let mut needs_evidence: Vec<String> = Vec::new();
    for line in instructions.lines() {
        let line = line.trim().trim_start_matches('-').trim().replace("**", "");
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("required outcome evidence:") {
            for name in fragment_names(rest) {
                if !needs_evidence.contains(&name) {
                    needs_evidence.push(name);
                }
            }
        } else if let Some(rest) = lower.strip_prefix("required outcome:") {
            for name in fragment_names(rest) {
                if !required.iter().any(|entry| entry.fragment == name) {
                    required.push(OutcomeRequirement {
                        fragment: name,
                        requires_evidence: false,
                    });
                }
            }
        }
    }
    for name in needs_evidence {
        match required.iter_mut().find(|entry| entry.fragment == name) {
            Some(entry) => entry.requires_evidence = true,
            None => required.push(OutcomeRequirement {
                fragment: name,
                requires_evidence: true,
            }),
        }
    }
    required
}

fn fragment_names(text: &str) -> Vec<String> {
    text.split(',')
        .map(normalize_fragment_name)
        .filter(|name| !name.is_empty())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FragmentReview {
    #[default]
    Pending,
    Approved,
    Rejected,
    Contradicted,
}

impl FragmentReview {
    pub fn label(&self) -> &'static str {
        match self {
            FragmentReview::Pending => "pending",
            FragmentReview::Approved => "approved",
            FragmentReview::Rejected => "rejected",
            FragmentReview::Contradicted => "contradicted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OutcomeRequirement {
    pub fragment: String,
    pub requires_evidence: bool,
}

/// Which run produced a fragment, and which evidence records support it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OutcomeProvenance {
    pub source_run: Option<String>,
    pub evidence_spec_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OutcomeFragment {
    pub name: String,
    pub content: String,
    pub provenance: OutcomeProvenance,
    pub review: FragmentReview,
    pub reviewer_chat_id: Option<String>,
    pub reviewed_at: Option<String>,
    pub review_note: Option<String>,
}

impl OutcomeFragment {
    pub fn new(
        name: impl Into<String>,
        content: impl Into<String>,
        source_run: Option<String>,
    ) -> Self {
        Self {
            name: normalize_fragment_name(&name.into()),
            content: content.into(),
            provenance: OutcomeProvenance {
                source_run,
                evidence_spec_ids: Vec::new(),
            },
            ..Default::default()
        }
    }

    pub fn has_content(&self) -> bool {
        !fragment_content_is_absent(&self.content)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardFinalization {
    pub finalized_by: String,
    pub finalized_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CardOutcome {
    pub required: Vec<OutcomeRequirement>,
    pub fragments: Vec<OutcomeFragment>,
    pub finalized: Option<CardFinalization>,
}

impl CardOutcome {
    pub fn is_empty(&self) -> bool {
        self.required.is_empty() && self.fragments.is_empty() && self.finalized.is_none()
    }

    pub fn fragment(&self, name: &str) -> Option<&OutcomeFragment> {
        let name = normalize_fragment_name(name);
        self.fragments.iter().find(|entry| entry.name == name)
    }

    pub fn requirement(&self, name: &str) -> Option<&OutcomeRequirement> {
        let name = normalize_fragment_name(name);
        self.required.iter().find(|entry| entry.fragment == name)
    }

    pub fn set_fragment(&mut self, fragment: OutcomeFragment) {
        match self
            .fragments
            .iter_mut()
            .find(|entry| entry.name == fragment.name)
        {
            Some(existing) => *existing = fragment,
            None => self.fragments.push(fragment),
        }
    }

    pub fn record_review(
        &mut self,
        fragment: &str,
        review: FragmentReview,
        reviewer_chat_id: &str,
        reviewed_at: String,
        note: Option<String>,
    ) -> Result<(), String> {
        let name = normalize_fragment_name(fragment);
        let entry = self
            .fragments
            .iter_mut()
            .find(|entry| entry.name == name)
            .ok_or_else(|| format!("outcome has no fragment {name}"))?;
        entry.review = review;
        entry.reviewer_chat_id = Some(reviewer_chat_id.to_string());
        entry.reviewed_at = Some(reviewed_at);
        entry.review_note = note;
        Ok(())
    }

    pub fn attach_evidence(&mut self, fragment: &str, spec_ids: &[String]) {
        let name = normalize_fragment_name(fragment);
        let Some(entry) = self.fragments.iter_mut().find(|entry| entry.name == name) else {
            return;
        };
        for spec_id in spec_ids {
            if !entry.provenance.evidence_spec_ids.contains(spec_id) {
                entry.provenance.evidence_spec_ids.push(spec_id.clone());
            }
        }
    }

    pub fn to_markdown(&self) -> String {
        let mut output = String::from("# Outcome Document\n\n");
        for requirement in &self.required {
            output.push_str("## ");
            output.push_str(&requirement.fragment);
            output.push_str(if requirement.requires_evidence {
                " (required, evidence)\n"
            } else {
                " (required)\n"
            });
            match self
                .fragment(&requirement.fragment)
                .filter(|fragment| fragment.has_content())
            {
                Some(fragment) => push_fragment_body(&mut output, fragment),
                None => output.push_str("**MISSING — required outcome fragment.**\n\n"),
            }
        }
        for fragment in &self.fragments {
            if self.requirement(&fragment.name).is_some() {
                continue;
            }
            output.push_str("## ");
            output.push_str(&fragment.name);
            output.push('\n');
            push_fragment_body(&mut output, fragment);
        }
        if let Some(finalized) = self.finalized.as_ref() {
            output.push_str(&format!(
                "## Finalized\n- by: {}\n- at: {}\n\n",
                finalized.finalized_by, finalized.finalized_at
            ));
        }
        while output.ends_with('\n') {
            output.pop();
        }
        output.push('\n');
        output
    }
}

fn push_fragment_body(output: &mut String, fragment: &OutcomeFragment) {
    let content = fragment.content.trim();
    if content.is_empty() {
        output.push_str("_None._\n");
    } else {
        output.push_str(content);
        output.push('\n');
    }
    output.push_str(&format!(
        "- source run: {}\n",
        fragment
            .provenance
            .source_run
            .as_deref()
            .unwrap_or("_unrecorded_")
    ));
    if fragment.provenance.evidence_spec_ids.is_empty() {
        output.push_str("- evidence: _unrecorded_\n");
    } else {
        output.push_str(&format!(
            "- evidence: {}\n",
            fragment.provenance.evidence_spec_ids.join(", ")
        ));
    }
    output.push_str(&format!("- review: {}\n\n", fragment.review.label()));
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FinalizeBlock {
    CardNotFound {
        card_id: String,
    },
    NotCompleted {
        card_id: String,
        column: String,
    },
    AlreadyFinalized {
        card_id: String,
        finalized_by: String,
    },
    ReviewDenied {
        reason: String,
    },
    MissingFragment {
        fragment: String,
    },
    EmptyFragment {
        fragment: String,
    },
    RejectedFragment {
        fragment: String,
        reason: String,
    },
    ContradictedFragment {
        fragment: String,
        reason: String,
    },
    MissingSourceRun {
        fragment: String,
    },
    MissingEvidence {
        fragment: String,
    },
}

impl FinalizeBlock {
    pub fn message(&self) -> String {
        match self {
            FinalizeBlock::CardNotFound { card_id } => {
                format!("card {card_id} not found on this board")
            }
            FinalizeBlock::NotCompleted { card_id, column } => {
                format!("card {card_id} is in '{column}'; finalising requires a completed card")
            }
            FinalizeBlock::AlreadyFinalized {
                card_id,
                finalized_by,
            } => format!("card {card_id} was already finalized by {finalized_by}"),
            FinalizeBlock::ReviewDenied { reason } => reason.clone(),
            FinalizeBlock::MissingFragment { fragment } => {
                format!("required outcome fragment '{fragment}' is missing")
            }
            FinalizeBlock::EmptyFragment { fragment } => {
                format!("required outcome fragment '{fragment}' is empty or a placeholder")
            }
            FinalizeBlock::RejectedFragment { fragment, reason } => {
                format!("required outcome fragment '{fragment}' was rejected in review: {reason}")
            }
            FinalizeBlock::ContradictedFragment { fragment, reason } => {
                format!(
                    "required outcome fragment '{fragment}' is contradicted and unresolved: {reason}"
                )
            }
            FinalizeBlock::MissingSourceRun { fragment } => format!(
                "required outcome fragment '{fragment}' does not record the run that produced it"
            ),
            FinalizeBlock::MissingEvidence { fragment } => {
                format!("required outcome fragment '{fragment}' references no evidence record")
            }
        }
    }
}

/// Every unmet requirement at once: a blocked finalize is a gate answer, not a
/// first-failure notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FinalizeDecision {
    pub blocks: Vec<FinalizeBlock>,
}

impl FinalizeDecision {
    pub fn allowed(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn message(&self) -> String {
        if self.blocks.is_empty() {
            return "finalize allowed".to_string();
        }
        let reasons = self
            .blocks
            .iter()
            .map(|block| format!("- {}", block.message()))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "finalize blocked by {} unmet requirement(s):\n{}",
            self.blocks.len(),
            reasons
        )
    }
}

fn default_columns() -> Vec<BoardColumn> {
    vec![
        BoardColumn {
            id: "planned".into(),
            title: "Planned".into(),
        },
        BoardColumn {
            id: "doing".into(),
            title: "Doing".into(),
        },
        BoardColumn {
            id: "done".into(),
            title: "Done".into(),
        },
        BoardColumn {
            id: "failed".into(),
            title: "Failed".into(),
        },
        BoardColumn {
            id: "regressed".into(),
            title: "Regressed".into(),
        },
    ]
}

impl Default for TaskBoard {
    fn default() -> Self {
        Self {
            schema_version: 1,
            rev: 0,
            columns: default_columns(),
            cards: vec![],
            owner_chat_id: None,
            reviewer_chat_ids: Vec::new(),
            review_policy: BoardReviewPolicy::default(),
            review_attestations: std::collections::BTreeMap::new(),
            card_claims: std::collections::BTreeMap::new(),
            dependency_overrides: std::collections::BTreeMap::new(),
            card_outcomes: std::collections::BTreeMap::new(),
            planner_replies: std::collections::BTreeMap::new(),
            card_attempt_generations: std::collections::BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardColumn {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FinalReport {
    pub summary: String,
    pub success: bool,
    pub files_changed: Vec<String>,
    pub tests_added_or_updated: Vec<String>,
    pub verification: Vec<VerificationResult>,
    pub followup_cards: Vec<SuggestedCard>,
    pub risks: Vec<String>,
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VerificationResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub passed: bool,
    pub output_tail: String,
    pub outcome: VerificationOutcome,
    /// True when this result was replayed from the evidence cache rather than
    /// produced by a run of the command.
    pub from_cache: bool,
    /// When the command actually ran, carried through replays unchanged.
    pub executed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    #[default]
    Unknown,
    Passed,
    CommandFailed,
    Rejected,
    PolicyDenied,
    InfrastructureFailed,
    NoCommands,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VerifierReport {
    pub passed: bool,
    pub command_results: Vec<VerificationResult>,
    pub concerns: Vec<String>,
    pub recommendation: String,
    pub classification: VerifierReportClassification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VerifierReportClassification {
    #[default]
    Unknown,
    Passed,
    VerificationFailed,
    HumanReview,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SuggestedCard {
    pub title: String,
    pub instructions: String,
    pub priority: String,
    pub target_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbVariantInfo {
    pub agent_id: String,
    pub chat_id: String,
    pub worktree: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<AbVariantFinish>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AbVariantFinish {
    pub success: bool,
    pub final_report: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_report_structured: Option<FinalReport>,
    pub completed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbVariants {
    pub a: AbVariantInfo,
    pub b: AbVariantInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner: Option<String>,
}

impl AbVariants {
    pub fn variant(&self, key: &str) -> Option<&AbVariantInfo> {
        match key {
            "a" => Some(&self.a),
            "b" => Some(&self.b),
            _ => None,
        }
    }

    pub fn variant_mut(&mut self, key: &str) -> Option<&mut AbVariantInfo> {
        match key {
            "a" => Some(&mut self.a),
            "b" => Some(&mut self.b),
            _ => None,
        }
    }

    pub fn variant_key_for_chat_id(&self, chat_id: &str) -> Option<&'static str> {
        if self.a.chat_id == chat_id {
            Some("a")
        } else if self.b.chat_id == chat_id {
            Some("b")
        } else {
            None
        }
    }

    pub fn all_finished(&self) -> bool {
        self.a.finish.is_some() && self.b.finish.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamMember {
    pub role: String,
    pub agent_chat_id: Option<String>,
    pub agent_branch: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardComment {
    pub id: String,
    pub author_role: String,
    pub author_id: Option<String>,
    pub timestamp: String,
    pub body: String,
    pub reply_to: Option<String>,
}

impl FinalReport {
    pub fn to_markdown(&self) -> String {
        let mut output = String::new();

        output.push_str("# Final Report\n\n");
        output.push_str("## Summary\n");
        push_text_or_empty(&mut output, &self.summary);
        output.push('\n');

        output.push_str("## Result\n");
        output.push_str(if self.success {
            "- Success: true\n\n"
        } else {
            "- Success: false\n\n"
        });

        push_string_list(&mut output, "Files Changed", &self.files_changed);
        push_string_list(
            &mut output,
            "Tests Added or Updated",
            &self.tests_added_or_updated,
        );
        push_verification(&mut output, &self.verification);
        push_followup_cards(&mut output, &self.followup_cards);
        push_string_list(&mut output, "Risks", &self.risks);
        push_string_list(&mut output, "Assumptions", &self.assumptions);

        while output.ends_with('\n') {
            output.pop();
        }
        output.push('\n');
        output
    }

    /// Outcome fragments carry the section text plus the run that produced it.
    /// Sections the run did not provide yield no fragment at all, so a declared
    /// requirement reports them as missing rather than as an empty placeholder.
    pub fn to_outcome_fragments(&self, source_run: Option<&str>) -> Vec<OutcomeFragment> {
        let source_run = source_run.map(str::to_string);
        let mut fragments = Vec::new();
        let mut push = |name: &str, content: String| {
            if !fragment_content_is_absent(&content) {
                fragments.push(OutcomeFragment::new(name, content, source_run.clone()));
            }
        };
        push(OUTCOME_FRAGMENT_SUMMARY, self.summary.clone());
        push(
            OUTCOME_FRAGMENT_FILES_CHANGED,
            bullet_lines(&self.files_changed),
        );
        push(
            OUTCOME_FRAGMENT_TESTS,
            bullet_lines(&self.tests_added_or_updated),
        );
        push(
            OUTCOME_FRAGMENT_VERIFICATION,
            self.verification
                .iter()
                .map(|result| match result.exit_code {
                    Some(exit_code) => format!(
                        "- {} — {} (exit code {})",
                        result.command,
                        if result.passed { "passed" } else { "failed" },
                        exit_code
                    ),
                    None => format!(
                        "- {} — {}",
                        result.command,
                        if result.passed { "passed" } else { "failed" }
                    ),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
        push(
            OUTCOME_FRAGMENT_FOLLOWUP_CARDS,
            self.followup_cards
                .iter()
                .map(|card| format!("- {}", card.title))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        push(OUTCOME_FRAGMENT_RISKS, bullet_lines(&self.risks));
        push(
            OUTCOME_FRAGMENT_ASSUMPTIONS,
            bullet_lines(&self.assumptions),
        );
        fragments
    }
}

fn bullet_lines(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {}", item))
        .collect::<Vec<_>>()
        .join("\n")
}

fn push_text_or_empty(output: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        output.push_str("_None provided._\n");
    } else {
        output.push_str(text);
        output.push('\n');
    }
}

fn push_string_list(output: &mut String, title: &str, items: &[String]) {
    output.push_str("## ");
    output.push_str(title);
    output.push('\n');
    if items.is_empty() {
        output.push_str("- _None._\n\n");
        return;
    }
    for item in items {
        output.push_str("- ");
        output.push_str(item);
        output.push('\n');
    }
    output.push('\n');
}

fn push_verification(output: &mut String, verification: &[VerificationResult]) {
    output.push_str("## Verification\n");
    if verification.is_empty() {
        output.push_str("- _None._\n\n");
        return;
    }
    for result in verification {
        output.push_str("- ");
        output.push_str(&markdown_inline_code(&result.command));
        output.push_str(" — ");
        output.push_str(if result.passed { "passed" } else { "failed" });
        if let Some(exit_code) = result.exit_code {
            output.push_str(&format!(" (exit code {})", exit_code));
        }
        output.push('\n');
        let tail = result.output_tail.trim();
        if !tail.is_empty() {
            let fence = markdown_code_fence(tail);
            output.push('\n');
            output.push_str(&fence);
            output.push_str("text\n");
            output.push_str(tail);
            output.push('\n');
            output.push_str(&fence);
            output.push('\n');
        }
    }
    output.push('\n');
}

fn markdown_inline_code(text: &str) -> String {
    let max_run = max_backtick_run(text);
    if max_run == 0 {
        return format!("`{}`", text);
    }
    let fence = "`".repeat(max_run + 1);
    format!("{} {} {}", fence, text, fence)
}

fn markdown_code_fence(text: &str) -> String {
    "`".repeat(max_backtick_run(text).max(2) + 1)
}

fn max_backtick_run(text: &str) -> usize {
    let mut max_run = 0;
    let mut current = 0;
    for c in text.chars() {
        if c == '`' {
            current += 1;
            max_run = max_run.max(current);
        } else {
            current = 0;
        }
    }
    max_run
}

fn push_followup_cards(output: &mut String, cards: &[SuggestedCard]) {
    output.push_str("## Follow-up Cards\n");
    if cards.is_empty() {
        output.push_str("- _None._\n\n");
        return;
    }
    for card in cards {
        output.push_str("### ");
        output.push_str(if card.title.trim().is_empty() {
            "Untitled"
        } else {
            card.title.trim()
        });
        output.push('\n');
        output.push_str("- Priority: ");
        output.push_str(if card.priority.trim().is_empty() {
            "unspecified"
        } else {
            card.priority.trim()
        });
        output.push('\n');
        if !card.target_files.is_empty() {
            output.push_str("- Target files: ");
            output.push_str(&card.target_files.join(", "));
            output.push('\n');
        }
        if !card.instructions.trim().is_empty() {
            output.push('\n');
            output.push_str(card.instructions.trim());
            output.push('\n');
        }
        output.push('\n');
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardCard {
    pub id: String,
    pub title: String,
    pub column: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub instructions: String,
    pub assignee: Option<String>,
    pub agent_chat_id: Option<String>,
    #[serde(default)]
    pub status_updates: Vec<StatusUpdate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<CardComment>,
    #[serde(default)]
    pub final_report: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_report_structured: Option<FinalReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier_report: Option<VerifierReport>,
    pub created_at: String,
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_by: Option<String>,
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_worktree_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ab_variants: Option<AbVariants>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub team_members: Vec<TeamMember>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_files: Vec<String>,
    #[serde(default)]
    pub scope_guard_mode: ScopeGuardMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<CardAttention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliverable: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification: Vec<serde_json::Value>,
}

fn default_priority() -> String {
    "P1".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusUpdate {
    pub timestamp: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyCardsResult {
    pub ready: Vec<String>,
    pub blocked: Vec<String>,
    pub in_progress: Vec<String>,
    pub completed: Vec<String>,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryInfo {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting_for_card_ids: Vec<String>,
}

impl TaskBoard {
    pub fn validate_dependencies(&self) -> Result<(), String> {
        let mut by_id = std::collections::HashMap::new();
        for (index, card) in self.cards.iter().enumerate() {
            if by_id.insert(card.id.as_str(), index).is_some() {
                return Err(format!("Duplicate card ID {}", card.id));
            }
        }
        let mut indegree = vec![0usize; self.cards.len()];
        let mut dependents = vec![Vec::new(); self.cards.len()];
        for card in &self.cards {
            if card
                .depends_on
                .iter()
                .any(|dependency| dependency == &card.id)
            {
                return Err(format!("Card {} cannot depend on itself", card.id));
            }
            let mut unique = std::collections::HashSet::new();
            if let Some(duplicate) = card
                .depends_on
                .iter()
                .find(|dependency| !unique.insert(dependency.as_str()))
            {
                return Err(format!(
                    "Card {} lists dependency {} more than once",
                    card.id, duplicate
                ));
            }
            if let Some(missing) = card
                .depends_on
                .iter()
                .find(|dependency| !by_id.contains_key(dependency.as_str()))
            {
                return Err(format!(
                    "Card {} dependency {} does not exist",
                    card.id, missing
                ));
            }
        }
        for (card_index, card) in self.cards.iter().enumerate() {
            indegree[card_index] = card.depends_on.len();
            for dependency in &card.depends_on {
                dependents[by_id[dependency.as_str()]].push(card_index);
            }
        }
        let mut ready = indegree
            .iter()
            .enumerate()
            .filter_map(|(index, degree)| (*degree == 0).then_some(index))
            .collect::<std::collections::VecDeque<_>>();
        let mut visited = 0usize;
        while let Some(index) = ready.pop_front() {
            visited += 1;
            for dependent in &dependents[index] {
                indegree[*dependent] -= 1;
                if indegree[*dependent] == 0 {
                    ready.push_back(*dependent);
                }
            }
        }
        if visited != self.cards.len() {
            let cycle = indegree
                .iter()
                .enumerate()
                .find_map(|(index, degree)| (*degree > 0).then_some(self.cards[index].id.as_str()))
                .unwrap_or("unknown");
            return Err(format!("Dependency cycle includes {cycle}"));
        }
        Ok(())
    }

    pub fn validate_claims(&self, max_claim_key_len: usize) -> Result<(), String> {
        for (card_id, claim) in &self.card_claims {
            if self.get_card(card_id).is_none() {
                return Err(format!("Claim references missing card {card_id}"));
            }
            if claim.claimant_chat_id.trim().is_empty() {
                return Err(format!("Claim on card {card_id} has an empty claimant"));
            }
            if claim.idempotency_key.trim().is_empty() {
                return Err(format!(
                    "Claim on card {card_id} has an empty idempotency key"
                ));
            }
            if claim.idempotency_key.chars().count() > max_claim_key_len {
                return Err(format!(
                    "Claim idempotency key on card {card_id} exceeds the configured maximum of {max_claim_key_len} characters"
                ));
            }
        }
        for (card_id, override_record) in &self.dependency_overrides {
            if self.get_card(card_id).is_none() {
                return Err(format!(
                    "Dependency override references missing card {card_id}"
                ));
            }
            if override_record.overridden_by.trim().is_empty() {
                return Err(format!(
                    "Dependency override on card {card_id} has no recorded principal"
                ));
            }
            if override_record.reason.trim().is_empty() {
                return Err(format!(
                    "Dependency override on card {card_id} has no recorded reason"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_authority(&self, max_reviewers: usize) -> Result<(), String> {
        if self.reviewer_chat_ids.len() > max_reviewers {
            return Err(format!(
                "Task board has {} reviewers, exceeding the configured maximum of {}",
                self.reviewer_chat_ids.len(),
                max_reviewers
            ));
        }
        let mut unique = std::collections::HashSet::new();
        for reviewer in &self.reviewer_chat_ids {
            if reviewer.trim().is_empty() {
                return Err("Task board reviewer IDs cannot be empty".to_string());
            }
            if !unique.insert(reviewer.as_str()) {
                return Err(format!("Task board reviewer {} is duplicated", reviewer));
            }
            if self.owner_chat_id.as_deref() == Some(reviewer.as_str()) {
                return Err("Task board owner cannot also be an assigned reviewer".to_string());
            }
        }
        if self.owner_chat_id.as_deref().is_some_and(str::is_empty) {
            return Err("Task board owner ID cannot be empty".to_string());
        }
        for (card_id, attestation) in &self.review_attestations {
            let card = self
                .get_card(card_id)
                .ok_or_else(|| format!("Review attestation references missing card {card_id}"))?;
            if card.verifier_report.as_ref() != Some(&attestation.report) {
                return Err(format!(
                    "Review attestation for card {card_id} does not match its verifier report"
                ));
            }
            if !self
                .reviewer_chat_ids
                .iter()
                .any(|reviewer| reviewer == &attestation.reviewer_chat_id)
            {
                return Err(format!(
                    "Review attestation for card {card_id} names unassigned reviewer {}",
                    attestation.reviewer_chat_id
                ));
            }
            if self.owner_chat_id.as_deref() == Some(attestation.reviewer_chat_id.as_str()) {
                return Err(format!(
                    "Review attestation for card {card_id} was produced by the board owner"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_for_write(
        &self,
        max_reviewers: usize,
        max_claim_key_len: usize,
    ) -> Result<(), String> {
        self.validate_dependencies()?;
        self.validate_authority(max_reviewers)?;
        self.validate_claims(max_claim_key_len)
    }

    pub fn card_claim(&self, card_id: &str) -> Option<&CardClaim> {
        self.card_claims.get(card_id)
    }

    pub fn unsatisfied_dependencies(
        &self,
        card_id: &str,
    ) -> Result<Vec<String>, BoardRuleViolation> {
        let card = self
            .get_card(card_id)
            .ok_or_else(|| BoardRuleViolation::CardNotFound {
                card_id: card_id.to_string(),
            })?;
        Ok(card
            .depends_on
            .iter()
            .filter(|dependency| {
                self.get_card(dependency)
                    .is_none_or(|dependency_card| dependency_card.column != "done")
            })
            .cloned()
            .collect())
    }

    pub fn dependency_gate(&self, card_id: &str) -> Result<(), BoardRuleViolation> {
        let unsatisfied = self.unsatisfied_dependencies(card_id)?;
        if unsatisfied.is_empty() || self.dependency_overrides.contains_key(card_id) {
            return Ok(());
        }
        Err(BoardRuleViolation::DependenciesUnsatisfied {
            card_id: card_id.to_string(),
            unsatisfied,
        })
    }

    /// Terminal columns are always reachable; only entering `doing` is gated on
    /// dependencies, and an explicit recorded override lifts that gate.
    pub fn ensure_column_move_allowed(
        &self,
        card_id: &str,
        column: &str,
    ) -> Result<(), BoardRuleViolation> {
        if self.get_card(card_id).is_none() {
            return Err(BoardRuleViolation::CardNotFound {
                card_id: card_id.to_string(),
            });
        }
        if column == "doing" {
            return self.dependency_gate(card_id);
        }
        Ok(())
    }

    pub fn claim_card(
        &mut self,
        card_id: &str,
        claimant_chat_id: &str,
        idempotency_key: &str,
        claimed_at: String,
    ) -> Result<ClaimOutcome, BoardRuleViolation> {
        self.dependency_gate(card_id)?;
        match self.card_claims.get(card_id) {
            Some(existing)
                if existing.claimant_chat_id == claimant_chat_id
                    && existing.idempotency_key == idempotency_key =>
            {
                Ok(ClaimOutcome::AlreadyHeld)
            }
            Some(existing) => Err(BoardRuleViolation::AlreadyClaimed {
                card_id: card_id.to_string(),
                holder: existing.claimant_chat_id.clone(),
            }),
            None => {
                self.card_claims.insert(
                    card_id.to_string(),
                    CardClaim {
                        claimant_chat_id: claimant_chat_id.to_string(),
                        idempotency_key: idempotency_key.to_string(),
                        claimed_at,
                    },
                );
                Ok(ClaimOutcome::Claimed)
            }
        }
    }

    pub fn release_claim(
        &mut self,
        card_id: &str,
        claimant_chat_id: &str,
    ) -> Result<(), BoardRuleViolation> {
        match self.card_claims.get(card_id) {
            None => Ok(()),
            Some(existing) if existing.claimant_chat_id == claimant_chat_id => {
                self.card_claims.remove(card_id);
                Ok(())
            }
            Some(existing) => Err(BoardRuleViolation::AlreadyClaimed {
                card_id: card_id.to_string(),
                holder: existing.claimant_chat_id.clone(),
            }),
        }
    }

    pub fn record_dependency_override(
        &mut self,
        card_id: &str,
        overridden_by: &str,
        reason: &str,
        overridden_at: String,
    ) -> Result<Vec<String>, BoardRuleViolation> {
        let unsatisfied = self.unsatisfied_dependencies(card_id)?;
        self.dependency_overrides.insert(
            card_id.to_string(),
            DependencyOverride {
                overridden_by: overridden_by.to_string(),
                reason: reason.to_string(),
                overridden_at,
                unsatisfied: unsatisfied.clone(),
            },
        );
        Ok(unsatisfied)
    }

    /// Boards without an owner are legacy boards and keep their previous
    /// behaviour. Once ownership exists, finalising a card requires the owner
    /// or an assigned reviewer, and the claimant may never finalise its own
    /// card unless self-review is explicitly enabled.
    pub fn authorize_finalizer(
        &self,
        card_id: &str,
        principal: &str,
        allow_self_review: bool,
    ) -> Result<(), BoardRuleViolation> {
        if self.get_card(card_id).is_none() {
            return Err(BoardRuleViolation::CardNotFound {
                card_id: card_id.to_string(),
            });
        }
        let Some(owner) = self.owner_chat_id.as_deref() else {
            return Ok(());
        };
        let is_owner = owner == principal;
        let is_reviewer = self
            .reviewer_chat_ids
            .iter()
            .any(|reviewer| reviewer == principal);
        if !is_owner && !is_reviewer {
            return Err(BoardRuleViolation::RoleDenied {
                card_id: card_id.to_string(),
                reason: "finalising requires the board owner or an assigned reviewer".to_string(),
            });
        }
        let claims_the_card = self
            .card_claims
            .get(card_id)
            .is_some_and(|claim| claim.claimant_chat_id == principal);
        if claims_the_card && !allow_self_review {
            return Err(BoardRuleViolation::RoleDenied {
                card_id: card_id.to_string(),
                reason: "self-review is refused; enable coordination.board_allow_self_review to permit the claimant to finalise its own card".to_string(),
            });
        }
        Ok(())
    }

    pub fn dependencies_satisfied(&self, card_id: &str) -> Result<(), String> {
        self.validate_dependencies()?;
        let card = self
            .get_card(card_id)
            .ok_or_else(|| format!("Card {card_id} not found"))?;
        for dependency in &card.depends_on {
            let dependency_card = self
                .get_card(dependency)
                .ok_or_else(|| format!("Card {card_id} dependency {dependency} does not exist"))?;
            if dependency_card.column != "done" {
                return Err(format!(
                    "Card {card_id} is blocked by dependency {dependency} in '{}'",
                    dependency_card.column
                ));
            }
        }
        Ok(())
    }

    pub fn authorize_owner(&mut self, chat_id: &str) -> Result<(), String> {
        match self.owner_chat_id.as_deref() {
            None => {
                self.owner_chat_id = Some(chat_id.to_string());
                Ok(())
            }
            Some(owner) if owner == chat_id => Ok(()),
            Some(_) => Err("task board mutation requires the owning planner chat".to_string()),
        }
    }

    pub fn require_owner(&self, chat_id: &str) -> Result<(), String> {
        match self.owner_chat_id.as_deref() {
            Some(owner) if owner == chat_id => Ok(()),
            Some(_) => Err("task board mutation requires the owning planner chat".to_string()),
            None => Err("task board has no owner; claim ownership first".to_string()),
        }
    }

    pub fn authorize_reviewer(&self, chat_id: &str, allow_self_review: bool) -> Result<(), String> {
        let owner = self.owner_chat_id.as_deref() == Some(chat_id);
        let assigned = self
            .reviewer_chat_ids
            .iter()
            .any(|reviewer| reviewer == chat_id);
        if assigned
            || (owner && allow_self_review && self.review_policy == BoardReviewPolicy::OwnerAllowed)
        {
            Ok(())
        } else if owner {
            Err("task review policy requires an independent assigned reviewer".to_string())
        } else {
            Err("task review requires the owner or an assigned reviewer".to_string())
        }
    }

    pub fn review_policy_allows_report(
        &self,
        card_id: &str,
        report: &VerifierReport,
        allow_self_review: bool,
    ) -> Result<(), String> {
        if allow_self_review && self.review_policy == BoardReviewPolicy::OwnerAllowed {
            return Ok(());
        }
        let attestation = self.review_attestations.get(card_id).ok_or_else(|| {
            "task review policy requires an independent reviewer report".to_string()
        })?;
        if &attestation.report != report {
            return Err("task review attestation no longer matches the stored report".to_string());
        }
        if self.owner_chat_id.as_deref() == Some(attestation.reviewer_chat_id.as_str()) {
            return Err("task review policy forbids owner self-review".to_string());
        }
        if !self
            .reviewer_chat_ids
            .iter()
            .any(|assigned| assigned == &attestation.reviewer_chat_id)
        {
            return Err(format!(
                "task reviewer {} is no longer assigned to this board",
                attestation.reviewer_chat_id
            ));
        }
        Ok(())
    }

    pub fn card_outcome(&self, card_id: &str) -> Option<&CardOutcome> {
        self.card_outcomes.get(card_id)
    }

    pub fn card_is_finalized(&self, card_id: &str) -> bool {
        self.card_outcomes
            .get(card_id)
            .is_some_and(|outcome| outcome.finalized.is_some())
    }

    /// Records what a run produced. Declared requirements come from the card
    /// instructions, so a run can never erase the requirements it must meet.
    pub fn record_card_outcome(
        &mut self,
        card_id: &str,
        required: Vec<OutcomeRequirement>,
        fragments: Vec<OutcomeFragment>,
    ) {
        let outcome = self.card_outcomes.entry(card_id.to_string()).or_default();
        outcome.required = required;
        for fragment in fragments {
            outcome.set_fragment(fragment);
        }
    }

    pub fn attach_card_evidence(&mut self, card_id: &str, fragment: &str, spec_ids: &[String]) {
        if spec_ids.is_empty() {
            return;
        }
        if let Some(outcome) = self.card_outcomes.get_mut(card_id) {
            outcome.attach_evidence(fragment, spec_ids);
        }
    }

    /// The finalize gate. Reuses the single review-authority path
    /// (`authorize_finalizer` plus `review_policy_allows_report`) and reports
    /// every unmet requirement in one answer.
    pub fn evaluate_finalize(
        &self,
        card_id: &str,
        principal: &str,
        allow_self_review: bool,
    ) -> FinalizeDecision {
        let mut blocks = Vec::new();
        let Some(card) = self.get_card(card_id) else {
            return FinalizeDecision {
                blocks: vec![FinalizeBlock::CardNotFound {
                    card_id: card_id.to_string(),
                }],
            };
        };
        if card.column != "done" {
            blocks.push(FinalizeBlock::NotCompleted {
                card_id: card_id.to_string(),
                column: card.column.clone(),
            });
        }
        let outcome = self.card_outcomes.get(card_id);
        if let Some(finalized) = outcome.and_then(|outcome| outcome.finalized.as_ref()) {
            blocks.push(FinalizeBlock::AlreadyFinalized {
                card_id: card_id.to_string(),
                finalized_by: finalized.finalized_by.clone(),
            });
        }
        if let Err(violation) = self.authorize_finalizer(card_id, principal, allow_self_review) {
            blocks.push(FinalizeBlock::ReviewDenied {
                reason: violation.message(),
            });
        }
        if self.owner_chat_id.is_some() {
            match card.verifier_report.as_ref() {
                Some(report) => {
                    if let Err(reason) =
                        self.review_policy_allows_report(card_id, report, allow_self_review)
                    {
                        blocks.push(FinalizeBlock::ReviewDenied { reason });
                    }
                }
                None => blocks.push(FinalizeBlock::ReviewDenied {
                    reason: "finalising requires a reviewer report for the card".to_string(),
                }),
            }
        }
        for requirement in outcome
            .map(|outcome| outcome.required.as_slice())
            .unwrap_or(&[])
        {
            let fragment = outcome.and_then(|outcome| outcome.fragment(&requirement.fragment));
            let Some(fragment) = fragment else {
                blocks.push(FinalizeBlock::MissingFragment {
                    fragment: requirement.fragment.clone(),
                });
                continue;
            };
            if !fragment.has_content() {
                blocks.push(FinalizeBlock::EmptyFragment {
                    fragment: requirement.fragment.clone(),
                });
            }
            let note = fragment
                .review_note
                .clone()
                .unwrap_or_else(|| "no reason recorded".to_string());
            match fragment.review {
                FragmentReview::Rejected => blocks.push(FinalizeBlock::RejectedFragment {
                    fragment: requirement.fragment.clone(),
                    reason: note,
                }),
                FragmentReview::Contradicted => blocks.push(FinalizeBlock::ContradictedFragment {
                    fragment: requirement.fragment.clone(),
                    reason: note,
                }),
                FragmentReview::Pending | FragmentReview::Approved => {}
            }
            if requirement.requires_evidence {
                if fragment
                    .provenance
                    .source_run
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(str::is_empty)
                {
                    blocks.push(FinalizeBlock::MissingSourceRun {
                        fragment: requirement.fragment.clone(),
                    });
                }
                if fragment.provenance.evidence_spec_ids.is_empty() {
                    blocks.push(FinalizeBlock::MissingEvidence {
                        fragment: requirement.fragment.clone(),
                    });
                }
            }
        }
        FinalizeDecision { blocks }
    }

    pub fn finalize_card(
        &mut self,
        card_id: &str,
        principal: &str,
        allow_self_review: bool,
        finalized_at: String,
    ) -> Result<(), FinalizeDecision> {
        let decision = self.evaluate_finalize(card_id, principal, allow_self_review);
        if !decision.allowed() {
            return Err(decision);
        }
        self.card_outcomes
            .entry(card_id.to_string())
            .or_default()
            .finalized = Some(CardFinalization {
            finalized_by: principal.to_string(),
            finalized_at,
        });
        Ok(())
    }

    pub fn clear_verifier_reports(&mut self) {
        self.review_attestations.clear();
        for card in &mut self.cards {
            card.verifier_report = None;
        }
        for outcome in self.card_outcomes.values_mut() {
            outcome.finalized = None;
        }
    }

    pub fn remove_reviewer(&mut self, reviewer_chat_id: &str) {
        self.reviewer_chat_ids
            .retain(|reviewer| reviewer != reviewer_chat_id);
        let stale = self
            .review_attestations
            .iter()
            .filter_map(|(card_id, attestation)| {
                (attestation.reviewer_chat_id == reviewer_chat_id).then(|| card_id.clone())
            })
            .collect::<Vec<_>>();
        for card_id in stale {
            self.invalidate_card_review(&card_id);
        }
    }

    pub fn invalidate_card_review(&mut self, card_id: &str) {
        self.review_attestations.remove(card_id);
        if let Some(outcome) = self.card_outcomes.get_mut(card_id) {
            outcome.finalized = None;
        }
        if let Some(card) = self.get_card_mut(card_id) {
            card.verifier_report = None;
        }
    }

    pub fn attest_review(
        &mut self,
        card_id: &str,
        reviewer_chat_id: &str,
        report: &VerifierReport,
        reviewed_at: String,
        allow_self_review: bool,
    ) -> Result<(), String> {
        self.authorize_reviewer(reviewer_chat_id, allow_self_review)?;
        self.review_attestations.insert(
            card_id.to_string(),
            BoardReviewAttestation {
                reviewer_chat_id: reviewer_chat_id.to_string(),
                reviewed_at,
                report: report.clone(),
            },
        );
        Ok(())
    }

    pub fn get_ready_cards(&self) -> ReadyCardsResult {
        let mut ready = vec![];
        let mut blocked = vec![];
        let mut in_progress = vec![];
        let mut completed = vec![];
        let mut failed = vec![];

        let done_cards: std::collections::HashSet<_> = self
            .cards
            .iter()
            .filter(|c| c.column == "done")
            .map(|c| c.id.as_str())
            .collect();

        for card in &self.cards {
            match card.column.as_str() {
                "done" => completed.push(card.id.clone()),
                "failed" | "regressed" => failed.push(card.id.clone()),
                "doing" => in_progress.push(card.id.clone()),
                "planned" => {
                    let deps_satisfied = card
                        .depends_on
                        .iter()
                        .all(|dep| done_cards.contains(dep.as_str()));
                    if deps_satisfied {
                        ready.push(card.id.clone());
                    } else {
                        blocked.push(card.id.clone());
                    }
                }
                _ => {}
            }
        }

        ReadyCardsResult {
            ready,
            blocked,
            in_progress,
            completed,
            failed,
        }
    }

    pub fn get_card(&self, card_id: &str) -> Option<&BoardCard> {
        self.cards.iter().find(|c| c.id == card_id)
    }

    pub fn get_card_mut(&mut self, card_id: &str) -> Option<&mut BoardCard> {
        self.cards.iter_mut().find(|c| c.id == card_id)
    }

    pub fn remove_card(&mut self, card_id: &str) -> Option<BoardCard> {
        self.review_attestations.remove(card_id);
        self.card_claims.remove(card_id);
        self.dependency_overrides.remove(card_id);
        self.card_outcomes.remove(card_id);
        self.cards
            .iter()
            .position(|card| card.id == card_id)
            .map(|index| self.cards.remove(index))
    }

    pub fn get_dependency_reports(&self, card_id: &str) -> Vec<(String, String)> {
        let card = match self.get_card(card_id) {
            Some(c) => c,
            None => return vec![],
        };

        card.depends_on
            .iter()
            .filter_map(|dep_id| {
                self.get_card(dep_id).and_then(|dep_card| {
                    dep_card
                        .final_report
                        .as_ref()
                        .map(|report| (dep_card.title.clone(), report.clone()))
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, title: &str, column: &str, depends_on: Vec<&str>) -> BoardCard {
        BoardCard {
            id: id.into(),
            title: title.into(),
            column: column.into(),
            priority: default_priority(),
            depends_on: depends_on.into_iter().map(String::from).collect(),
            instructions: String::new(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: "2026-05-16T00:00:00Z".into(),
            started_at: None,
            last_heartbeat_at: None,
            paused_at: None,
            paused_by: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: ScopeGuardMode::Off,
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }

    #[test]
    fn ready_cards_separate_ready_blocked_and_terminal_columns() {
        let board = TaskBoard {
            cards: vec![
                card("dep-done", "Dependency done", "done", vec![]),
                card("dep-failed", "Dependency failed", "failed", vec![]),
                card("ready", "Ready", "planned", vec!["dep-done"]),
                card("blocked", "Blocked", "planned", vec!["dep-failed"]),
                card(
                    "blocked-missing",
                    "Blocked missing",
                    "planned",
                    vec!["missing"],
                ),
                card("in-progress", "In progress", "doing", vec![]),
                card("regressed", "Regressed", "regressed", vec![]),
            ],
            ..TaskBoard::default()
        };

        let result = board.get_ready_cards();

        assert_eq!(result.ready, vec!["ready"]);
        assert_eq!(result.blocked, vec!["blocked", "blocked-missing"]);
        assert_eq!(result.in_progress, vec!["in-progress"]);
        assert_eq!(result.completed, vec!["dep-done"]);
        assert_eq!(result.failed, vec!["dep-failed", "regressed"]);
    }

    #[test]
    fn claim_is_compare_and_set_and_idempotent_for_the_same_key() {
        let mut board = TaskBoard {
            cards: vec![card("T-1", "Claimable", "planned", vec![])],
            ..TaskBoard::default()
        };

        assert_eq!(
            board.claim_card("T-1", "agent-a", "key-a", "t0".into()),
            Ok(ClaimOutcome::Claimed)
        );
        assert_eq!(
            board.claim_card("T-1", "agent-a", "key-a", "t1".into()),
            Ok(ClaimOutcome::AlreadyHeld)
        );
        assert_eq!(
            board.claim_card("T-1", "agent-b", "key-b", "t2".into()),
            Err(BoardRuleViolation::AlreadyClaimed {
                card_id: "T-1".into(),
                holder: "agent-a".into()
            })
        );
        assert_eq!(
            board.claim_card("T-1", "agent-a", "other-key", "t3".into()),
            Err(BoardRuleViolation::AlreadyClaimed {
                card_id: "T-1".into(),
                holder: "agent-a".into()
            })
        );
        assert_eq!(board.card_claim("T-1").unwrap().claimed_at, "t0");

        assert!(board.release_claim("T-1", "agent-b").is_err());
        board.release_claim("T-1", "agent-a").unwrap();
        assert!(board.card_claim("T-1").is_none());
        assert_eq!(
            board.claim_card("missing", "agent-a", "key", "t4".into()),
            Err(BoardRuleViolation::CardNotFound {
                card_id: "missing".into()
            })
        );
    }

    #[test]
    fn dependency_gate_blocks_doing_until_dependencies_are_done_or_overridden() {
        let mut board = TaskBoard {
            cards: vec![
                card("T-1", "Dependency", "planned", vec![]),
                card("T-2", "Dependent", "planned", vec!["T-1"]),
            ],
            ..TaskBoard::default()
        };

        let blocked = BoardRuleViolation::DependenciesUnsatisfied {
            card_id: "T-2".into(),
            unsatisfied: vec!["T-1".into()],
        };
        assert_eq!(
            board.ensure_column_move_allowed("T-2", "doing"),
            Err(blocked)
        );
        assert_eq!(
            board.claim_card("T-2", "agent-a", "key", "t0".into()),
            Err(BoardRuleViolation::DependenciesUnsatisfied {
                card_id: "T-2".into(),
                unsatisfied: vec!["T-1".into()]
            })
        );
        assert!(board.ensure_column_move_allowed("T-2", "failed").is_ok());
        assert!(board.ensure_column_move_allowed("T-2", "planned").is_ok());
        assert!(board.ensure_column_move_allowed("T-1", "doing").is_ok());

        assert_eq!(
            board
                .record_dependency_override("T-2", "planner-chat", "hotfix", "t1".into())
                .unwrap(),
            vec!["T-1".to_string()]
        );
        assert!(board.ensure_column_move_allowed("T-2", "doing").is_ok());
        let recorded = &board.dependency_overrides["T-2"];
        assert_eq!(recorded.overridden_by, "planner-chat");
        assert_eq!(recorded.reason, "hotfix");
        assert_eq!(recorded.unsatisfied, vec!["T-1".to_string()]);

        board.dependency_overrides.clear();
        board.get_card_mut("T-1").unwrap().column = "done".into();
        assert!(board.ensure_column_move_allowed("T-2", "doing").is_ok());
    }

    #[test]
    fn legacy_board_without_claim_fields_loads_and_round_trips_without_them() {
        let board: TaskBoard = serde_json::from_str(
            r#"{"cards":[{
                "id":"T-1","title":"Legacy","column":"planned",
                "created_at":"2026-05-16T00:00:00Z","started_at":null,
                "completed_at":null,"assignee":null,"agent_chat_id":null
            }]}"#,
        )
        .unwrap();

        assert!(board.card_claims.is_empty());
        assert!(board.dependency_overrides.is_empty());
        assert!(board.validate_for_write(256, 256).is_ok());
        assert!(board.ensure_column_move_allowed("T-1", "doing").is_ok());
        let encoded = serde_json::to_string(&board).unwrap();
        assert!(!encoded.contains("card_claims"));
        assert!(!encoded.contains("dependency_overrides"));
    }

    #[test]
    fn claim_validation_rejects_orphans_and_oversized_idempotency_keys() {
        let mut board = TaskBoard {
            cards: vec![card("T-1", "Claimable", "planned", vec![])],
            ..TaskBoard::default()
        };
        board
            .claim_card("T-1", "agent-a", "0123456789", "t0".into())
            .unwrap();

        assert!(board.validate_for_write(256, 10).is_ok());
        assert!(board
            .validate_for_write(256, 9)
            .unwrap_err()
            .contains("exceeds the configured maximum"));

        board.card_claims.insert(
            "ghost".into(),
            CardClaim {
                claimant_chat_id: "agent-a".into(),
                idempotency_key: "key".into(),
                claimed_at: "t0".into(),
            },
        );
        assert!(board
            .validate_for_write(256, 256)
            .unwrap_err()
            .contains("missing card ghost"));
    }

    #[test]
    fn reviewer_authorisation_requires_the_self_review_flag_for_the_owner() {
        let board = TaskBoard {
            cards: vec![card("T-1", "Reviewable", "doing", vec![])],
            owner_chat_id: Some("planner-chat".into()),
            reviewer_chat_ids: vec!["reviewer-chat".into()],
            ..TaskBoard::default()
        };

        assert!(board.authorize_reviewer("planner-chat", false).is_err());
        assert!(board.authorize_reviewer("planner-chat", true).is_ok());
        assert!(board.authorize_reviewer("reviewer-chat", false).is_ok());
        assert!(board.authorize_reviewer("stranger-chat", true).is_err());
    }

    fn passing_verifier_report() -> VerifierReport {
        VerifierReport {
            passed: true,
            recommendation: "merge".into(),
            classification: VerifierReportClassification::Passed,
            ..Default::default()
        }
    }

    fn reviewed_board() -> TaskBoard {
        let mut done = card("T-1", "Card", "done", vec![]);
        done.verifier_report = Some(passing_verifier_report());
        let mut board = TaskBoard {
            cards: vec![done],
            owner_chat_id: Some("planner-chat".into()),
            reviewer_chat_ids: vec!["reviewer-chat".into()],
            ..Default::default()
        };
        board
            .attest_review(
                "T-1",
                "reviewer-chat",
                &passing_verifier_report(),
                "2026-05-16T00:00:00Z".into(),
                false,
            )
            .unwrap();
        board
    }

    #[test]
    fn remove_card_prunes_metadata_before_reusing_a_finalized_id() {
        let mut board = reviewed_board();
        let mut other = board.get_card("T-1").unwrap().clone();
        other.id = "T-2".into();
        board.cards.push(other);
        for card_id in ["T-1", "T-2"] {
            board
                .attest_review(
                    card_id,
                    "reviewer-chat",
                    &passing_verifier_report(),
                    now(),
                    false,
                )
                .unwrap();
            board
                .claim_card(card_id, "agent-chat", "key", now())
                .unwrap();
            board
                .record_dependency_override(card_id, "planner-chat", "verified", now())
                .unwrap();
            board.record_card_outcome(
                card_id,
                vec![requirement("summary")],
                vec![fragment("summary", "Implemented.")],
            );
            board
                .finalize_card(card_id, "planner-chat", false, now())
                .unwrap();
        }
        let mut expected = serde_json::to_value(&board).unwrap();
        expected["cards"].as_array_mut().unwrap().remove(0);
        for map in [
            "review_attestations",
            "card_claims",
            "dependency_overrides",
            "card_outcomes",
        ] {
            expected[map].as_object_mut().unwrap().remove("T-1");
        }
        assert!(board.card_is_finalized("T-1"));

        let removed = board.remove_card("T-1").unwrap();

        assert_eq!(removed.id, "T-1");
        assert_eq!(removed.column, "done");
        assert!(removed.verifier_report.is_some());
        assert!(board.get_card("T-1").is_none());
        assert!(board.validate_for_write(256, 256).is_ok());
        board
            .cards
            .push(card("T-1", "Replacement", "planned", vec![]));

        assert!(!board.card_is_finalized("T-1"));
        assert!(board.card_claim("T-1").is_none());
        assert!(!board.dependency_overrides.contains_key("T-1"));
        assert!(!board.review_attestations.contains_key("T-1"));
        assert!(board.card_outcome("T-1").is_none());
        expected["cards"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(board.get_card("T-1").unwrap()).unwrap());
        assert_eq!(serde_json::to_value(&board).unwrap(), expected);
        assert!(board.validate_for_write(256, 256).is_ok());
        assert!(board.remove_card("missing").is_none());
        assert_eq!(serde_json::to_value(&board).unwrap(), expected);
    }

    #[test]
    fn remove_card_leaves_dependents_for_dependency_validation() {
        let mut board = TaskBoard {
            cards: vec![
                card("T-1", "Dependency", "done", vec![]),
                card("T-2", "Dependent", "planned", vec!["T-1"]),
            ],
            ..Default::default()
        };

        board.remove_card("T-1").unwrap();

        assert_eq!(board.get_card("T-2").unwrap().depends_on, vec!["T-1"]);
        assert_eq!(
            board.validate_for_write(256, 256),
            Err("Card T-2 dependency T-1 does not exist".into())
        );
    }

    fn fragment(name: &str, content: &str) -> OutcomeFragment {
        OutcomeFragment::new(name, content, Some("agent-chat-1".into()))
    }

    fn requirement(name: &str) -> OutcomeRequirement {
        OutcomeRequirement {
            fragment: name.into(),
            requires_evidence: false,
        }
    }

    #[test]
    fn a_missing_required_fragment_blocks_finalize_and_a_placeholder_never_satisfies_one() {
        let mut board = reviewed_board();
        board.record_card_outcome(
            "T-1",
            vec![requirement("summary"), requirement("files_changed")],
            vec![fragment("summary", "_None._")],
        );

        let decision = board.evaluate_finalize("T-1", "planner-chat", false);

        assert!(!decision.allowed());
        assert!(
            decision.blocks.contains(&FinalizeBlock::EmptyFragment {
                fragment: "summary".into()
            }),
            "the renderer's placeholder must never stand in for a required fragment: {:?}",
            decision.blocks
        );
        assert!(decision.blocks.contains(&FinalizeBlock::MissingFragment {
            fragment: "files_changed".into()
        }));
        assert!(board
            .finalize_card("T-1", "planner-chat", false, now())
            .is_err());
        assert!(!board.card_is_finalized("T-1"));
    }

    fn now() -> String {
        "2026-05-16T01:00:00Z".to_string()
    }

    #[test]
    fn a_rejected_required_fragment_blocks_finalize_even_when_another_is_approved() {
        let mut board = reviewed_board();
        board.record_card_outcome(
            "T-1",
            vec![requirement("summary"), requirement("verification")],
            vec![
                fragment("summary", "Implemented the gate."),
                fragment("verification", "- cargo test -p refact-tasks — passed"),
            ],
        );
        let outcome = board.card_outcomes.get_mut("T-1").unwrap();
        outcome
            .record_review(
                "summary",
                FragmentReview::Approved,
                "reviewer-chat",
                now(),
                None,
            )
            .unwrap();
        outcome
            .record_review(
                "verification",
                FragmentReview::Rejected,
                "reviewer-chat",
                now(),
                Some("the quoted run never happened".into()),
            )
            .unwrap();

        let decision = board.evaluate_finalize("T-1", "planner-chat", false);

        assert_eq!(
            decision.blocks,
            vec![FinalizeBlock::RejectedFragment {
                fragment: "verification".into(),
                reason: "the quoted run never happened".into()
            }],
            "an approved sibling fragment must not buy off a rejected required one"
        );
    }

    #[test]
    fn an_unresolved_contradicted_required_fragment_blocks_finalize() {
        let mut board = reviewed_board();
        board.record_card_outcome(
            "T-1",
            vec![requirement("summary")],
            vec![fragment("summary", "All tests pass.")],
        );
        board
            .card_outcomes
            .get_mut("T-1")
            .unwrap()
            .record_review(
                "summary",
                FragmentReview::Contradicted,
                "reviewer-chat",
                now(),
                Some("the verifier recorded a failing gate".into()),
            )
            .unwrap();

        let decision = board.evaluate_finalize("T-1", "planner-chat", false);

        assert_eq!(
            decision.blocks,
            vec![FinalizeBlock::ContradictedFragment {
                fragment: "summary".into(),
                reason: "the verifier recorded a failing gate".into()
            }]
        );
    }

    #[test]
    fn self_review_is_refused_by_default_and_permitted_only_with_the_setting() {
        let mut done = card("T-1", "Card", "done", vec![]);
        done.verifier_report = Some(passing_verifier_report());
        let mut board = TaskBoard {
            cards: vec![done],
            owner_chat_id: Some("planner-chat".into()),
            reviewer_chat_ids: vec!["agent-chat".into()],
            ..Default::default()
        };
        board
            .claim_card("T-1", "agent-chat", "key-1", now())
            .unwrap();

        let refused = board.evaluate_finalize("T-1", "agent-chat", false);

        assert!(refused.blocks.iter().any(|block| matches!(
            block,
            FinalizeBlock::ReviewDenied { reason } if reason.contains("self-review is refused")
        )));
        assert!(board.evaluate_finalize("T-1", "agent-chat", true).allowed());
    }

    #[test]
    fn independent_required_still_forbids_owner_self_attestation() {
        let mut board = reviewed_board();
        board
            .attest_review(
                "T-1",
                "planner-chat",
                &passing_verifier_report(),
                now(),
                true,
            )
            .unwrap();
        board.review_policy = BoardReviewPolicy::IndependentRequired;

        let decision = board.evaluate_finalize("T-1", "planner-chat", true);

        assert!(decision.blocks.iter().any(|block| matches!(
            block,
            FinalizeBlock::ReviewDenied { reason } if reason.contains("forbids owner self-review")
        )));
    }

    #[test]
    fn a_blocked_finalize_names_every_unmet_requirement_at_once() {
        let mut doing = card("T-1", "Card", "doing", vec![]);
        doing.verifier_report = Some(passing_verifier_report());
        let mut board = TaskBoard {
            cards: vec![doing],
            owner_chat_id: Some("planner-chat".into()),
            reviewer_chat_ids: vec!["reviewer-chat".into()],
            ..Default::default()
        };
        board
            .attest_review(
                "T-1",
                "reviewer-chat",
                &passing_verifier_report(),
                now(),
                false,
            )
            .unwrap();
        board.record_card_outcome(
            "T-1",
            vec![
                requirement("summary"),
                OutcomeRequirement {
                    fragment: "verification".into(),
                    requires_evidence: true,
                },
            ],
            vec![OutcomeFragment::new(
                "verification",
                "- cargo test — passed",
                None,
            )],
        );

        let decision = board.evaluate_finalize("T-1", "stranger-chat", false);

        assert_eq!(
            decision.blocks,
            vec![
                FinalizeBlock::NotCompleted {
                    card_id: "T-1".into(),
                    column: "doing".into()
                },
                FinalizeBlock::ReviewDenied {
                    reason: "card T-1: finalising requires the board owner or an assigned reviewer"
                        .into()
                },
                FinalizeBlock::MissingFragment {
                    fragment: "summary".into()
                },
                FinalizeBlock::MissingSourceRun {
                    fragment: "verification".into()
                },
                FinalizeBlock::MissingEvidence {
                    fragment: "verification".into()
                },
            ],
            "a blocked finalize is a gate answer, not a first-failure notification"
        );
        let message = decision.message();
        assert!(message.contains("5 unmet requirement"));
        assert!(message.contains("'summary' is missing"));
        assert!(message.contains("references no evidence record"));
    }

    #[test]
    fn done_stays_agent_declared_and_finalized_is_a_separate_state() {
        let mut board = reviewed_board();
        board.record_card_outcome(
            "T-1",
            vec![requirement("summary")],
            vec![fragment("summary", "Implemented the gate.")],
        );

        assert_eq!(board.get_card("T-1").unwrap().column, "done");
        assert!(!board.card_is_finalized("T-1"));

        board
            .finalize_card("T-1", "planner-chat", false, now())
            .unwrap();

        assert_eq!(board.get_card("T-1").unwrap().column, "done");
        assert!(board.card_is_finalized("T-1"));
        assert_eq!(
            board.card_outcome("T-1").unwrap().finalized,
            Some(CardFinalization {
                finalized_by: "planner-chat".into(),
                finalized_at: now()
            })
        );
        assert!(board
            .finalize_card("T-1", "planner-chat", false, now())
            .unwrap_err()
            .blocks
            .contains(&FinalizeBlock::AlreadyFinalized {
                card_id: "T-1".into(),
                finalized_by: "planner-chat".into()
            }));
    }

    #[test]
    fn an_outcome_with_no_declared_requirements_finalizes_as_before() {
        let mut board = TaskBoard {
            cards: vec![card("T-1", "Card", "done", vec![])],
            ..Default::default()
        };

        assert!(board.evaluate_finalize("T-1", "any-chat", false).allowed());
        assert!(board.finalize_card("T-1", "any-chat", false, now()).is_ok());
    }

    #[test]
    fn evidence_provenance_is_required_only_where_it_was_declared() {
        let mut board = reviewed_board();
        board.record_card_outcome(
            "T-1",
            vec![
                requirement("summary"),
                OutcomeRequirement {
                    fragment: "verification".into(),
                    requires_evidence: true,
                },
            ],
            vec![
                fragment("summary", "Implemented the gate."),
                fragment("verification", "- cargo test -p refact-tasks — passed"),
            ],
        );

        assert_eq!(
            board.evaluate_finalize("T-1", "planner-chat", false).blocks,
            vec![FinalizeBlock::MissingEvidence {
                fragment: "verification".into()
            }]
        );

        board.attach_card_evidence("T-1", "verification", &["T-1#0".to_string()]);

        assert!(board
            .evaluate_finalize("T-1", "planner-chat", false)
            .allowed());
        assert_eq!(
            board
                .card_outcome("T-1")
                .unwrap()
                .fragment("verification")
                .unwrap()
                .provenance
                .evidence_spec_ids,
            vec!["T-1#0".to_string()]
        );
    }

    #[test]
    fn invalidating_a_card_review_unfinalizes_it() {
        let mut board = reviewed_board();
        board.record_card_outcome("T-1", vec![], vec![fragment("summary", "Done.")]);
        board
            .finalize_card("T-1", "planner-chat", false, now())
            .unwrap();

        board.invalidate_card_review("T-1");

        assert!(!board.card_is_finalized("T-1"));
        assert!(board
            .card_outcome("T-1")
            .unwrap()
            .fragment("summary")
            .is_some());
    }

    #[test]
    fn required_outcome_fragments_are_declared_in_card_instructions() {
        let requirements = outcome_requirements_from_instructions(
            "## Requirements\n- **Required outcome:** Summary, files changed, `verification`\n- Required outcome evidence: verification\n",
        );

        assert_eq!(
            requirements,
            vec![
                OutcomeRequirement {
                    fragment: "summary".into(),
                    requires_evidence: false
                },
                OutcomeRequirement {
                    fragment: "files_changed".into(),
                    requires_evidence: false
                },
                OutcomeRequirement {
                    fragment: "verification".into(),
                    requires_evidence: true
                },
            ]
        );
        assert!(outcome_requirements_from_instructions("no declaration here").is_empty());
    }

    #[test]
    fn a_final_report_becomes_fragments_that_name_the_run_that_produced_them() {
        let report = FinalReport {
            summary: "Implemented the gate.".into(),
            success: true,
            files_changed: vec!["types.rs".into()],
            verification: vec![VerificationResult {
                command: "cargo test -p refact-tasks".into(),
                exit_code: Some(0),
                passed: true,
                ..Default::default()
            }],
            ..Default::default()
        };

        let fragments = report.to_outcome_fragments(Some("agent-chat-1"));

        assert_eq!(
            fragments
                .iter()
                .map(|fragment| fragment.name.as_str())
                .collect::<Vec<_>>(),
            vec!["summary", "files_changed", "verification"],
            "sections the run did not provide must not appear as empty fragments"
        );
        assert!(fragments
            .iter()
            .all(|fragment| fragment.provenance.source_run.as_deref() == Some("agent-chat-1")));
        assert_eq!(
            fragments[2].content,
            "- cargo test -p refact-tasks — passed (exit code 0)"
        );
        assert!(FinalReport::default()
            .to_outcome_fragments(Some("agent-chat-1"))
            .is_empty());
    }
}
