//! One typed assignment record: TASK, DELIVERABLE, SCOPE, VERIFY.
//!
//! The record is built once at dispatch, rendered into the prompt the agent receives,
//! and stored beside the board.
//! TASK stays prose verbatim; DELIVERABLE reuses `DeliverableContract`; SCOPE reuses
//! `ScopeGuardMode` and reports whether `target_files` is declared intent or enforced;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use refact_chat_api::DeliverableContract;

use crate::chat::evidence::{specs_from_card, EvidenceDefaults, VerificationSpec};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, ScopeGuardMode};

const ASSIGNMENTS_DIR: &str = "assignments";
const MAX_CARD_ID_LEN: usize = 64;

/// Whether the declared `target_files` are merely intent or are actually refused
/// at write time. Only `Strict` enforces; everything else is a declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeEnforcement {
    Declared,
    Enforced,
}

impl ScopeEnforcement {
    pub fn label(self) -> &'static str {
        match self {
            ScopeEnforcement::Declared => "declared",
            ScopeEnforcement::Enforced => "enforced",
        }
    }

    pub fn is_enforced(self) -> bool {
        matches!(self, ScopeEnforcement::Enforced)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AssignmentScope {
    #[serde(default)]
    pub target_files: Vec<String>,
    #[serde(default)]
    pub guard: ScopeGuardMode,
}

impl AssignmentScope {
    pub fn enforcement(&self) -> ScopeEnforcement {
        match self.guard {
            ScopeGuardMode::Strict => ScopeEnforcement::Enforced,
            ScopeGuardMode::Off | ScopeGuardMode::Warn | ScopeGuardMode::Reject => {
                ScopeEnforcement::Declared
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub card_id: String,
    pub title: String,
    pub task: String,
    #[serde(default)]
    pub dependency_context: String,
    #[serde(default)]
    pub suggested_steps: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliverable: Option<DeliverableContract>,
    #[serde(default)]
    pub scope: AssignmentScope,
    #[serde(default)]
    pub verify: Vec<VerificationSpec>,
    pub dispatched_at: String,
}

impl Assignment {
    pub fn new(
        card_id: impl Into<String>,
        title: impl Into<String>,
        task: impl Into<String>,
    ) -> Self {
        Self {
            card_id: card_id.into(),
            title: title.into(),
            task: task.into(),
            dependency_context: String::new(),
            suggested_steps: 0,
            deliverable: None,
            scope: AssignmentScope::default(),
            verify: Vec::new(),
            dispatched_at: Utc::now().to_rfc3339(),
        }
    }

    pub fn from_card(card: &BoardCard) -> Self {
        let mut assignment = Self::new(&card.id, &card.title, &card.instructions);
        assignment.scope = AssignmentScope {
            target_files: card.target_files.clone(),
            guard: card.scope_guard_mode,
        };
        match typed_parts(card) {
            Ok((deliverable, verification)) => assignment
                .with_deliverable(deliverable)
                .with_verification(verification),
            Err(error) => {
                tracing::warn!(card_id = %card.id, %error, "ignoring malformed typed assignment data");
                assignment
            }
        }
    }

    pub fn with_dependency_context(mut self, dependency_context: impl Into<String>) -> Self {
        self.dependency_context = dependency_context.into();
        self
    }

    pub fn with_suggested_steps(mut self, suggested_steps: usize) -> Self {
        self.suggested_steps = suggested_steps;
        self
    }

    pub fn with_scope(mut self, scope: AssignmentScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn with_deliverable(mut self, deliverable: Option<DeliverableContract>) -> Self {
        self.deliverable = deliverable.filter(|contract| !contract.is_empty());
        self
    }

    pub fn with_verification(mut self, verify: Vec<VerificationSpec>) -> Self {
        self.verify = verify;
        self
    }

    pub fn has_typed_verification(&self) -> bool {
        !self.verify.is_empty()
    }

    pub fn render_prompt(&self) -> String {
        let mut prompt = format!("# Card: {}\n\n## Instructions\n{}", self.title, self.task);
        if !self.dependency_context.is_empty() {
            prompt.push_str("\n\n## Context from Dependencies\n");
            prompt.push_str(&self.dependency_context);
        }
        if let Some(deliverable) = self.deliverable.as_ref() {
            prompt.push_str("\n\n");
            prompt.push_str(&deliverable.render_instructions());
        }
        prompt.push_str(&format!(
            "\n\n## Guidelines\n- Suggested step budget: ~{} steps\n- Focus only on this specific card\n- Report progress clearly\n- **Remember to call `agent_finish()` when done!**",
            self.suggested_steps
        ));
        prompt
    }
}

pub fn typed_parts(
    card: &BoardCard,
) -> Result<(Option<DeliverableContract>, Vec<VerificationSpec>), String> {
    let deliverable = card
        .deliverable
        .as_ref()
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|error| format!("card {} deliverable: {error}", card.id))
        })
        .transpose()?;
    let verification = card
        .verification
        .iter()
        .enumerate()
        .map(|(index, value)| {
            serde_json::from_value(value.clone())
                .map_err(|error| format!("card {} verification[{index}]: {error}", card.id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((deliverable, verification))
}

pub fn specs_for_validation(
    assignment: Option<&Assignment>,
    card: &BoardCard,
    defaults: &EvidenceDefaults,
) -> Vec<VerificationSpec> {
    let mut specs = assignment
        .filter(|assignment| assignment.has_typed_verification())
        .map(|assignment| assignment.verify.clone())
        .unwrap_or_default();
    for mut spec in specs_from_card(card, defaults) {
        if specs
            .iter()
            .any(|existing| existing.command == spec.command)
        {
            continue;
        }
        while specs
            .iter()
            .any(|existing| existing.spec_id == spec.spec_id)
        {
            spec.spec_id.push_str(":live");
        }
        specs.push(spec);
    }
    specs
}

fn assignment_path(task_dir: &Path, card_id: &str) -> Result<PathBuf, String> {
    if card_id.is_empty() || card_id.len() > MAX_CARD_ID_LEN {
        return Err(format!("card id '{card_id}' cannot address an assignment"));
    }
    if !card_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "card id '{card_id}' contains characters that cannot address an assignment"
        ));
    }
    Ok(task_dir
        .join(ASSIGNMENTS_DIR)
        .join(format!("{card_id}.json")))
}

pub async fn save_assignment(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    assignment: &Assignment,
) -> Result<(), String> {
    let (task_dir, _guard) = storage::locked_task_dir(gcx, task_id).await?;
    let path = assignment_path(&task_dir, &assignment.card_id)?;
    let parent = path
        .parent()
        .ok_or_else(|| "assignment path has no parent directory".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| error.to_string())?;
    let canonical_parent = dunce::canonicalize(parent).map_err(|error| error.to_string())?;
    if canonical_parent != parent {
        return Err("Assignment directory is an unsafe alias".into());
    }
    if tokio::fs::symlink_metadata(&path).await.is_ok() {
        storage::require_regular_task_file(&path).await?;
    }
    let content = serde_json::to_vec_pretty(assignment).map_err(|error| error.to_string())?;
    crate::file_activity::write_file_durably(
        &path,
        &content,
        crate::file_activity::ReplaceMode::Replace,
    )
    .await?;
    Ok(())
}

/// Store the record a dispatch just handed out. A storage failure leaves no record,
/// so validation falls back to the card instead of checking a contract nobody sent.
pub async fn record_assignment(gcx: Arc<GlobalContext>, task_id: &str, assignment: &Assignment) {
    if let Err(error) = save_assignment(gcx, task_id, assignment).await {
        tracing::warn!(
            task_id = %task_id,
            card_id = %assignment.card_id,
            "assignment not recorded, validation falls back to the card: {error}"
        );
    }
}

/// Drop a recorded assignment for a dispatch that has no single contract, such as an
/// A/B run whose variants received different prose.
pub async fn clear_assignment(gcx: Arc<GlobalContext>, task_id: &str, card_id: &str) {
    let removed = async {
        let (task_dir, _guard) = storage::locked_task_dir(gcx, task_id).await?;
        let path = assignment_path(&task_dir, card_id)?;
        let parent = path.parent().ok_or("Assignment parent missing")?;
        if parent.exists()
            && dunce::canonicalize(parent).map_err(|error| error.to_string())? != parent
        {
            return Err("Assignment directory is an unsafe alias".into());
        }
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
    .await;
    if let Err(error) = removed {
        tracing::warn!(
            task_id = %task_id,
            card_id = %card_id,
            "assignment record not cleared: {error}"
        );
    }
}

pub async fn load_assignment(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
) -> Result<Option<Assignment>, String> {
    let task_dir = storage::find_task_dir(gcx, task_id).await?;
    let Ok(path) = assignment_path(&task_dir, card_id) else {
        return Ok(None);
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| format!("assignment for card {card_id} is unreadable: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::evidence::SpecAuthority;
    use crate::tasks::types::{FinalReport, VerificationResult};

    fn defaults() -> EvidenceDefaults {
        EvidenceDefaults {
            timeout_secs: 60,
            output_cap_bytes: 4_000,
        }
    }

    fn card(instructions: &str) -> BoardCard {
        let mut card: BoardCard = serde_json::from_value(serde_json::json!({
            "id": "T-1",
            "title": "Typed assignment",
            "column": "doing",
            "created_at": "2026-05-16T00:00:00Z",
            "started_at": null,
            "completed_at": null,
            "assignee": null,
            "agent_chat_id": "agent-1"
        }))
        .unwrap();
        card.instructions = instructions.to_string();
        card
    }

    async fn setup_workspace(root: &Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    #[test]
    fn scope_reports_declared_intent_unless_the_guard_enforces_it() {
        for guard in [
            ScopeGuardMode::Off,
            ScopeGuardMode::Warn,
            ScopeGuardMode::Reject,
        ] {
            let mut card = card("## Verification\n- cargo test --lib");
            card.target_files = vec!["src/lib.rs".to_string()];
            card.scope_guard_mode = guard;

            let assignment = Assignment::from_card(&card);

            assert_eq!(
                assignment.scope.target_files,
                vec!["src/lib.rs".to_string()]
            );
            assert_eq!(
                assignment.scope.enforcement(),
                ScopeEnforcement::Declared,
                "{guard:?} declares intent, it does not enforce it"
            );
            assert!(!assignment.scope.enforcement().is_enforced());
            assert_eq!(assignment.scope.enforcement().label(), "declared");
        }

        let mut strict = card("## Verification\n- cargo test --lib");
        strict.target_files = vec!["src/lib.rs".to_string()];
        strict.scope_guard_mode = ScopeGuardMode::Strict;

        let assignment = Assignment::from_card(&strict);

        assert_eq!(assignment.scope.enforcement(), ScopeEnforcement::Enforced);
        assert!(assignment.scope.enforcement().is_enforced());
        assert_eq!(assignment.scope.enforcement().label(), "enforced");
    }

    #[test]
    fn a_typed_verification_spec_is_preserved_alongside_live_verify_lines() {
        let mut card = card("## Verification\n- Verify: `cargo test --lib`");
        card.target_files = vec!["src/lib.rs".to_string()];
        let mut typed = VerificationSpec::propose(
            "T-1:typed",
            "cargo test --lib -- assignment",
            SpecAuthority::CardInstructions,
            &defaults(),
        );
        typed.cache.deterministic_inputs = vec!["src".to_string(), "Cargo.toml".to_string()];

        let assignment = Assignment::from_card(&card).with_verification(vec![typed.clone()]);

        let specs = specs_for_validation(Some(&assignment), &card, &defaults());
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0], typed);
        assert_eq!(specs[1].command, "cargo test --lib");
        assert!(specs[1].cache.deterministic_inputs.is_empty());
        assert!(crate::chat::evidence_cache::spec_is_cacheable(&specs[0]).is_ok());
        assert_eq!(
            crate::chat::evidence_cache::spec_is_cacheable(&specs[1]),
            Err(crate::chat::evidence_cache::NonCacheable::NoDeclaredInputs)
        );
    }

    #[test]
    fn live_verify_lines_change_without_rewriting_the_dispatched_assignment() {
        let mut card = card("## Verification\n- Verify: `cargo test --lib`");
        card.target_files = vec!["src/lib.rs".to_string()];

        let assignment = Assignment::from_card(&card)
            .with_dependency_context("dependency report")
            .with_suggested_steps(40);
        let dispatched_prompt = assignment.render_prompt();

        card.instructions = "## Verification\n- Verify: `cargo test --lib -- rewritten`".into();
        card.title = "Rewritten title".into();
        card.target_files = vec!["src/other.rs".to_string()];
        card.scope_guard_mode = ScopeGuardMode::Strict;

        let validated = specs_for_validation(Some(&assignment), &card, &defaults());

        assert_eq!(assignment.render_prompt(), dispatched_prompt);
        assert_eq!(
            validated
                .iter()
                .map(|spec| spec.command.clone())
                .collect::<Vec<_>>(),
            vec!["cargo test --lib -- rewritten".to_string()]
        );
        assert!(validated[0].cache.deterministic_inputs.is_empty());
        assert_eq!(assignment.scope.enforcement(), ScopeEnforcement::Declared);
        assert_eq!(
            specs_from_card(&card, &defaults())
                .iter()
                .map(|spec| spec.command.clone())
                .collect::<Vec<_>>(),
            vec!["cargo test --lib -- rewritten".to_string()],
            "the card alone would have validated the rewritten contract"
        );
    }

    #[test]
    fn commands_the_agent_reported_are_not_verification_specs() {
        let mut card = card("## Verification\n- Verify: `cargo test --lib`");
        let assignment = Assignment::from_card(&card);
        card.final_report_structured = Some(FinalReport {
            verification: vec![VerificationResult {
                command: "npm test".to_string(),
                passed: true,
                ..Default::default()
            }],
            ..Default::default()
        });

        let specs = specs_for_validation(Some(&assignment), &card, &defaults());

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].proposed_by, SpecAuthority::CardInstructions);
    }

    #[test]
    fn a_card_without_an_assignment_validates_exactly_as_before() {
        let mut card = card("## Verification\n- Verify: `cargo test --lib`");
        card.target_files = vec!["src/lib.rs".to_string()];

        assert_eq!(
            specs_for_validation(None, &card, &defaults()),
            specs_from_card(&card, &defaults())
        );
        assert!(specs_for_validation(None, &card, &defaults())[0]
            .cache
            .deterministic_inputs
            .is_empty());
    }

    #[tokio::test]
    async fn a_stored_assignment_round_trips_and_is_absent_until_dispatched() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_workspace(temp.path()).await;
        let meta = storage::create_task(gcx.clone(), "Assignment task")
            .await
            .unwrap();
        let assignment = Assignment::from_card(&card("## Verification\n- Verify: `cargo test`"))
            .with_suggested_steps(40);

        assert!(load_assignment(gcx.clone(), &meta.id, "T-1")
            .await
            .unwrap()
            .is_none());

        save_assignment(gcx.clone(), &meta.id, &assignment)
            .await
            .unwrap();

        assert_eq!(
            load_assignment(gcx.clone(), &meta.id, "T-1").await.unwrap(),
            Some(assignment.clone())
        );
        let mut traversal = assignment.clone();
        traversal.card_id = "../escape".to_string();
        assert!(save_assignment(gcx.clone(), &meta.id, &traversal)
            .await
            .is_err());
        assert_eq!(
            load_assignment(gcx, &meta.id, "../escape").await.unwrap(),
            None,
            "an id that cannot address a record simply has none"
        );
    }
}
