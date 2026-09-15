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

    pub fn clear_verifier_reports(&mut self) {
        self.review_attestations.clear();
        for card in &mut self.cards {
            card.verifier_report = None;
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
        }
    }

    #[test]
    fn default_board_has_schema_and_columns() {
        let board = TaskBoard::default();

        assert_eq!(board.schema_version, 1);
        assert_eq!(board.rev, 0);
        assert!(board.cards.is_empty());
        assert_eq!(
            board
                .columns
                .iter()
                .map(|column| (column.id.as_str(), column.title.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("planned", "Planned"),
                ("doing", "Doing"),
                ("done", "Done"),
                ("failed", "Failed"),
                ("regressed", "Regressed"),
            ]
        );
    }

    #[test]
    fn serde_defaults_preserve_schema_values() {
        let meta: TaskMeta = serde_json::from_str(
            r#"{
                "id": "task-1",
                "name": "Task One",
                "status": "active",
                "created_at": "created",
                "updated_at": "updated"
            }"#,
        )
        .unwrap();
        let board: TaskBoard = serde_json::from_str(r#"{"cards": []}"#).unwrap();

        assert_eq!(meta.schema_version, 1);
        assert_eq!(meta.cards_total, 0);
        assert_eq!(meta.cards_done, 0);
        assert_eq!(meta.cards_failed, 0);
        assert_eq!(meta.agents_active, 0);
        assert!(!meta.is_name_generated);
        assert_eq!(board.schema_version, 1);
        assert_eq!(board.rev, 0);
        assert_eq!(board.columns.len(), 5);
    }

    #[test]
    fn deserialize_old_card_json_without_structured_report() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "done",
                "depends_on": [],
                "instructions": "",
                "assignee": null,
                "agent_chat_id": null,
                "status_updates": [],
                "final_report": "legacy markdown report",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": "2026-05-16T01:00:00Z"
            }"#,
        )
        .unwrap();

        assert_eq!(card.final_report.as_deref(), Some("legacy markdown report"));
        assert!(card.final_report_structured.is_none());
        assert!(card.verifier_report.is_none());
        assert!(card.ab_variants.is_none());
        assert!(card.comments.is_empty());
    }

    #[test]
    fn board_card_without_comments_deserializes_with_empty_vec() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "planned",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": null,
                "assignee": null,
                "agent_chat_id": null
            }"#,
        )
        .unwrap();

        assert!(card.comments.is_empty());
    }

    #[test]
    fn ab_variants_field_defaults_to_none() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "planned",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": null,
                "assignee": null,
                "agent_chat_id": null
            }"#,
        )
        .unwrap();

        assert!(card.ab_variants.is_none());
    }

    #[test]
    fn spawn_ab_legacy_ab_variants_deserialize_without_finish_state() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "A/B legacy card",
                "column": "doing",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": "2026-05-16T00:00:00Z",
                "completed_at": null,
                "assignee": "ab",
                "agent_chat_id": null,
                "ab_variants": {
                    "a": {
                        "agent_id": "agent-a",
                        "chat_id": "chat-a",
                        "worktree": "/tmp/a",
                        "worktree_name": "wt-a",
                        "branch": "branch-a",
                        "model": "model-a"
                    },
                    "b": {
                        "agent_id": "agent-b",
                        "chat_id": "chat-b",
                        "worktree": "/tmp/b",
                        "worktree_name": "wt-b",
                        "branch": "branch-b",
                        "model": "model-b"
                    }
                }
            }"#,
        )
        .unwrap();

        let variants = card.ab_variants.unwrap();
        assert!(variants.winner.is_none());
        assert!(variants.a.finish.is_none());
        assert!(variants.b.finish.is_none());
        assert_eq!(variants.variant_key_for_chat_id("chat-a"), Some("a"));
        assert_eq!(variants.variant_key_for_chat_id("chat-b"), Some("b"));
    }

    #[test]
    fn scope_guard_mode_defaults_to_off() {
        assert_eq!(ScopeGuardMode::default(), ScopeGuardMode::Off);
    }

    #[test]
    fn board_card_without_scope_guard_field_deserializes() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "planned",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": null,
                "assignee": null,
                "agent_chat_id": null
            }"#,
        )
        .unwrap();

        assert_eq!(card.scope_guard_mode, ScopeGuardMode::Off);
    }

    #[test]
    fn board_card_without_team_members_deserializes_with_empty_vec() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "planned",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": null,
                "assignee": null,
                "agent_chat_id": null
            }"#,
        )
        .unwrap();

        assert!(card.team_members.is_empty());
    }

    #[test]
    fn empty_team_members_are_skipped_in_serialized_yaml_output() {
        let card = card("T-1", "Schema card", "planned", vec![]);

        let output = serde_json::to_string(&card).unwrap();

        assert!(!output.contains("team_members"));
    }

    #[test]
    fn team_members_round_trip_three_entries() {
        let mut card = card("T-1", "Team card", "doing", vec![]);
        card.team_members = vec![
            TeamMember {
                role: "implementer".into(),
                agent_chat_id: Some("chat-implementer".into()),
                agent_branch: Some("team/implementer".into()),
                status: Some("doing".into()),
            },
            TeamMember {
                role: "tester".into(),
                agent_chat_id: Some("chat-tester".into()),
                agent_branch: Some("team/tester".into()),
                status: Some("idle".into()),
            },
            TeamMember {
                role: "reviewer".into(),
                agent_chat_id: None,
                agent_branch: None,
                status: Some("done".into()),
            },
        ];

        let encoded = serde_json::to_string(&card).unwrap();
        let decoded: BoardCard = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.team_members.len(), 3);
        assert_eq!(decoded.team_members[0].role, "implementer");
        assert_eq!(
            decoded.team_members[0].agent_chat_id.as_deref(),
            Some("chat-implementer")
        );
        assert_eq!(
            decoded.team_members[0].agent_branch.as_deref(),
            Some("team/implementer")
        );
        assert_eq!(decoded.team_members[0].status.as_deref(), Some("doing"));
        assert_eq!(decoded.team_members[1].role, "tester");
        assert_eq!(
            decoded.team_members[1].agent_chat_id.as_deref(),
            Some("chat-tester")
        );
        assert_eq!(
            decoded.team_members[1].agent_branch.as_deref(),
            Some("team/tester")
        );
        assert_eq!(decoded.team_members[1].status.as_deref(), Some("idle"));
        assert_eq!(decoded.team_members[2].role, "reviewer");
        assert!(decoded.team_members[2].agent_chat_id.is_none());
        assert!(decoded.team_members[2].agent_branch.is_none());
        assert_eq!(decoded.team_members[2].status.as_deref(), Some("done"));
    }

    #[test]
    fn final_report_round_trips_json() {
        let report = FinalReport {
            summary: "Implemented structured reports".into(),
            success: true,
            files_changed: vec!["refact-agent/engine/src/tools/tool_task_agent_finish.rs".into()],
            tests_added_or_updated: vec!["final_report_round_trips_json".into()],
            verification: vec![VerificationResult {
                command: "cargo test --lib -p refact-tasks".into(),
                exit_code: Some(0),
                passed: true,
                output_tail: "test result: ok".into(),
                outcome: VerificationOutcome::Unknown,
                from_cache: false,
                executed_at: None,
            }],
            followup_cards: vec![SuggestedCard {
                title: "Render structured reports in GUI".into(),
                instructions: "Prefer final_report_structured when present.".into(),
                priority: "P2".into(),
                target_files: vec!["refact-agent/gui/src/features/Tasks".into()],
            }],
            risks: vec!["Older consumers still read markdown.".into()],
            assumptions: vec!["Legacy report remains populated.".into()],
        };

        let encoded = serde_json::to_string(&report).unwrap();
        let decoded: FinalReport = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, report);
    }

    #[test]
    fn verifier_report_round_trips_json() {
        let report = VerifierReport {
            passed: false,
            command_results: vec![VerificationResult {
                command: "cargo test --lib -p refact-lsp -- verifier".into(),
                exit_code: Some(1),
                passed: false,
                output_tail: "test failed".into(),
                outcome: VerificationOutcome::Unknown,
                from_cache: false,
                executed_at: None,
            }],
            concerns: vec!["Diff removes a required guard".into()],
            recommendation: "fix-needed".into(),
            classification: VerifierReportClassification::VerificationFailed,
        };

        let encoded = serde_json::to_string(&report).unwrap();
        let decoded: VerifierReport = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, report);
    }

    #[test]
    fn legacy_verification_metadata_defaults_to_unknown() {
        let result: VerificationResult = serde_json::from_str(
            r#"{"command":"cargo test","exit_code":0,"passed":true,"output_tail":"ok"}"#,
        )
        .unwrap();
        let report: VerifierReport = serde_json::from_str(
            r#"{"passed":false,"command_results":[],"concerns":[],"recommendation":"human-review"}"#,
        )
        .unwrap();

        assert_eq!(result.outcome, VerificationOutcome::Unknown);
        assert_eq!(report.classification, VerifierReportClassification::Unknown);
    }

    #[test]
    fn final_report_to_markdown_renders_all_sections() {
        let report = FinalReport {
            summary: "Summary text".into(),
            success: false,
            files_changed: vec!["src/lib.rs".into()],
            tests_added_or_updated: vec!["report test".into()],
            verification: vec![VerificationResult {
                command: "cargo test".into(),
                exit_code: Some(1),
                passed: false,
                output_tail: "failure tail".into(),
                outcome: VerificationOutcome::Unknown,
                from_cache: false,
                executed_at: None,
            }],
            followup_cards: vec![SuggestedCard {
                title: "Fix follow-up".into(),
                instructions: "Do the next thing.".into(),
                priority: "P1".into(),
                target_files: vec!["src/followup.rs".into()],
            }],
            risks: vec!["Risk item".into()],
            assumptions: vec!["Assumption item".into()],
        };

        let markdown = report.to_markdown();

        assert!(markdown.contains("## Summary\nSummary text"));
        assert!(markdown.contains("## Result\n- Success: false"));
        assert!(markdown.contains("## Files Changed\n- src/lib.rs"));
        assert!(markdown.contains("## Tests Added or Updated\n- report test"));
        assert!(markdown.contains("## Verification\n- `cargo test` — failed (exit code 1)"));
        assert!(markdown.contains("failure tail"));
        assert!(markdown.contains("## Follow-up Cards\n### Fix follow-up"));
        assert!(markdown.contains("- Priority: P1"));
        assert!(markdown.contains("- Target files: src/followup.rs"));
        assert!(markdown.contains("Do the next thing."));
        assert!(markdown.contains("## Risks\n- Risk item"));
        assert!(markdown.contains("## Assumptions\n- Assumption item"));
    }

    #[test]
    fn final_report_to_markdown_escapes_command_backticks() {
        let report = FinalReport {
            verification: vec![VerificationResult {
                command: "cargo test -- --exact `final_report`".into(),
                exit_code: Some(0),
                passed: true,
                output_tail: String::new(),
                outcome: VerificationOutcome::Unknown,
                from_cache: false,
                executed_at: None,
            }],
            ..Default::default()
        };

        let markdown = report.to_markdown();

        assert!(markdown.contains("- `` cargo test -- --exact `final_report` `` — passed"));
        assert!(!markdown.contains("- `cargo test -- --exact `final_report`` — passed"));
    }

    #[test]
    fn final_report_to_markdown_uses_safe_output_fence() {
        let report = FinalReport {
            verification: vec![VerificationResult {
                command: "cargo test".into(),
                exit_code: Some(1),
                passed: false,
                output_tail: "before\n```\n## Fake Section\n```\nafter".into(),
                outcome: VerificationOutcome::Unknown,
                from_cache: false,
                executed_at: None,
            }],
            ..Default::default()
        };

        let markdown = report.to_markdown();

        assert!(markdown.contains("\n````text\nbefore\n```\n## Fake Section\n```\nafter\n````\n"));
        assert!(!markdown.contains("\n```text\nbefore\n```\n## Fake Section"));
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
    fn dependency_reports_include_only_dependencies_with_reports() {
        let mut reported = card("dep-reported", "Reported dependency", "done", vec![]);
        reported.final_report = Some("finished cleanly".into());
        let unreported = card("dep-unreported", "Unreported dependency", "done", vec![]);
        let consumer = card(
            "consumer",
            "Consumer",
            "planned",
            vec!["dep-reported", "dep-unreported", "missing"],
        );
        let board = TaskBoard {
            cards: vec![reported, unreported, consumer],
            ..TaskBoard::default()
        };

        assert_eq!(
            board.get_dependency_reports("consumer"),
            vec![("Reported dependency".into(), "finished cleanly".into())]
        );
        assert!(board.get_dependency_reports("missing").is_empty());
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
    fn finalising_validates_roles_and_refuses_self_review_by_default() {
        let mut board = TaskBoard {
            cards: vec![card("T-1", "Reviewable", "doing", vec![])],
            owner_chat_id: Some("planner-chat".into()),
            reviewer_chat_ids: vec!["reviewer-chat".into()],
            ..TaskBoard::default()
        };
        board
            .claim_card("T-1", "agent-chat", "key", "t0".into())
            .unwrap();

        assert!(board
            .authorize_finalizer("T-1", "stranger-chat", false)
            .is_err());
        assert!(board
            .authorize_finalizer("T-1", "agent-chat", false)
            .is_err());
        assert!(board
            .authorize_finalizer("T-1", "planner-chat", false)
            .is_ok());
        assert!(board
            .authorize_finalizer("T-1", "reviewer-chat", false)
            .is_ok());

        board.reviewer_chat_ids.push("agent-chat".into());
        assert_eq!(
            board.authorize_finalizer("T-1", "agent-chat", false),
            Err(BoardRuleViolation::RoleDenied {
                card_id: "T-1".into(),
                reason: "self-review is refused; enable coordination.board_allow_self_review to permit the claimant to finalise its own card".into()
            })
        );
        assert!(board.authorize_finalizer("T-1", "agent-chat", true).is_ok());
    }

    #[test]
    fn board_without_owner_keeps_legacy_finalising_behaviour() {
        let board = TaskBoard {
            cards: vec![card("T-1", "Legacy", "doing", vec![])],
            ..TaskBoard::default()
        };

        assert!(board.authorize_finalizer("T-1", "any-chat", false).is_ok());
        assert!(board
            .authorize_finalizer("missing", "any-chat", false)
            .is_err());
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

    #[test]
    fn legacy_card_serde_defaults_card_base_refs() {
        let original = card("legacy", "Legacy", "planned", vec![]);
        let mut json = serde_json::to_value(&original).unwrap();
        json.as_object_mut().unwrap().remove("base_branch");
        json.as_object_mut().unwrap().remove("base_commit");

        let decoded: BoardCard = serde_json::from_value(json).unwrap();

        assert!(decoded.base_branch.is_none());
        assert!(decoded.base_commit.is_none());
    }
}
