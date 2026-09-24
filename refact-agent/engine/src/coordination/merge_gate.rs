//! Serialized worktree merges with dirty-baseline refusal and post-merge
//! reverification (M3).
//!
//! Three rules, and each of them exists because the opposite has already gone
//! wrong here:
//!
//! * **One merge at a time per target checkout.** Two merges rewriting the same
//!   working tree interleave into a state neither of them verified. The lock is
//!   an M3 lease (`leases.rs`) keyed on the target [`CheckoutKey`], so merges
//!   into *different* checkouts never wait on each other, and a merge that
//!   cannot take the lease fails as retryable instead of blocking forever.
//! * **Uncommitted work in the target is not ours to move.** It is reported and
//!   the merge is refused; the acknowledged path stashes and restores it. It is
//!   never discarded either way.
//! * **A branch verified in isolation says nothing about the combined tree.**
//!   The pre-merge result is carried through untouched and the merged checkout
//!   is verified again, separately. [`GuardedMerge::merged_tree_verified`]
//!   counts only the post-merge run, and the source worktree is not deleted
//!   until that run passes.
//!
//! **The T-12 wait-for graph cannot see this edge.** Its nodes are runs
//! ([`refact_work_core::RunKey`]) and its targets are runs or the user, while a
//! merge lease is held by an *engine instance* — there is no run to point the
//! edge at. Making it visible would mean recording the holding run on the lease
//! row, which M3 leases do not carry. The wait is bounded by
//! `merge_lease_wait_ms` and the lease itself expires, so a blocked merge ends
//! in a structured error rather than an invisible deadlock; that bound is the
//! guarantee here, not graph visibility.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use refact_work_core::settings::CoordinationSettings;
use refact_work_core::store::records::now_ms;
use refact_work_core::CheckoutKey;
use serde::Serialize;

use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{
    DeleteWorktreeResponse, DirtyBaseline, MergeWorktreeRequest, MergeWorktreeResponse,
    WorktreeConflictState,
};
use crate::tasks::types::VerificationOutcome;

use super::leases::{LeaseError, LeaseHolder, LeaseManager, LeaseRenewal};

/// Identity of a checkout a merge writes into.
pub fn checkout_key(root: &Path) -> CheckoutKey {
    CheckoutKey::new(
        dunce::canonicalize(root)
            .unwrap_or_else(|_| root.to_path_buf())
            .to_string_lossy()
            .to_string(),
    )
}

/// The lease that serializes merges into one checkout.
pub fn merge_lease_key(checkout: &CheckoutKey) -> String {
    format!("merge:{}", checkout.as_str())
}

/// Someone else is merging into this checkout right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergeBusy {
    pub checkout: String,
    pub lease_key: String,
    pub holder_instance: String,
    pub held_for_ms: i64,
    pub waited_ms: i64,
    /// Always true: a busy checkout is a transient state, not a rejection.
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeGateError {
    Busy(Box<MergeBusy>),
    Lease(LeaseError),
    Merge(String),
}

impl std::fmt::Display for MergeGateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeGateError::Busy(busy) => write!(
                f,
                "checkout `{}` is being merged into by engine `{}` (held for {}ms); \
                 waited {}ms for the merge lease, retry later",
                busy.checkout, busy.holder_instance, busy.held_for_ms, busy.waited_ms
            ),
            MergeGateError::Lease(error) => write!(f, "{error}"),
            MergeGateError::Merge(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for MergeGateError {}

impl MergeGateError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, MergeGateError::Busy(_))
    }
}

/// The right to merge into one checkout, for as long as the lease is held.
pub struct MergeSlot {
    leases: Arc<LeaseManager>,
    holder: LeaseHolder,
    checkout: CheckoutKey,
    waited_ms: i64,
}

impl std::fmt::Debug for MergeSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergeSlot")
            .field("checkout", &self.checkout)
            .field("lease_key", &self.holder.lease_key)
            .field("fencing_generation", &self.holder.fencing_generation)
            .field("waited_ms", &self.waited_ms)
            .finish()
    }
}

