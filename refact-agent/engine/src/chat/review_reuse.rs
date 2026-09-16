//! Reusing a review instead of paying for it again, without letting a reuse pass
//! for a fresh reading of the code.
//!
//! A review is not deterministic, so V-3's "declared deterministic inputs" rule
//! does not transfer. What does transfer is *reuse*: a [`ReviewReport`] still
//! describes the thing it reviewed, so it stands while that thing is unchanged —
//! the same diff, the same reviewer model, the same stage selection and the same
//! review policy. Move any one of those and the report describes something else,
//! so the reviewer runs again.
//!
//! Nothing here is a second cache. The bounded store is V-3's
//! [`ReuseStore`], the key is V-3's [`EvidenceCacheKey`], and the reuse marker is
//! V-3's `from_cache`/`replayed_at` on the evidence record, so a reused review
//! reaches the gates already admitting that it was not re-run.

use std::path::Path;
use std::sync::LazyLock;

use tokio::process::Command;
use tokio::sync::Mutex;

use crate::chat::evidence::EvidenceRecord;
use crate::chat::evidence_cache::{
    digest_text, input_digests, EvidenceCacheKey, EvidenceCacheLimits, Reusable, ReuseStore,
};
use crate::tools::review_agents::config::{ReviewConfig, ReviewSettings};
use crate::tools::review_types::ReviewReport;

const FIELD: char = '\u{1e}';

/// What a reviewer would read and with what. Every field is known *before* the
/// review runs: the reviewer model here is the configured one, never the model a
/// finished report happens to name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewIdentity {
    /// A digest of the diff under review, including uncommitted and untracked work.
    pub revision: String,
    pub reviewer_models: Vec<String>,
    pub stages: Vec<String>,
    pub policy: String,
}

/// The identity plus what the caller asked for. Both decide the review, so both
/// are keyed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewReuseInputs {
    pub identity: ReviewIdentity,
    pub what_to_check: Option<String>,
    pub plan: Option<String>,
    pub base: Option<String>,
    pub files: Vec<String>,
}

impl ReviewReuseInputs {
    pub fn key(&self) -> EvidenceCacheKey {
        let mut canonical = String::new();
        canonical.push_str("revision:");
        canonical.push_str(&self.identity.revision);
        canonical.push_str("\nmodels:");
        for model in &self.identity.reviewer_models {
            canonical.push_str(model);
            canonical.push(FIELD);
        }
        canonical.push_str("\nstages:");
        for stage in &self.identity.stages {
            canonical.push_str(stage);
            canonical.push(FIELD);
        }
        canonical.push_str("\npolicy:");
        canonical.push_str(&self.identity.policy);
        canonical.push_str("\nwhat_to_check:");
        canonical.push_str(self.what_to_check.as_deref().unwrap_or(""));
        canonical.push_str("\nplan:");
        canonical.push_str(self.plan.as_deref().unwrap_or(""));
        canonical.push_str("\nbase:");
        canonical.push_str(self.base.as_deref().unwrap_or(""));
        canonical.push_str("\nfiles:");
        for file in &self.files {
            canonical.push_str(file);
            canonical.push(FIELD);
        }
        EvidenceCacheKey::of(&canonical)
    }
}

/// A review kept for reuse: the report the goal path consumes and the evidence
/// record the gates consume, so a reuse hands back both or neither.
#[derive(Debug, Clone)]
pub struct ReusableReview {
    pub report: ReviewReport,
    pub record: EvidenceRecord,
}

impl Reusable for ReusableReview {
    fn is_replay(&self) -> bool {
        self.record.is_replay()
    }

    fn mark_replayed(&mut self, now_ms: i64) {
        self.record.mark_replayed(now_ms);
    }

    /// An `Unavailable` review — inconclusive, partial, or a reviewer that could
    /// not start — is not a result, so re-running is exactly what should happen.
    fn is_storable(&self) -> bool {
        self.record.is_storable()
    }

    fn size_bytes(&self) -> usize {
        self.record.size_bytes()
            + serde_json::to_string(&self.report)
                .map(|json| json.len())
                .unwrap_or_default()
    }
}

pub type ReviewReuseStore = ReuseStore<ReusableReview>;

pub fn review_reuse_limits() -> EvidenceCacheLimits {
    let settings = crate::runtime_settings::current();
    EvidenceCacheLimits {
        max_entries: settings.goal_review_reuse_max_entries,
        max_bytes: settings.goal_review_reuse_max_bytes,
        ttl_secs: settings.goal_review_reuse_ttl_secs,
    }
}

/// The one process-wide store of reusable reviews.
pub fn review_reuse_store() -> &'static Mutex<ReviewReuseStore> {
    static STORE: LazyLock<Mutex<ReviewReuseStore>> =
        LazyLock::new(|| Mutex::new(ReviewReuseStore::new(review_reuse_limits())));
    &STORE
}

