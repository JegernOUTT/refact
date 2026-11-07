//! The content revision an evidence record was produced against.
//!
//! # A fourth axis, not a unification
//!
//! The engine guards staleness on four independent axes today, and each one
//! watches a *different* write surface:
//!
//! | Axis | Surface it guards |
//! |---|---|
//! | goal ledger seq (`session.goal_ledger_last_seq`) | goal status transitions |
//! | `ChatSession.trajectory_version` | queue commands, edits, compaction rewrites |
//! | `ExpectedCardState { board_rev, card_fingerprint }` | board writes to one card |
//! | `ResolvedProfile.revision` | agent profile edits |
//!
//! **Decision: add a fourth axis for evidence, do not unify the existing four.**
//! Unifying them means one number that moves whenever *any* surface moves, so
//! every guard inherits every other guard's false positives. That failure is
//! not hypothetical: V-1 shipped `content_revision = card_id@board_rev`, and
//! `board_rev` bumps on every board write, so renaming an unrelated card
//! invalidated evidence that was still perfectly valid. A global counter cannot
//! answer "did *this* evidence's inputs change".
//!
//! So this axis is content-derived and spec-scoped. A [`ContentRevision`]
//! carries only what the evidence itself depended on — the spec's identity and
//! revision, the worktree and its commit, a digest per declared input file and
//! a digest per expected artifact — and therefore moves only when one of those
//! moves. It composes with the other four rather than replacing them: each
//! still guards its own surface, and none of them is weakened.
//!
//! # Digests, never timestamps
//!
//! Every comparison here is over content digests. A file rewritten with
//! identical bytes has a new mtime and the same revision, and evidence about it
//! stands. Input digests come from `evidence_cache::input_digests`, the same
//! walker the evidence cache keys on, so there is one digest scheme in
//! the evidence subsystem and not two. When a drift names a path, the durable
//! file provenance from M3 supplies the writer, so the reason says who moved it
//! instead of only that it moved.
//!
//! # A moved revision is `superseded`
//!
//! [`supersede_stale`] rewrites a record whose revision has moved into
//! [`EvidenceVerdict::Superseded`]. That verdict is not a pass, so
//! `evaluate_gates` blocks completion on it, and it is not a result, so the
//! evidence cache refuses to store it. The only way past it is to verify again
//! against the tree that exists now.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::chat::evidence::{resolve_artifact_path, EvidenceRecord, EvidenceVerdict, VerificationSpec};
use crate::chat::evidence_cache::input_digests;
use crate::coordination::file_provenance::{last_writer, render, WriterAttribution};
use crate::file_activity::{digest_bytes, resolve_key};
use crate::tasks::types::VerificationOutcome;

const ABSENT: &str = "<absent>";
const REFUSED: &str = "<refused>";
const FIELD: char = '\u{1e}';

/// What the evidence was produced against. Every field is content the evidence
/// depended on; nothing here is a global counter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContentRevision {
    pub spec_id: String,
    pub spec_revision: u64,
    pub worktree: String,
    pub commit: Option<String>,
    pub inputs: BTreeMap<String, String>,
    pub artifacts: BTreeMap<String, String>,
}

impl ContentRevision {
    /// Identity alone, for a spec that was never executed against a tree.
    pub fn declared(spec: &VerificationSpec) -> Self {
        Self {
            spec_id: spec.spec_id.clone(),
            spec_revision: spec.revision,
            ..Self::default()
        }
    }

    /// Read the tree and record what this spec currently depends on.
    pub async fn observe(worktree: &Path, commit: Option<&str>, spec: &VerificationSpec) -> Self {
        Self {
            spec_id: spec.spec_id.clone(),
            spec_revision: spec.revision,
            worktree: worktree.to_string_lossy().to_string(),
            commit: commit.map(str::to_string),
            inputs: input_digests(worktree, &spec.cache.deterministic_inputs).await,
            artifacts: artifact_digests(worktree, spec).await,
        }
    }