impl MergeSlot {
    pub fn checkout(&self) -> &CheckoutKey {
        &self.checkout
    }

    pub fn holder(&self) -> &LeaseHolder {
        &self.holder
    }

    /// How long this merge queued behind another one.
    pub fn waited_ms(&self) -> i64 {
        self.waited_ms
    }

    /// Give the checkout back. A slot dropped without this leaves the lease to
    /// expire on its TTL rather than wedging the checkout forever.
    pub async fn release(self) -> Result<bool, LeaseError> {
        self.leases.release(&self.holder).await
    }
}

/// Hands out one merge slot per checkout.
pub struct MergeGate {
    leases: Arc<LeaseManager>,
    wait: Duration,
    poll: Duration,
}

impl MergeGate {
    pub fn new(leases: Arc<LeaseManager>, settings: &CoordinationSettings) -> MergeGate {
        MergeGate {
            leases,
            wait: Duration::from_millis(settings.merge_lease_wait_ms),
            poll: Duration::from_millis(settings.merge_lease_poll_interval_ms),
        }
    }

    /// Wait for the checkout, bounded. Call this **outside** any held admission
    /// permit: a merge holding a scheduler permit while queueing for a lease
    /// another permit-holder owns is the one cycle this cannot see.
    pub async fn acquire(&self, checkout: &CheckoutKey) -> Result<MergeSlot, MergeGateError> {
        let lease_key = merge_lease_key(checkout);
        let started_at_ms = now_ms();
        let deadline_ms = self.wait.as_millis() as i64;
        loop {
            match self.leases.acquire(&lease_key, now_ms()).await {
                Ok(holder) => {
                    let waited_ms = now_ms().saturating_sub(started_at_ms).max(0);
                    super::metrics::record_queue_latency("merge_lease_granted", waited_ms);
                    return Ok(MergeSlot {
                        leases: self.leases.clone(),
                        holder,
                        checkout: checkout.clone(),
                        waited_ms,
                    });
                }
                Err(LeaseError::Held(stale)) => {
                    let waited_ms = now_ms().saturating_sub(started_at_ms).max(0);
                    if waited_ms >= deadline_ms {
                        super::metrics::record_queue_latency("merge_lease_refused", waited_ms);
                        super::metrics::explain(super::metrics::Explanation::new(
                            super::metrics::Decision::MergeRefusal,
                            "refused",
                            "merge_gate.lease_held_past_wait_deadline",
                            vec![
                                (
                                    "reason".to_string(),
                                    format!(
                                        "merge lease held for {}ms, waited {}ms of {}ms",
                                        stale.held_for_ms, waited_ms, deadline_ms
                                    ),
                                ),
                                ("source".to_string(), stale.holder_instance.clone()),
                                ("result".to_string(), "retryable".to_string()),
                            ],
                        ));
                        return Err(MergeGateError::Busy(Box::new(MergeBusy {
                            checkout: checkout.as_str().to_string(),
                            lease_key,
                            holder_instance: stale.holder_instance,
                            held_for_ms: stale.held_for_ms,
                            waited_ms,
                            retryable: true,
                        })));
                    }
                    tokio::time::sleep(self.poll).await;
                }
                Err(error) => return Err(MergeGateError::Lease(error)),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStage {
    /// Ran in the source worktree, before the merge existed.
    PreMerge,
    /// Ran in the target checkout, against the merged tree.
    PostMerge,
}

impl Default for VerificationStage {
    fn default() -> Self {
        Self::PostMerge
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct MergeVerification {
    pub stage: VerificationStage,
    pub command: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub output_tail: String,
    #[serde(default)]
    pub auto_reverted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_skipped_reason: Option<String>,
    #[serde(default)]
    pub outcome: VerificationOutcome,
}

impl MergeVerification {}

/// One merge, with the two verification results kept apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardedMerge {
    pub checkout: String,
    pub waited_for_lease_ms: i64,
    pub response: MergeWorktreeResponse,
    /// What the branch proved about itself in isolation. Never evidence about
    /// the merged tree.
    pub pre_merge: Option<MergeVerification>,
    /// What the merged checkout proved. `None` means nobody checked.
    pub post_merge: Option<MergeVerification>,
    pub cleanup: Option<DeleteWorktreeResponse>,
}

impl GuardedMerge {
    /// Only a post-merge pass counts. A green branch merged into a tree nobody
    /// re-ran is unverified, whatever the pre-merge result said.
    pub fn merged_tree_verified(&self) -> bool {
        self.post_merge
            .as_ref()
            .is_some_and(|verification| verification.passed)
    }

    pub fn refused_dirty_baseline(&self) -> Option<&DirtyBaseline> {
        self.response.dirty_baseline.as_ref()
    }

    pub fn conflict(&self) -> Option<&WorktreeConflictState> {
        self.response.conflict.as_ref()
    }

    /// True while the source worktree is still on disk to be inspected.
    pub fn source_preserved(&self) -> bool {
        !self.cleanup.as_ref().is_some_and(|cleanup| cleanup.deleted)
    }
}

/// Merge `worktree_id` into `service`'s checkout while holding that checkout's
/// merge lease, then verify the merged tree.
///
/// `verify_merged_tree` is handed the target checkout after the merge landed;
/// returning `None` means no verification was configured, which is reported as
/// unverified rather than as a pass. The source worktree is deleted only when
/// the merge landed, deletion was asked for, and the post-merge verification
/// did not fail — a failed merge or a failed reverification always leaves it
/// intact and inspectable.
pub async fn merge_under_lease<F, Fut>(
    gate: &MergeGate,
    service: &WorktreeService,
    worktree_id: &str,
    request: MergeWorktreeRequest,
    pre_merge: Option<MergeVerification>,
    verify_merged_tree: F,
) -> Result<GuardedMerge, MergeGateError>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = Option<MergeVerification>>,
{
    let checkout = checkout_key(service.source_workspace_root());
    let slot = gate.acquire(&checkout).await?;
    let renewal = LeaseRenewal::spawn(slot.leases.clone(), slot.holder.clone());
    let waited_for_lease_ms = slot.waited_ms();
    let delete_requested = request.delete_after_merge;
    let response = service
        .merge_worktree(
            worktree_id,
            MergeWorktreeRequest {
                delete_after_merge: false,
                ..request
            },
        )
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            shutdown_renewal(renewal).await;
            return Err(MergeGateError::Merge(error));
        }
    };
    let post_merge = match response.merged {
        true => verify_merged_tree(service.source_workspace_root().to_path_buf()).await,
        false => None,
    };
    let verification_failed = post_merge
        .as_ref()
        .is_some_and(|verification| !verification.passed);
    let cleanup = match delete_requested
        && !verification_failed
        && (response.merged || response.status == "nothing_to_merge")
    {
        true => match service.delete_worktree(worktree_id, true, true).await {
            Ok(cleanup) => Some(cleanup),
            Err(error) => {
                tracing::warn!(
                    worktree_id,
                    "merged worktree could not be cleaned up, it stays on disk: {error}"
                );
                None
            }
        },
        false => None,
    };
    shutdown_renewal(renewal).await;
    Ok(GuardedMerge {
        checkout: checkout.as_str().to_string(),
        waited_for_lease_ms,
        response,
        pre_merge,
        post_merge,
        cleanup,
    })
}

async fn shutdown_renewal(mut renewal: LeaseRenewal) {
    if let Err(error) = renewal.shutdown().await {
        tracing::warn!("merge lease renewal could not release the lease: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Mutex;

    use refact_work_core::store::CoordinationStore;
    use refact_work_core::ProjectKey;
    use uuid::Uuid;

    use crate::worktrees::types::{CreateWorktreeRequest, WorktreeMergeStrategy};

    use super::*;

    const LEASE_TTL_MS: u64 = 30_000;

    fn settings() -> CoordinationSettings {
        CoordinationSettings {
            lease_ttl_ms: LEASE_TTL_MS,
            lease_renew_interval_ms: 10_000,
            merge_lease_wait_ms: 10_000,
            merge_lease_poll_interval_ms: 10,
            ..CoordinationSettings::default()
        }
    }

    async fn lease_manager(instance: &str, project: &str) -> Arc<LeaseManager> {
        let store = Arc::new(
            CoordinationStore::open_in_memory(&settings())
                .await
                .unwrap(),
        );
        let access = store.authorize(ProjectKey::new(project));
        Arc::new(LeaseManager::new(store, access, instance, &settings()).unwrap())
    }

    async fn shared_gate(project: &str) -> (MergeGate, Arc<LeaseManager>) {
        let leases = lease_manager("engine-1", project).await;
        (MergeGate::new(leases.clone(), &settings()), leases)
    }

    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|error| panic!("git {args:?}: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn init_repo(root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn commit_file(root: &Path, file: &str, content: &str) {
        std::fs::write(root.join(file), content).unwrap();
        run_git(root, &["add", file]);
        run_git(root, &["commit", "-m", &format!("add {file}")]);
    }

    async fn worktree_with_commit(
        service: &WorktreeService,
        branch: &str,
        file: &str,
        content: &str,
    ) -> (String, PathBuf) {
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some(branch.to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(&created.worktree.meta.root, file, content);
        (created.worktree.meta.id, created.worktree.meta.root)
    }

    fn merge_request() -> MergeWorktreeRequest {
        MergeWorktreeRequest {
            strategy: WorktreeMergeStrategy::Squash,
            delete_after_merge: false,
            target_branch: Some("main".to_string()),
            commit_message: Some("merge under lease".to_string()),
            ..Default::default()
        }
    }

    fn verification(stage: VerificationStage, passed: bool) -> MergeVerification {
        MergeVerification {
            stage,
            command: "cargo test --lib".to_string(),
            passed,
            exit_code: Some(i32::from(!passed)),
            output_tail: String::new(),
            auto_reverted: false,
            merge_commit: None,
            revert_commit: None,
            fix_card_id: None,
            skipped_reason: None,
            revert_skipped_reason: None,
            outcome: if passed {
                VerificationOutcome::Passed
            } else {
                VerificationOutcome::CommandFailed
            },
        }
    }

    async fn nothing_verified(_checkout: PathBuf) -> Option<MergeVerification> {
        None
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merges_into_one_checkout_hold_the_slot_one_at_a_time() {
        let project = Uuid::new_v4().to_string();
        let (gate, _leases) = shared_gate(&project).await;
        let checkout = CheckoutKey::new("/repo/integration");
        let order = Arc::new(Mutex::new(Vec::new()));

        let merge = |label: &'static str| {
            let gate = &gate;
            let checkout = checkout.clone();
            let order = order.clone();
            async move {
                let slot = gate.acquire(&checkout).await.unwrap();
                order.lock().unwrap().push(format!("{label}-start"));
                tokio::time::sleep(Duration::from_millis(80)).await;
                order.lock().unwrap().push(format!("{label}-end"));
                let waited = slot.waited_ms();
                slot.release().await.unwrap();
                waited
            }
        };
        let (first, second) = tokio::join!(merge("a"), merge("b"));

        let order = order.lock().unwrap().clone();
        assert!(
            order == ["a-start", "a-end", "b-start", "b-end"]
                || order == ["b-start", "b-end", "a-start", "a-end"],
            "merges into one checkout interleaved: {order:?}"
        );
        assert!(
            first.max(second) > 0,
            "the second merge must have queued behind the first"
        );
    }

    #[tokio::test]
    async fn merges_into_different_checkouts_are_held_at_the_same_time() {
        let project = Uuid::new_v4().to_string();
        let (gate, _leases) = shared_gate(&project).await;
        let here = CheckoutKey::new("/repo/one");
        let there = CheckoutKey::new("/repo/two");

        let first = gate.acquire(&here).await.unwrap();
        let second = gate.acquire(&there).await.unwrap();
        let blocked = gate.acquire(&here).await.unwrap_err();

        assert_ne!(first.holder().lease_key, second.holder().lease_key);
        assert_eq!(first.checkout(), &here);
        assert_eq!(second.checkout(), &there);
        assert!(matches!(blocked, MergeGateError::Busy(_)), "{blocked}");
        first.release().await.unwrap();
        second.release().await.unwrap();
        assert!(
            gate.acquire(&here).await.is_ok(),
            "a released checkout is free again"
        );
    }

    #[tokio::test]
    #[serial_test::serial(coordination_metrics)]
    async fn a_merge_that_cannot_take_the_lease_fails_retryably_instead_of_blocking() {
        let project = Uuid::new_v4().to_string();
        let (holding_gate, _leases) = shared_gate(&project).await;
        let checkout = CheckoutKey::new("/repo/contended");
        let held = holding_gate.acquire(&checkout).await.unwrap();
        let impatient = MergeGate::new(
            holding_gate.leases.clone(),
            &CoordinationSettings {
                merge_lease_wait_ms: 120,
                merge_lease_poll_interval_ms: 10,
                ..settings()
            },
        );

        use crate::coordination::metrics as work_metrics;
        work_metrics::recorder().reset();
        let started = std::time::Instant::now();
        let error = impatient.acquire(&checkout).await.unwrap_err();
        let elapsed = started.elapsed();

        let explained = work_metrics::recorder().snapshot();
        let refusals = explained.explanations_for(work_metrics::Decision::MergeRefusal);
        assert_eq!(refusals.len(), 1, "the refusal was not explained");
        assert_eq!(
            refusals[0].rule, "merge_gate.lease_held_past_wait_deadline",
            "a merge refusal must name the rule that fired"
        );
        assert!(
            refusals[0]
                .inputs
                .iter()
                .any(|(name, value)| name == "reason" && value.contains("waited")),
            "a merge refusal must name the inputs that produced it: {:?}",
            refusals[0].inputs
        );

        match error {
            MergeGateError::Busy(busy) => {
                assert!(busy.retryable);
                assert_eq!(busy.checkout, "/repo/contended");
                assert_eq!(busy.lease_key, merge_lease_key(&checkout));
                assert_eq!(busy.holder_instance, "engine-1");
                assert!(busy.waited_ms >= 120, "gave up after {}ms", busy.waited_ms);
            }
            other => panic!("expected a retryable busy error, got {other}"),
        }
        assert!(
            elapsed < Duration::from_secs(5),
            "waiting for a busy checkout must be bounded, took {elapsed:?}"
        );
        held.release().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_concurrent_merges_into_one_checkout_both_land() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        init_repo(&source);
        let service = WorktreeService::new(temp.path().join("cache"), source.clone()).unwrap();
        let (gate, _leases) = shared_gate(&Uuid::new_v4().to_string()).await;
        let (first_id, _) = worktree_with_commit(&service, "refact/chat/a", "a.txt", "a\n").await;
        let (second_id, _) = worktree_with_commit(&service, "refact/chat/b", "b.txt", "b\n").await;

        let (first, second) = tokio::join!(
            merge_under_lease(
                &gate,
                &service,
                &first_id,
                merge_request(),
                None,
                nothing_verified
            ),
            merge_under_lease(
                &gate,
                &service,
                &second_id,
                merge_request(),
                None,
                nothing_verified
            )
        );

        let first = first.unwrap();
        let second = second.unwrap();
        assert!(first.response.merged, "{:?}", first.response.status);
        assert!(second.response.merged, "{:?}", second.response.status);
        assert!(source.join("a.txt").exists(), "first merge was lost");
        assert!(source.join("b.txt").exists(), "second merge was lost");
        assert!(
            first.waited_for_lease_ms > 0 || second.waited_for_lease_ms > 0,
            "one merge must have queued behind the other"
        );
    }

    #[tokio::test]
    async fn a_dirty_target_checkout_refuses_the_merge_and_the_user_changes_survive() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        init_repo(&source);
        let service = WorktreeService::new(temp.path().join("cache"), source.clone()).unwrap();
        let (gate, _leases) = shared_gate(&Uuid::new_v4().to_string()).await;
        let (id, worktree_root) =
            worktree_with_commit(&service, "refact/chat/dirty", "merged.txt", "merged\n").await;
        std::fs::write(source.join("file.txt"), "user was editing this\n").unwrap();
        std::fs::write(source.join("scratch.txt"), "untracked work\n").unwrap();

        let outcome = merge_under_lease(
            &gate,
            &service,
            &id,
            MergeWorktreeRequest {
                delete_after_merge: true,
                ..merge_request()
            },
            None,
            nothing_verified,
        )
        .await
        .unwrap();

        assert!(!outcome.response.merged);
        assert_eq!(outcome.response.status, "dirty_baseline");
        let baseline = outcome.refused_dirty_baseline().expect("dirty baseline");
        assert_eq!(baseline.branch.as_deref(), Some("main"));
        assert_eq!(baseline.unstaged_count, 1);
        assert_eq!(baseline.untracked_count, 1);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "user was editing this\n",
            "the merge trampled uncommitted user work"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("scratch.txt")).unwrap(),
            "untracked work\n"
        );
        assert!(!source.join("merged.txt").exists(), "nothing may be merged");
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
        assert!(worktree_root.exists(), "a refused merge keeps its worktree");
        assert!(outcome.source_preserved());
    }

    #[tokio::test]
    async fn an_acknowledged_dirty_baseline_merges_and_restores_the_user_changes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        init_repo(&source);
        let service = WorktreeService::new(temp.path().join("cache"), source.clone()).unwrap();
        let (gate, _leases) = shared_gate(&Uuid::new_v4().to_string()).await;
        let (id, _) =
            worktree_with_commit(&service, "refact/chat/ack", "merged.txt", "merged\n").await;
        std::fs::write(source.join("scratch.txt"), "untracked work\n").unwrap();

        let outcome = merge_under_lease(
            &gate,
            &service,
            &id,
            MergeWorktreeRequest {
                acknowledge_dirty_baseline: true,
                ..merge_request()
            },
            None,
            nothing_verified,
        )
        .await
        .unwrap();

        assert!(outcome.response.merged, "{}", outcome.response.status);
        assert!(outcome.refused_dirty_baseline().is_none());
        assert!(source.join("merged.txt").exists());
        assert_eq!(
            std::fs::read_to_string(source.join("scratch.txt")).unwrap(),
            "untracked work\n",
            "acknowledged dirt is restored, never discarded"
        );
    }

    #[tokio::test]
    async fn post_merge_verification_is_reported_apart_from_the_pre_merge_result() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        init_repo(&source);
        let service = WorktreeService::new(temp.path().join("cache"), source.clone()).unwrap();
        let (gate, _leases) = shared_gate(&Uuid::new_v4().to_string()).await;
        let (id, worktree_root) =
            worktree_with_commit(&service, "refact/chat/verify", "feature.txt", "feature\n").await;
        let verified_tree = Arc::new(Mutex::new(Vec::new()));
        let seen = verified_tree.clone();

        let outcome = merge_under_lease(
            &gate,
            &service,
            &id,
            MergeWorktreeRequest {
                delete_after_merge: true,
                ..merge_request()
            },
            Some(verification(VerificationStage::PreMerge, true)),
            |checkout: PathBuf| async move {
                seen.lock()
                    .unwrap()
                    .push(checkout.join("feature.txt").exists());
                Some(verification(VerificationStage::PostMerge, false))
            },
        )
        .await
        .unwrap();

        assert!(outcome.response.merged);
        assert_eq!(
            verified_tree.lock().unwrap().as_slice(),
            [true],
            "reverification must run against the merged tree"
        );
        assert!(outcome.pre_merge.as_ref().is_some_and(|pre| pre.passed));
        assert!(outcome
            .post_merge
            .as_ref()
            .is_some_and(|post| !post.passed && post.stage == VerificationStage::PostMerge));
        assert!(
            !outcome.merged_tree_verified(),
            "a pre-merge pass must never read as a post-merge pass"
        );
        assert!(
            worktree_root.exists() && outcome.source_preserved(),
            "a failed reverification keeps the source worktrees on disk"
        );
    }

    #[tokio::test]
    async fn infrastructure_verification_failure_preserves_source_without_auto_revert() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        init_repo(&source);
        let service = WorktreeService::new(temp.path().join("cache"), source.clone()).unwrap();
        let (gate, _leases) = shared_gate(&Uuid::new_v4().to_string()).await;
        let (id, worktree_root) = worktree_with_commit(
            &service,
            "refact/chat/infrastructure",
            "feature.txt",
            "feature\n",
        )
        .await;

        let outcome = merge_under_lease(
            &gate,
            &service,
            &id,
            MergeWorktreeRequest {
                delete_after_merge: true,
                ..merge_request()
            },
            None,
            |_checkout| async {
                Some(MergeVerification {
                    stage: VerificationStage::PostMerge,
                    command: "cargo test".into(),
                    passed: false,
                    exit_code: None,
                    output_tail: "runner unavailable".into(),
                    auto_reverted: false,
                    merge_commit: None,
                    revert_commit: None,
                    fix_card_id: None,
                    skipped_reason: None,
                    revert_skipped_reason: Some("verification infrastructure failed".into()),
                    outcome: VerificationOutcome::InfrastructureFailed,
                })
            },
        )
        .await
        .unwrap();

        let verification = outcome.post_merge.as_ref().unwrap();
        assert_eq!(
            verification.outcome,
            VerificationOutcome::InfrastructureFailed
        );
        assert_eq!(verification.output_tail, "runner unavailable");
        assert!(!verification.auto_reverted);
        assert!(worktree_root.exists());
        assert!(outcome.source_preserved());
        assert!(source.join("feature.txt").exists());
    }