/// Everything in the review policy that changes what a reviewer does. A change
/// here means the next review is a different review, so it is keyed.
pub fn review_policy_fingerprint(cfg: &ReviewConfig) -> String {
    let ReviewSettings {
        default_depth,
        parallel_depth,
        variants,
        idle_timeout_secs,
        max_steps,
        max_files,
        model_slot,
        variant_slots,
        n_ctx,
        max_new_tokens,
        stages,
    } = &cfg.settings;
    let mut overrides: Vec<String> = stages
        .iter()
        .map(|(id, over)| {
            format!(
                "{id}={:?}/{:?}/{:?}",
                over.enabled,
                over.model_slot.map(|slot| slot.label()),
                over.max_steps
            )
        })
        .collect();
    overrides.sort();
    let slots: Vec<&str> = variant_slots.iter().map(|slot| slot.label()).collect();
    digest_text(&format!(
        "depth={default_depth} parallel={parallel_depth} variants={variants} idle={idle_timeout_secs} max_steps={max_steps} max_files={max_files} slot={} variant_slots=[{}] n_ctx={n_ctx:?} max_new_tokens={max_new_tokens:?} overrides=[{}] guardrails={}",
        model_slot.label(),
        slots.join(","),
        overrides.join(","),
        digest_text(&cfg.guardrails_prompt)
    ))
}

async fn git_output(worktree: &Path, args: &[&str]) -> String {
    let Ok(output) = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .output()
        .await
    else {
        return String::new();
    };
    match output.status.success() {
        true => String::from_utf8_lossy(&output.stdout).to_string(),
        false => String::new(),
    }
}

/// A digest of exactly what a review looks at: the commit under review, the diff
/// against the base, the uncommitted working-tree diff, and the contents of every
/// untracked file. Untracked contents go through the same digest walker the
/// evidence cache keys on, so there is one digest scheme and not two.
pub async fn review_revision(worktree: &Path, base: Option<&str>) -> String {
    let mut canonical = String::new();
    canonical.push_str("head:");
    canonical.push_str(git_output(worktree, &["rev-parse", "HEAD"]).await.trim());
    if let Some(base) = base.map(str::trim).filter(|base| !base.is_empty()) {
        canonical.push_str("\nbase-diff:");
        canonical.push_str(&git_output(worktree, &["diff", &format!("{base}...HEAD")]).await);
    }
    canonical.push_str("\nworking-diff:");
    canonical.push_str(&git_output(worktree, &["diff", "HEAD"]).await);
    let untracked: Vec<String> =
        git_output(worktree, &["ls-files", "--others", "--exclude-standard"])
            .await
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
    canonical.push_str("\nuntracked:");
    for (path, digest) in input_digests(worktree, &untracked).await {
        canonical.push_str(&path);
        canonical.push('=');
        canonical.push_str(&digest);
        canonical.push(FIELD);
    }
    digest_text(&canonical)
}