    pub fn digest(&self) -> String {
        let mut canonical = String::new();
        canonical.push_str("spec:");
        canonical.push_str(&self.spec_id);
        canonical.push_str("\nrevision:");
        canonical.push_str(&self.spec_revision.to_string());
        canonical.push_str("\nworktree:");
        canonical.push_str(&self.worktree);
        canonical.push_str("\ncommit:");
        canonical.push_str(self.commit.as_deref().unwrap_or(""));
        canonical.push_str("\ninputs:");
        push_pairs(&mut canonical, &self.inputs);
        canonical.push_str("\nartifacts:");
        push_pairs(&mut canonical, &self.artifacts);
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// The first way `current` differs from the revision this evidence was
    /// produced against, or `None` when the evidence still describes the tree.
    pub fn drift(&self, current: &ContentRevision) -> Option<RevisionDrift> {
        if self.worktree != current.worktree {
            return Some(RevisionDrift::Worktree {
                from: self.worktree.clone(),
                to: current.worktree.clone(),
            });
        }
        if self.spec_revision != current.spec_revision {
            return Some(RevisionDrift::SpecRevision {
                from: self.spec_revision,
                to: current.spec_revision,
            });
        }
        if self.commit != current.commit {
            return Some(RevisionDrift::Commit {
                from: self.commit.clone(),
                to: current.commit.clone(),
            });
        }
        if let Some(drift) = pair_drift(&self.inputs, &current.inputs, true) {
            return Some(drift);
        }
        pair_drift(&self.artifacts, &current.artifacts, false)
    }
}

fn push_pairs(canonical: &mut String, pairs: &BTreeMap<String, String>) {
    for (path, digest) in pairs {
        canonical.push_str(path);
        canonical.push('=');
        canonical.push_str(digest);
        canonical.push(FIELD);
    }
}

fn pair_drift(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    input: bool,
) -> Option<RevisionDrift> {
    let paths: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for path in paths {
        let from = before.get(path).map(String::as_str).unwrap_or(ABSENT);
        let to = after.get(path).map(String::as_str).unwrap_or(ABSENT);
        if from == to {
            continue;
        }
        let (path, from, to) = (path.clone(), from.to_string(), to.to_string());
        return Some(match input {
            true => RevisionDrift::Input {
                path,
                from,
                to,
            },
            false => RevisionDrift::Artifact {
                path,
                from,
                to,
            },
        });
    }
    None
}

async fn artifact_digests(worktree: &Path, spec: &VerificationSpec) -> BTreeMap<String, String> {
    let mut digests = BTreeMap::new();
    for artifact in &spec.expected_artifacts {
        let digest = match resolve_artifact_path(worktree, spec, &artifact.path) {
            Err(_) => REFUSED.to_string(),
            Ok(path) => match tokio::fs::read(&path).await {
                Ok(bytes) => digest_bytes(&bytes),
                Err(_) => ABSENT.to_string(),
            },
        };
        digests.insert(artifact.path.clone(), digest);
    }
    digests
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionDrift {
    Worktree {
        from: String,
        to: String,
    },
    SpecRevision {
        from: u64,
        to: u64,
    },
    Commit {
        from: Option<String>,
        to: Option<String>,
    },
    Input {
        path: String,
        from: String,
        to: String,
    },
    Artifact {
        path: String,
        from: String,
        to: String,
    },
}

impl RevisionDrift {
    /// The worktree-relative path that moved, when the drift is about a file.
    pub fn path(&self) -> Option<&str> {
        match self {
            RevisionDrift::Input {
                path, ..
            }
            | RevisionDrift::Artifact {
                path, ..
            } => Some(path),
            _ => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            RevisionDrift::Worktree {
                from,
                to,
            } => format!(
                "evidence was produced in worktree '{}', the tree under test is '{}'",
                from, to
            ),
            RevisionDrift::SpecRevision {
                from,
                to,
            } => format!(
                "the verification spec moved from revision {} to {}",
                from, to
            ),
            RevisionDrift::Commit {
                from,
                to,
            } => format!(
                "the worktree moved from commit {} to {}",
                from.as_deref().unwrap_or(ABSENT),
                to.as_deref().unwrap_or(ABSENT)
            ),
            RevisionDrift::Input {
                path,
                from,
                to,
            } => {
                format!("declared input '{}' changed ({} -> {})", path, from, to)
            }
            RevisionDrift::Artifact {
                path,
                from,
                to,
            } => {
                format!("expected artifact '{}' changed ({} -> {})", path, from, to)
            }
        }
    }
}

/// Re-read the tree and supersede every executed result whose own inputs have
/// moved since it was produced. A record that never obtained a result keeps its
/// verdict: `blocked` and `unavailable` mean the evidence was never gathered,
/// and calling that `superseded` would misreport why completion is refused.
pub async fn supersede_stale(
    worktree: &Path,
    commit: Option<&str>,
    specs: &[VerificationSpec],
    records: Vec<EvidenceRecord>,
) -> Vec<EvidenceRecord> {
    let mut checked = Vec::with_capacity(records.len());
    for mut record in records {
        let is_result = matches!(
            record.verdict,
            EvidenceVerdict::Passed | EvidenceVerdict::Failed
        );
        let spec = specs.iter().find(|spec| spec.spec_id == record.spec_id);
        let (Some(spec), true) = (spec, is_result) else {
            checked.push(record);
            continue;
        };
        let current = ContentRevision::observe(worktree, commit, spec).await;
        if let Some(drift) = record.content_revision.drift(&current) {
            record.verdict = EvidenceVerdict::Superseded;
            record.outcome = VerificationOutcome::Unknown;
            record.reason = Some(supersede_reason(worktree, &drift).await);
        }
        checked.push(record);
    }
    checked
}

async fn supersede_reason(worktree: &Path, drift: &RevisionDrift) -> String {
    let attribution = match drift.path() {
        Some(path) => {
            let (key, _) = resolve_key(&worktree.join(path));
            last_writer(&key, None, 0).await
        }
        None => WriterAttribution::ExternalOrUnknown,
    };
    let writer = match attribution {
        WriterAttribution::ExternalOrUnknown => String::new(),
        named => render(&named),
    };
    format!(
        "evidence describes an earlier revision: {}.{} Re-verification is required",
        drift.describe(),
        writer
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::evidence::{
        evaluate_gates, ArtifactAssertion, EvidenceContext, EvidenceDefaults, EvidenceRunner,
        EvidenceService, ExpectedArtifact, SpecAuthority,
    };
    use crate::chat::verify_cmd::VerifyCommandPolicy;
    use crate::tasks::types::{BoardCard, ScopeGuardMode, TaskBoard, VerificationResult};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::path::PathBuf;

    #[derive(Default)]
    struct CountingRunner {
        runs: usize,
    }

    #[async_trait]
    impl EvidenceRunner for CountingRunner {
        async fn run(&mut self, _worktree: &Path, spec: &VerificationSpec) -> VerificationResult {
            self.runs += 1;
            VerificationResult {
                command: spec.command.clone(),
                exit_code: Some(0),
                passed: true,
                output_tail: "ok".to_string(),
                outcome: VerificationOutcome::Passed,
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

    fn declared_specs_from_card(
        card: &BoardCard,
        defaults: &EvidenceDefaults,
    ) -> Vec<VerificationSpec> {
        let mut specs = crate::chat::evidence::specs_from_card(card, defaults);
        for spec in &mut specs {
            spec.cache.deterministic_inputs = vec!["src".to_string()];
        }
        specs
    }

    fn service(worktree: &Path) -> EvidenceService {
        EvidenceService::new(
            VerifyCommandPolicy::permissive(),
            EvidenceContext {
                worktree: worktree.to_path_buf(),
                commit: None,
                scope: "T-1".to_string(),
            },
        )
    }

    fn card(id: &str, title: &str) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: title.to_string(),
            column: "done".to_string(),
            priority: "P1".to_string(),
            depends_on: Vec::new(),
            instructions: "## Acceptance Criteria\n- Verify: `cargo test --lib`".to_string(),
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
            paused_at: None,
            paused_by: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: Vec::new(),
            target_files: vec!["src".to_string()],
            scope_guard_mode: ScopeGuardMode::Off,
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }

    fn board() -> TaskBoard {
        TaskBoard {
            rev: 7,
            cards: vec![
                card("T-1", "the card under verification"),
                card("T-2", "an unrelated card"),
            ],
            ..Default::default()
        }
    }

    async fn worktree_with_source() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(temp.path().join("src"))
            .await
            .unwrap();
        tokio::fs::write(temp.path().join("src").join("lib.rs"), "fn main() {}")
            .await
            .unwrap();
        temp
    }

    async fn verify(
        worktree: &Path,
        specs: &[VerificationSpec],
        runner: &mut CountingRunner,
    ) -> Vec<EvidenceRecord> {
        service(worktree)
            .accept_and_execute_all(specs.to_vec(), runner)
            .await
    }

    #[tokio::test]
    async fn an_unrelated_board_write_does_not_supersede_valid_evidence() {
        let temp = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        let mut board = board();
        let specs = declared_specs_from_card(board.get_card("T-1").unwrap(), &defaults());
        let records = verify(temp.path(), &specs, &mut runner).await;
        assert_eq!(records[0].verdict, EvidenceVerdict::Passed);
        let produced_against = records[0].fingerprint.content_revision.clone();

        board.rev = 41;
        let unrelated = board.get_card_mut("T-2").unwrap();
        unrelated.title = "renamed while the verdict was in flight".to_string();
        unrelated.column = "doing".to_string();
        unrelated.instructions = "## Acceptance Criteria\n- Verify: `npm test`".to_string();
        let specs = declared_specs_from_card(board.get_card("T-1").unwrap(), &defaults());
        let records = supersede_stale(temp.path(), None, &specs, records).await;
        let gates = evaluate_gates(&specs, &records);

        assert_eq!(
            records[0].verdict,
            EvidenceVerdict::Passed,
            "a write to another card must not invalidate this card's evidence: {:?}",
            records[0].reason
        );
        assert_eq!(
            records[0].fingerprint.content_revision, produced_against,
            "the content revision must not follow a global board counter"
        );
        assert!(gates.completion_allowed());
        assert_eq!(runner.runs, 1);
    }

    #[tokio::test]
    async fn editing_a_declared_file_after_a_pass_supersedes_the_verdict() {
        let temp = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        let specs = declared_specs_from_card(board().get_card("T-1").unwrap(), &defaults());
        let records = verify(temp.path(), &specs, &mut runner).await;
        assert_eq!(records[0].verdict, EvidenceVerdict::Passed);

        tokio::fs::write(
            temp.path().join("src").join("lib.rs"),
            "fn main() { todo!() }",
        )
        .await
        .unwrap();
        let records = supersede_stale(temp.path(), None, &specs, records).await;
        let gates = evaluate_gates(&specs, &records);

        assert_eq!(records[0].verdict, EvidenceVerdict::Superseded);
        assert!(
            records[0].reason.as_deref().unwrap().contains("src/lib.rs"),
            "{:?}",
            records[0].reason
        );
        assert!(!gates.completion_allowed());
        assert_eq!(gates.blocks[0].verdict, EvidenceVerdict::Superseded);
    }

    #[tokio::test]
    async fn a_file_rewritten_with_identical_content_is_not_superseded() {
        let temp = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        let specs = declared_specs_from_card(board().get_card("T-1").unwrap(), &defaults());
        let records = verify(temp.path(), &specs, &mut runner).await;

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        tokio::fs::write(temp.path().join("src").join("lib.rs"), "fn main() {}")
            .await
            .unwrap();
        let records = supersede_stale(temp.path(), None, &specs, records).await;

        assert_eq!(
            records[0].verdict,
            EvidenceVerdict::Passed,
            "a new mtime over identical bytes is not a new revision"
        );
    }

    #[tokio::test]
    async fn changing_the_spec_revision_supersedes_prior_evidence() {
        let temp = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        let specs = declared_specs_from_card(board().get_card("T-1").unwrap(), &defaults());
        let records = verify(temp.path(), &specs, &mut runner).await;

        let revised: Vec<VerificationSpec> = specs
            .iter()
            .cloned()
            .map(|mut spec| {
                spec.revision = 2;
                spec
            })
            .collect();
        let records = supersede_stale(temp.path(), None, &revised, records).await;
        let gates = evaluate_gates(&revised, &records);

        assert_eq!(records[0].verdict, EvidenceVerdict::Superseded);
        assert!(records[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("revision 1 to 2"));
        assert!(!gates.completion_allowed());
    }

    #[tokio::test]
    async fn replacing_an_expected_artifact_supersedes_the_verdict() {
        let temp = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        tokio::fs::write(temp.path().join("report.json"), "{\"passed\": 12}")
            .await
            .unwrap();
        let mut specs = declared_specs_from_card(board().get_card("T-1").unwrap(), &defaults());
        specs[0].expected_artifacts = vec![ExpectedArtifact {
            path: "report.json".to_string(),
            assertion: ArtifactAssertion::Contains("passed".to_string()),
        }];
        let records = verify(temp.path(), &specs, &mut runner).await;
        assert_eq!(records[0].verdict, EvidenceVerdict::Passed);

        tokio::fs::write(temp.path().join("report.json"), "{\"passed\": 3}")
            .await
            .unwrap();
        let records = supersede_stale(temp.path(), None, &specs, records).await;

        assert_eq!(records[0].verdict, EvidenceVerdict::Superseded);
        assert!(records[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("report.json"));
    }

    #[tokio::test]
    async fn a_moved_commit_supersedes_evidence_about_the_earlier_tree() {
        let temp = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        let specs = declared_specs_from_card(board().get_card("T-1").unwrap(), &defaults());
        let service = EvidenceService::new(
            VerifyCommandPolicy::permissive(),
            EvidenceContext {
                worktree: temp.path().to_path_buf(),
                commit: Some("aaaa111".to_string()),
                scope: "T-1".to_string(),
            },
        );
        let records = service
            .accept_and_execute_all(specs.clone(), &mut runner)
            .await;

        let unchanged =
            supersede_stale(temp.path(), Some("aaaa111"), &specs, records.clone()).await;
        let moved = supersede_stale(temp.path(), Some("bbbb222"), &specs, records).await;

        assert_eq!(unchanged[0].verdict, EvidenceVerdict::Passed);
        assert_eq!(moved[0].verdict, EvidenceVerdict::Superseded);
        assert!(moved[0].reason.as_deref().unwrap().contains("bbbb222"));
    }

    #[tokio::test]
    async fn superseded_blocks_completion_until_the_evidence_is_produced_again() {
        let temp = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        let specs = declared_specs_from_card(board().get_card("T-1").unwrap(), &defaults());
        let records = verify(temp.path(), &specs, &mut runner).await;
        tokio::fs::write(temp.path().join("src").join("lib.rs"), "fn main() { 1; }")
            .await
            .unwrap();

        let stale = supersede_stale(temp.path(), None, &specs, records).await;
        let blocked = evaluate_gates(&specs, &stale);
        assert!(!blocked.completion_allowed());
        assert!(!blocked.has_contract_violation());
        assert!(
            !crate::chat::evidence::completion_decision(&blocked, true).allowed,
            "a report claiming success must not defeat a superseded verdict"
        );

        let fresh = verify(temp.path(), &specs, &mut runner).await;
        let fresh = supersede_stale(temp.path(), None, &specs, fresh).await;
        let allowed = evaluate_gates(&specs, &fresh);

        assert_eq!(fresh[0].verdict, EvidenceVerdict::Passed);
        assert!(
            allowed.completion_allowed(),
            "re-verification against the current tree clears the block"
        );
        assert_eq!(runner.runs, 2);
    }

    #[tokio::test]
    async fn evidence_from_another_worktree_never_answers_for_this_one() {
        let temp = worktree_with_source().await;
        let other = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        let specs = declared_specs_from_card(board().get_card("T-1").unwrap(), &defaults());
        let records = verify(temp.path(), &specs, &mut runner).await;

        let records = supersede_stale(other.path(), None, &specs, records).await;

        assert_eq!(records[0].verdict, EvidenceVerdict::Superseded);
        assert!(records[0].reason.as_deref().unwrap().contains("worktree"));
    }

    #[tokio::test]
    async fn a_verdict_that_obtained_no_evidence_keeps_its_own_reason() {
        let temp = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        let mut specs = declared_specs_from_card(board().get_card("T-1").unwrap(), &defaults());
        specs[0].command = "cargo test | tee log".to_string();
        let records = verify(temp.path(), &specs, &mut runner).await;
        assert_eq!(records[0].verdict, EvidenceVerdict::Blocked);

        tokio::fs::write(temp.path().join("src").join("lib.rs"), "fn main() { 2; }")
            .await
            .unwrap();
        let records = supersede_stale(temp.path(), None, &specs, records).await;

        assert_eq!(
            records[0].verdict,
            EvidenceVerdict::Blocked,
            "evidence that was never obtained is not evidence about an older revision"
        );
        assert_eq!(runner.runs, 0);
    }

    #[tokio::test]
    async fn an_executed_record_carries_the_revision_it_was_produced_against() {
        let temp = worktree_with_source().await;
        let mut runner = CountingRunner::default();
        let specs = declared_specs_from_card(board().get_card("T-1").unwrap(), &defaults());

        let records = verify(temp.path(), &specs, &mut runner).await;

        let revision = &records[0].content_revision;
        assert_eq!(revision.spec_id, specs[0].spec_id);
        assert_eq!(revision.spec_revision, 1);
        assert_eq!(revision.worktree, temp.path().to_string_lossy());
        assert!(revision.inputs.contains_key("src/lib.rs"));
        assert_eq!(
            records[0].fingerprint.content_revision,
            revision.digest(),
            "the fingerprint carries the revision digest, not a board counter"
        );
        assert_eq!(
            ContentRevision::observe(temp.path(), None, &specs[0])
                .await
                .digest(),
            revision.digest(),
            "an unchanged tree recomputes to the same revision"
        );
    }

    #[tokio::test]
    async fn an_artifact_outside_the_worktree_is_refused_by_the_revision_too() {
        let temp = worktree_with_source().await;
        let mut spec = VerificationSpec::propose(
            "S-1",
            "cargo test --lib",
            SpecAuthority::CardInstructions,
            &defaults(),
        );
        spec.cwd = Some(PathBuf::from("src"));
        spec.expected_artifacts = vec![ExpectedArtifact {
            path: "../../escape.txt".to_string(),
            assertion: ArtifactAssertion::Exists,
        }];

        let revision = ContentRevision::observe(temp.path(), None, &spec).await;

        assert_eq!(
            revision
                .artifacts
                .get("../../escape.txt")
                .map(String::as_str),
            Some(REFUSED)
        );
    }
}