    #[tokio::test]
    async fn a_verified_merge_is_the_only_one_that_retires_its_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        init_repo(&source);
        let service = WorktreeService::new(temp.path().join("cache"), source.clone()).unwrap();
        let (gate, _leases) = shared_gate(&Uuid::new_v4().to_string()).await;
        let (id, worktree_root) =
            worktree_with_commit(&service, "refact/chat/green", "green.txt", "green\n").await;

        let outcome =
            merge_under_lease(
                &gate,
                &service,
                &id,
                MergeWorktreeRequest {
                    delete_after_merge: true,
                    ..merge_request()
                },
                None,
                |_checkout: PathBuf| async move {
                    Some(verification(VerificationStage::PostMerge, true))
                },
            )
            .await
            .unwrap();

        assert!(outcome.merged_tree_verified());
        assert!(outcome.cleanup.is_some_and(|cleanup| cleanup.deleted));
        assert!(!worktree_root.exists());
    }

    #[tokio::test]
    async fn post_merge_revalidation_runs_fresh_evidence_against_the_merged_tree() {
        use crate::chat::evidence::{EvidenceDefaults, EvidenceRunner, SpecAuthority, VerificationSpec};
        use crate::chat::evidence_cache::{
            execute_cached, revalidate_merged_tree, test_cache, CacheMode,
        };
        use crate::chat::verify_cmd::VerifyCommandPolicy;
        use crate::tasks::types::{VerificationOutcome, VerificationResult};

        struct FixedRunner {
            exit_code: i32,
            seen: Arc<Mutex<Vec<PathBuf>>>,
        }

        #[async_trait::async_trait]
        impl EvidenceRunner for FixedRunner {
            async fn run(
                &mut self,
                worktree: &Path,
                spec: &VerificationSpec,
            ) -> VerificationResult {
                self.seen.lock().unwrap().push(worktree.to_path_buf());
                VerificationResult {
                    command: spec.command.clone(),
                    exit_code: Some(self.exit_code),
                    passed: self.exit_code == 0,
                    output_tail: "merged tree output".to_string(),
                    outcome: match self.exit_code {
                        0 => VerificationOutcome::Passed,
                        _ => VerificationOutcome::CommandFailed,
                    },
                    from_cache: false,
                    executed_at: None,
                }
            }
        }

        fn contract() -> VerificationSpec {
            let mut spec = VerificationSpec::propose(
                "T-1#0",
                "cargo test --lib",
                SpecAuthority::CardInstructions,
                &EvidenceDefaults {
                    timeout_secs: 60,
                    output_cap_bytes: 4_000,
                },
            );
            spec.cache.deterministic_inputs = vec!["feature.txt".to_string()];
            spec
        }

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        init_repo(&source);
        let service = WorktreeService::new(temp.path().join("cache"), source.clone()).unwrap();
        let (gate, _leases) = shared_gate(&Uuid::new_v4().to_string()).await;
        let (id, worktree_root) = worktree_with_commit(
            &service,
            "refact/chat/revalidate",
            "feature.txt",
            "feature\n",
        )
        .await;
        let cache = test_cache();

        let branch_service = crate::chat::evidence::EvidenceService::new(
            VerifyCommandPolicy::permissive(),
            crate::chat::evidence::EvidenceContext {
                worktree: worktree_root.clone(),
                commit: None,
                scope: "T-1".to_string(),
            },
        );
        let mut green = FixedRunner {
            exit_code: 0,
            seen: Arc::new(Mutex::new(Vec::new())),
        };
        let branch_evidence = execute_cached(
            &branch_service,
            &cache,
            contract(),
            &mut green,
            CacheMode::Replay,
            1_000,
        )
        .await;
        assert!(
            branch_evidence.verdict.is_passed(),
            "the branch proved itself green"
        );

        let checkouts = Arc::new(Mutex::new(Vec::new()));
        let seen = checkouts.clone();
        let outcome = merge_under_lease(
            &gate,
            &service,
            &id,
            MergeWorktreeRequest {
                delete_after_merge: true,
                ..merge_request()
            },
            Some(verification(VerificationStage::PreMerge, true)),
            |checkout: PathBuf| async move {
                let mut red = FixedRunner { exit_code: 1, seen };
                revalidate_merged_tree(
                    checkout,
                    vec![contract()],
                    VerifyCommandPolicy::permissive(),
                    "T-1".to_string(),
                    &cache,
                    &mut red,
                )
                .await
            },
        )
        .await
        .unwrap();

        assert!(outcome.response.merged);
        assert_eq!(
            checkouts.lock().unwrap().as_slice(),
            [source.clone()],
            "revalidation must execute against the merged checkout, not the branch worktree"
        );
        let post_merge = outcome
            .post_merge
            .as_ref()
            .expect("the merged tree is verified");
        assert_eq!(post_merge.stage, VerificationStage::PostMerge);
        assert!(
            !post_merge.passed,
            "a cached green branch result must never stand in for the merged tree"
        );
        assert_eq!(post_merge.exit_code, Some(1));
        assert!(outcome.pre_merge.as_ref().is_some_and(|pre| pre.passed));
        assert!(!outcome.merged_tree_verified());
        assert!(
            worktree_root.exists() && outcome.source_preserved(),
            "a failed reverification keeps the source worktree on disk"
        );
    }

    #[tokio::test]
    async fn a_conflicting_merge_reports_its_files_and_preserves_both_worktrees() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        init_repo(&source);
        let service = WorktreeService::new(temp.path().join("cache"), source.clone()).unwrap();
        let (gate, _leases) = shared_gate(&Uuid::new_v4().to_string()).await;
        let (id, worktree_root) =
            worktree_with_commit(&service, "refact/chat/conflict", "file.txt", "agent side\n")
                .await;
        commit_file(&source, "file.txt", "target side\n");

        let outcome = merge_under_lease(
            &gate,
            &service,
            &id,
            MergeWorktreeRequest {
                delete_after_merge: true,
                ..merge_request()
            },
            None,
            nothing_verified,
        )
        .await
        .unwrap();

        assert!(!outcome.response.merged);
        assert_eq!(outcome.response.status, "conflict");
        let conflict = outcome.conflict().expect("conflict state");
        assert_eq!(conflict.files, vec!["file.txt".to_string()]);
        assert!(
            !conflict.merge_in_progress,
            "conflicts are never left half-applied"
        );
        assert!(outcome.post_merge.is_none());
        assert!(worktree_root.exists() && outcome.source_preserved());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "target side\n"
        );
    }
}