#[cfg(test)]
pub(crate) fn test_store() -> Mutex<ReviewReuseStore> {
    Mutex::new(ReviewReuseStore::new(EvidenceCacheLimits {
        max_entries: 64,
        max_bytes: 1_048_576,
        ttl_secs: 3_600,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatModelType, SubchatParameters};
    use crate::llm::params::CacheControl;
    use crate::tools::review_agents::config::{ModelSlot, StageOverride};

    fn identity() -> ReviewIdentity {
        ReviewIdentity {
            revision: "rev-1".to_string(),
            reviewer_models: vec!["reviewer-a".to_string()],
            stages: vec!["diff".to_string(), "spec".to_string()],
            policy: "policy-1".to_string(),
        }
    }

    fn inputs() -> ReviewReuseInputs {
        ReviewReuseInputs {
            identity: identity(),
            what_to_check: Some("the acceptance criteria".to_string()),
            plan: None,
            base: Some("main".to_string()),
            files: vec!["src/lib.rs".to_string()],
        }
    }

    fn config() -> ReviewConfig {
        ReviewConfig {
            settings: ReviewSettings::default(),
            guardrails_prompt: "stay in scope".to_string(),
            base_params: SubchatParameters {
                subchat_model_type: ChatModelType::Default,
                subchat_model: "m".to_string(),
                subchat_n_ctx: 100_000,
                subchat_max_new_tokens: 8_000,
                subchat_temperature: None,
                subchat_tokens_for_rag: 0,
                subchat_reasoning_effort: None,
                subchat_cache_control: CacheControl::Off,
            },
        }
    }

    #[test]
    fn every_keyed_review_input_moves_the_key_when_it_changes() {
        let baseline = inputs().key();

        let mutations: Vec<(&str, Box<dyn Fn(&mut ReviewReuseInputs)>)> = vec![
            (
                "revision",
                Box::new(|i: &mut ReviewReuseInputs| i.identity.revision = "rev-2".to_string()),
            ),
            (
                "reviewer model",
                Box::new(|i: &mut ReviewReuseInputs| {
                    i.identity.reviewer_models = vec!["reviewer-b".to_string()]
                }),
            ),
            (
                "stage set",
                Box::new(|i: &mut ReviewReuseInputs| {
                    i.identity.stages.push("execution".to_string())
                }),
            ),
            (
                "policy",
                Box::new(|i: &mut ReviewReuseInputs| i.identity.policy = "policy-2".to_string()),
            ),
            (
                "what_to_check",
                Box::new(|i: &mut ReviewReuseInputs| i.what_to_check = None),
            ),
            (
                "plan",
                Box::new(|i: &mut ReviewReuseInputs| i.plan = Some("other".to_string())),
            ),
            (
                "base",
                Box::new(|i: &mut ReviewReuseInputs| i.base = Some("release".to_string())),
            ),
            (
                "files",
                Box::new(|i: &mut ReviewReuseInputs| i.files.push("src/main.rs".to_string())),
            ),
        ];

        for (label, mutate) in mutations {
            let mut mutated = inputs();
            mutate(&mut mutated);
            assert_ne!(baseline, mutated.key(), "{label} must be a keyed input");
        }
        assert_eq!(
            baseline,
            inputs().key(),
            "the key is stable for equal inputs"
        );
    }

    #[test]
    fn the_policy_fingerprint_moves_with_every_setting_that_changes_a_review() {
        let baseline = review_policy_fingerprint(&config());

        let mutations: Vec<(&str, Box<dyn Fn(&mut ReviewConfig)>)> = vec![
            (
                "depth",
                Box::new(|c: &mut ReviewConfig| c.settings.default_depth = "deep".to_string()),
            ),
            (
                "variants",
                Box::new(|c: &mut ReviewConfig| c.settings.variants = 3),
            ),
            (
                "model slot",
                Box::new(|c: &mut ReviewConfig| c.settings.model_slot = ModelSlot::Thinking),
            ),
            (
                "max steps",
                Box::new(|c: &mut ReviewConfig| c.settings.max_steps = 4),
            ),
            (
                "stage override",
                Box::new(|c: &mut ReviewConfig| {
                    c.settings.stages.insert(
                        "execution".to_string(),
                        StageOverride {
                            enabled: Some(false),
                            ..StageOverride::default()
                        },
                    );
                }),
            ),
            (
                "guardrails prompt",
                Box::new(|c: &mut ReviewConfig| {
                    c.guardrails_prompt = "stay in scope, always".to_string()
                }),
            ),
        ];

        for (label, mutate) in mutations {
            let mut mutated = config();
            mutate(&mut mutated);
            assert_ne!(
                baseline,
                review_policy_fingerprint(&mutated),
                "{label} must move the policy fingerprint"
            );
        }
        assert_eq!(baseline, review_policy_fingerprint(&config()));
    }

    #[tokio::test]
    async fn the_revision_follows_the_working_tree_and_not_the_clock() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path();
        for args in [
            vec!["init"],
            vec!["checkout", "-b", "main"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test User"],
        ] {
            git_output(repo, &args).await;
        }
        tokio::fs::write(repo.join("lib.rs"), "fn main() {}")
            .await
            .unwrap();
        git_output(repo, &["add", "lib.rs"]).await;
        git_output(repo, &["commit", "-m", "initial"]).await;

        let baseline = review_revision(repo, None).await;
        assert_eq!(
            baseline,
            review_revision(repo, None).await,
            "an unchanged tree keeps its revision"
        );

        tokio::fs::write(repo.join("lib.rs"), "fn main() { todo!() }")
            .await
            .unwrap();
        let edited = review_revision(repo, None).await;
        assert_ne!(baseline, edited, "an uncommitted edit moves the revision");

        tokio::fs::write(repo.join("lib.rs"), "fn main() {}")
            .await
            .unwrap();
        assert_eq!(
            baseline,
            review_revision(repo, None).await,
            "restoring the content restores the revision"
        );

        tokio::fs::write(repo.join("scratch.rs"), "// untracked")
            .await
            .unwrap();
        let untracked = review_revision(repo, None).await;
        assert_ne!(baseline, untracked, "an untracked file moves the revision");
        tokio::fs::write(repo.join("scratch.rs"), "// untracked, edited")
            .await
            .unwrap();
        assert_ne!(
            untracked,
            review_revision(repo, None).await,
            "editing an untracked file moves the revision"
        );
    }
}
