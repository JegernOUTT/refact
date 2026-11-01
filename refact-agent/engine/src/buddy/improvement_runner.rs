use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use refact_buddy_core::operator_types::{EvidenceRef, NativeSourceIdentity, NativeSourceKind};
use serde::{Deserialize, Serialize};

use super::improvement::{self, CandidateRecord, DeploymentTarget};
use super::improvement_evaluation::{
    digest, CheckObservation, CheckResult, EvaluationProof, EvaluationSpec, SPEC_NAMESPACE,
    PROOF_NAMESPACE,
};
use super::operator_store::{OperatorStore, OperatorStoreError, Stored};
use super::outcome_runtime::{OutcomeRuntime, WorkflowTerminal};
use crate::exec::{ExecRegistry, ExecSpawnRequest, ExecMode, ExecStatus, ExecOwnerMeta};
use crate::worktrees::service::WorktreeService;

pub const RUN_NAMESPACE: &str = "improvement_fixed_runs";
const COMPARISON_NAMESPACE: &str = "improvement_comparisons";
const FIXED_EVALUATION_PRODUCER: &str = "fixed_evaluation";
const FIXED_RUN_TERMINAL_BUDGET: usize = 16;

fn fixed_evaluation_terminal_slots(id: &str) -> [(&'static str, String, usize); 3] {
    [
        (RUN_NAMESPACE, id.to_owned(), FIXED_RUN_TERMINAL_BUDGET),
        (COMPARISON_NAMESPACE, id.to_owned(), 1),
        (PROOF_NAMESPACE, format!("fixed:{id}"), 1),
    ]
}

fn ensure_fixed_evaluation_reservation(
    store: &mut OperatorStore,
    id: &str,
    grandfathered: bool,
) -> Result<(), OperatorStoreError> {
    let slots = fixed_evaluation_terminal_slots(id);
    if grandfathered {
        store.reserve_grandfathered_terminal_documents(FIXED_EVALUATION_PRODUCER, id, &slots)
    } else {
        store.reserve_terminal_documents(FIXED_EVALUATION_PRODUCER, id, &slots)
    }
}

#[cfg(test)]
type ClaimGate = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
);
#[cfg(test)]
static CLAIM_GATES: std::sync::LazyLock<std::sync::Mutex<BTreeMap<(String, String), ClaimGate>>> =
    std::sync::LazyLock::new(Default::default);

#[cfg(test)]
pub(crate) struct ClaimGateGuard((String, String));
#[cfg(test)]
impl Drop for ClaimGateGuard {
    fn drop(&mut self) {
        CLAIM_GATES.lock().unwrap().remove(&self.0);
    }
}
#[cfg(test)]
pub(crate) fn gate_claim(
    project: String,
    run: String,
) -> (
    ClaimGateGuard,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let key = (project, run);
    let (claimed_tx, claimed_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    assert!(CLAIM_GATES
        .lock()
        .unwrap()
        .insert(key.clone(), (claimed_tx, release_rx, done_tx))
        .is_none());
    (ClaimGateGuard(key), claimed_rx, release_tx, done_rx)
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub async fn evaluate_owned(
    app: &crate::app_state::AppState,
    runtime: OutcomeRuntime,
    service: WorktreeService,
    candidate_id: String,
    grant: EvaluationGrant,
    requester: NativeSourceIdentity,
) -> Result<EvaluationProof, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let mut buddy = app.buddy.buddy.lock().await;
    let owner = buddy.as_mut().ok_or("Buddy service unavailable")?;
    if app.gcx.shutdown_flag.load(Ordering::Acquire)
        || !owner
            .operator_features
            .allows(super::operator_runtime::OperatorCapability::Improvement)
        || owner.outcome_runtime.as_ref().is_none_or(|current| {
            current.worker() != runtime.worker() || current.project() != runtime.project()
        })
    {
        return Err("evaluation owner changed or stopped".into());
    }
    let registry = app.runtime.exec_registry.as_ref().clone();
    let worker = runtime.worker();
    let project = runtime.project();
    let task_cancel = cancel.clone();
    crate::agents::spawn::track_cancellable_spawn_owner(
        async move {
            let result = evaluate(
                runtime,
                service,
                registry,
                candidate_id,
                grant,
                requester,
                task_cancel,
            )
            .await;
            let _ = result_tx.send(result);
            let _ = done_tx.send(());
        },
        &cancel,
    )?;
    let guard = CancelOnDrop(cancel);
    let shutdown = app.gcx.shutdown_flag.clone();
    let buddy_state = app.buddy.buddy.clone();
    owner.track_background_task(tokio::spawn(async move {
        let _guard = guard;
        tokio::pin!(done_rx);
        loop {
            tokio::select! {
                _ = &mut done_rx => break,
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    let stopped = shutdown.load(Ordering::Acquire) || buddy_state.lock().await.as_ref().is_none_or(|owner|
                        !owner.operator_features.allows(super::operator_runtime::OperatorCapability::Improvement)
                        || owner.outcome_runtime.as_ref().is_none_or(|current| current.worker() != worker || current.project() != project));
                    if stopped { _guard.0.store(true, Ordering::Release); }
                }
            }
        }
    }));
    drop(buddy);
    result_rx
        .await
        .map_err(|_| "evaluation owner ended without a result".to_string())?
}
// Full lifecycle tests use scratch stores; no native process is launched by fake checks.
#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_run(
        project: refact_buddy_core::operator_types::ProjectIdentity,
        id: &str,
    ) -> FixedRun {
        FixedRun {
            schema_version: 1,
            project,
            grant: EvaluationGrant {
                candidate_digest: "candidate".into(),
                evaluation_digest: "evaluation".into(),
                run_id: id.into(),
                expires_at: "2099-01-01T00:00:00Z".into(),
            },
            requester: NativeSourceIdentity {
                source_kind: NativeSourceKind::Check,
                source_id: "requester".into(),
                source_version: "v1".into(),
            },
            evaluator: NativeSourceIdentity {
                source_kind: NativeSourceKind::Check,
                source_id: "evaluator".into(),
                source_version: "v1".into(),
            },
            candidate_id: "candidate".into(),
            evaluation_spec_id: Some("spec".into()),
            state: "claimed_outcome_unknown_until_finalized".into(),
            process_id: None,
            cancel_requested: false,
            exact: None,
        }
    }

    #[tokio::test]
    async fn owner_abort_before_first_poll_signals_cancellation() {
        let cancel = Arc::new(AtomicBool::new(false));
        let guard = CancelOnDrop(cancel.clone());
        let task = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(cancel.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn disconnected_result_receiver_does_not_drop_owned_execution() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel::<()>();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let flag = cancel.clone();
        crate::agents::spawn::track_cancellable_spawn_owner(
            async move {
                release_rx.await.unwrap();
                assert!(!flag.load(Ordering::Acquire));
                assert!(result_tx.send(()).is_err());
                finished_tx.send(()).unwrap();
            },
            &cancel,
        )
        .unwrap();
        drop(result_rx);
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), finished_rx)
            .await
            .unwrap()
            .unwrap();
    }

    struct FakeExecutor;
    #[async_trait::async_trait]
    impl Executor for FakeExecutor {
        async fn execute(
            &self,
            command: &FixedCommand,
            root: &Path,
            scratch: &Path,
            limit: usize,
            cancel: Arc<AtomicBool>,
        ) -> Result<CommandResult, String> {
            assert!(root != scratch);
            let request = native_request(command, root, scratch, limit, cancel.clone());
            assert!(!request.sandbox.as_ref().unwrap().allow_network);
            assert!(request
                .argv
                .as_ref()
                .unwrap()
                .iter()
                .any(|arg| arg == "--clearenv"));
            std::fs::write(scratch.join("observed"), b"fixed check").unwrap();
            Ok(CommandResult {
                result: if cancel.load(Ordering::Acquire) {
                    CheckResult::Unavailable
                } else {
                    CheckResult::Passed
                },
                status: "fake scratch executor".into(),
                output_digest: "a".repeat(64),
                output_bytes: 11,
            })
        }
    }

    #[test]
    fn scratch_snapshot_detects_content_drift_and_rejects_symlink() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a"), "first").unwrap();
        let before = digest(&snapshot_files(root.path()).unwrap()).unwrap();
        std::fs::write(root.path().join("a"), "second").unwrap();
        assert_ne!(
            before,
            digest(&snapshot_files(root.path()).unwrap()).unwrap()
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", root.path().join("escape")).unwrap();
            assert!(snapshot_files(root.path()).is_err());
        }
    }

    #[test]
    fn saturated_admission_prevents_the_fixed_run_claim() {
        let root = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "saturated-fixed-evaluation".into(),
            canonical_root: dunce::canonicalize(root.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        store
            .set_document_growth_policy(Some(
                super::super::operator_observability::DocumentGrowthPolicy {
                    max_rows_per_namespace: 1,
                    replace_editorial_aggregate: false,
                },
            ))
            .unwrap();
        assert!(ensure_fixed_evaluation_reservation(&mut store, "blocked-run", false).is_err());
        assert!(store
            .get_document::<FixedRun>(RUN_NAMESPACE, "blocked-run")
            .unwrap()
            .is_none());
    }

    #[test]
    fn unknown_fixed_evaluation_retains_remaining_reservation() {
        let root = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "unknown-fixed-evaluation".into(),
            canonical_root: dunce::canonicalize(root.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let mut run = fixed_run(project.clone(), "unknown-run");
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        ensure_fixed_evaluation_reservation(&mut store, "unknown-run", false).unwrap();
        let claim = store
            .put_reserved_document(
                FIXED_EVALUATION_PRODUCER,
                "unknown-run",
                RUN_NAMESPACE,
                "unknown-run",
                &run,
                None,
            )
            .unwrap();
        run.state = "stopped_outcome_unknown_no_replay".into();
        store
            .put_reserved_document(
                FIXED_EVALUATION_PRODUCER,
                "unknown-run",
                RUN_NAMESPACE,
                "unknown-run",
                &run,
                Some(claim.revision),
            )
            .unwrap();
        let reservations = store
            .diagnostic_summary()
            .unwrap()
            .document_growth
            .terminal_reservations;
        assert_eq!(reservations.retained_entries, 3);
        assert_eq!(reservations.outstanding_slots, 16);
    }

    #[test]
    fn fixed_evaluation_releases_only_with_durable_terminal_run() {
        let root = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "terminal-fixed-evaluation".into(),
            canonical_root: dunce::canonicalize(root.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let mut run = fixed_run(project.clone(), "terminal-run");
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        ensure_fixed_evaluation_reservation(&mut store, "terminal-run", false).unwrap();
        let claim = store
            .put_reserved_document(
                FIXED_EVALUATION_PRODUCER,
                "terminal-run",
                RUN_NAMESPACE,
                "terminal-run",
                &run,
                None,
            )
            .unwrap();
        assert_eq!(
            store
                .diagnostic_summary()
                .unwrap()
                .document_growth
                .terminal_reservations
                .outstanding_slots,
            17
        );
        let comparison = serde_json::json!({
            "schema_version": 1,
            "project": project.clone(),
            "kind": "comparison"
        });
        store
            .put_reserved_document(
                FIXED_EVALUATION_PRODUCER,
                "terminal-run",
                COMPARISON_NAMESPACE,
                "terminal-run",
                &comparison,
                None,
            )
            .unwrap();
        let proof = serde_json::json!({
            "schema_version": 1,
            "project": run.project.clone(),
            "kind": "proof"
        });
        store
            .put_reserved_document(
                FIXED_EVALUATION_PRODUCER,
                "terminal-run",
                PROOF_NAMESPACE,
                "fixed:terminal-run",
                &proof,
                None,
            )
            .unwrap();
        run.state = "finished_no_activation".into();
        store
            .put_terminal_document(
                FIXED_EVALUATION_PRODUCER,
                "terminal-run",
                RUN_NAMESPACE,
                "terminal-run",
                &run,
                Some(claim.revision),
            )
            .unwrap();
        let reservations = store
            .diagnostic_summary()
            .unwrap()
            .document_growth
            .terminal_reservations;
        assert_eq!(reservations.retained_entries, 3);
        assert_eq!(reservations.outstanding_slots, 0);
        assert_eq!(
            store
                .get_document::<FixedRun>(RUN_NAMESPACE, "terminal-run")
                .unwrap()
                .unwrap()
                .value
                .state,
            "finished_no_activation"
        );
    }

    #[test]
    fn legacy_active_evaluation_can_be_cancelled_after_saturation() {
        let root = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "legacy-fixed-evaluation".into(),
            canonical_root: dunce::canonicalize(root.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let run = fixed_run(project.clone(), "legacy-run");
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        store
            .put_document(RUN_NAMESPACE, "legacy-run", &run, None)
            .unwrap();
        let database = store.path().to_owned();
        drop(store);
        let connection = rusqlite::Connection::open(database).unwrap();
        connection
            .execute_batch("DROP TABLE terminal_reservations; PRAGMA user_version=5;")
            .unwrap();
        drop(connection);
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        store
            .set_document_growth_policy(Some(
                super::super::operator_observability::DocumentGrowthPolicy {
                    max_rows_per_namespace: 1,
                    replace_editorial_aggregate: false,
                },
            ))
            .unwrap();
        assert!(store
            .reserve_terminal_documents(
                FIXED_EVALUATION_PRODUCER,
                "legacy-run",
                &fixed_evaluation_terminal_slots("legacy-run"),
            )
            .is_err());
        let cancelled = cancel_run(&mut store, "legacy-run").unwrap();
        assert!(cancelled.value.cancel_requested);
        assert_eq!(
            cancelled.value.state,
            "claimed_outcome_unknown_until_finalized"
        );
        let reservations = store
            .diagnostic_summary()
            .unwrap()
            .document_growth
            .terminal_reservations;
        assert_eq!(reservations.retained_entries, 3);
        assert_eq!(reservations.outstanding_slots, 17);
        assert_eq!(reservations.grandfathered_entries, 3);
    }
}

const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FixedCommand {
    pub criterion_id: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub timeout_seconds: u32,
    pub expected_exit: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CorpusCase {
    pub id: String,
    pub intent: String,
    pub quiet: bool,
    pub expected_delivery: bool,
    pub max_chars: usize,
    pub forbidden: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfflineProfile {
    pub outputs: BTreeMap<String, String>,
    pub respect_quiet: bool,
}

/// Versioned resource-draft document; the immutable candidate hashes this entire envelope.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineProfileDraft {
    pub schema_version: u16,
    pub project: refact_buddy_core::operator_types::ProjectIdentity,
    pub profile: OfflineProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationComparison {
    pub schema_version: u16,
    pub project: refact_buddy_core::operator_types::ProjectIdentity,
    pub run_id: String,
    pub candidate_digest: String,
    pub evaluation_digest: String,
    pub cases: Vec<CaseComparison>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfflineComparison {
    pub criterion_id: String,
    pub cases: Vec<CorpusCase>,
    pub baseline: OfflineProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FixedExecution {
    pub commands: Vec<FixedCommand>,
    pub offline: Option<OfflineComparison>,
    pub output_limit_bytes: usize,
    pub total_timeout_seconds: u32,
}

fn invalid(message: &'static str) -> OperatorStoreError {
    OperatorStoreError::InvalidInput(message)
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|p| matches!(p, Component::Normal(_)))
}

impl FixedExecution {
    pub fn validate(&self, spec: &EvaluationSpec) -> Result<(), OperatorStoreError> {
        if spec.criteria.len() > 12
            || self.commands.len() > 10
            || !(1024..=65_536).contains(&self.output_limit_bytes)
            || !(1..=3600).contains(&self.total_timeout_seconds)
            || (self.commands.is_empty() && self.offline.is_none())
        {
            return Err(invalid("invalid fixed execution limits"));
        }
        let mut ids = BTreeSet::new();
        for command in &self.commands {
            if !ids.insert(command.criterion_id.clone())
                || command.argv.is_empty()
                || command.argv.len() > 64
                || !command.argv[0].starts_with("/usr/bin/")
                || command
                    .argv
                    .iter()
                    .any(|arg| arg.len() > 4096 || arg.contains('\0'))
                || (command.cwd != "." && !relative(&command.cwd))
                || command.timeout_seconds == 0
                || command.timeout_seconds > self.total_timeout_seconds
                || command.expected_exit != 0
            {
                return Err(invalid("invalid preauthorized argv or command bounds"));
            }
        }
        if let Some(offline) = &self.offline {
            if !ids.insert(offline.criterion_id.clone())
                || offline.cases.is_empty()
                || offline.cases.len() > 120
                || digest(&offline.cases)? != spec.corpus_digest
            {
                return Err(invalid("offline corpus does not match fixed specification"));
            }
            let mut cases = BTreeSet::new();
            for case in &offline.cases {
                if !cases.insert(&case.id)
                    || case.id.is_empty()
                    || case.intent.is_empty()
                    || case.max_chars > 2048
                    || case.forbidden.len() > 32
                    || case.forbidden.iter().any(|s| s.is_empty() || s.len() > 128)
                {
                    return Err(invalid("invalid fixed corpus case"));
                }
            }
            validate_profile(&offline.baseline)?;
        }
        for id in ids {
            if !spec
                .criteria
                .iter()
                .any(|c| c.id == id && !c.external_review)
            {
                return Err(invalid(
                    "executable criterion absent or is an external review gate",
                ));
            }
        }
        Ok(())
    }
}

fn validate_profile(profile: &OfflineProfile) -> Result<(), OperatorStoreError> {
    if profile.outputs.len() > 120
        || profile
            .outputs
            .iter()
            .any(|(k, v)| k.len() > 128 || v.len() > 4096 || v.contains('\0'))
    {
        return Err(invalid("offline profile exceeds bounds"));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaseComparison {
    pub case_id: String,
    pub baseline_output: String,
    pub candidate_output: String,
    pub baseline_pass: bool,
    pub candidate_pass: bool,
}

pub fn compare_profiles(
    fixed: &OfflineComparison,
    candidate: &OfflineProfile,
) -> Result<Vec<CaseComparison>, OperatorStoreError> {
    validate_profile(candidate)?;
    Ok(fixed
        .cases
        .iter()
        .map(|case| {
            let output = |profile: &OfflineProfile| {
                if case.quiet && profile.respect_quiet {
                    String::new()
                } else {
                    profile
                        .outputs
                        .get(&case.intent)
                        .cloned()
                        .unwrap_or_default()
                }
            };
            let baseline = output(&fixed.baseline);
            let proposed = output(candidate);
            let passes = |text: &str| {
                (!text.is_empty()) == case.expected_delivery
                    && text.chars().count() <= case.max_chars
                    && !case.forbidden.iter().any(|needle| text.contains(needle))
            };
            CaseComparison {
                case_id: case.id.clone(),
                baseline_pass: passes(&baseline),
                candidate_pass: passes(&proposed),
                baseline_output: baseline,
                candidate_output: proposed,
            }
        })
        .collect())
}

pub fn runner_origin(
    worker: &refact_buddy_core::operator_types::WorkerIdentity,
) -> NativeSourceIdentity {
    NativeSourceIdentity {
        source_kind: NativeSourceKind::Check,
        source_id: "buddy-fixed-evaluator".into(),
        source_version: format!(
            "{}:{}:{}:v1",
            worker.engine_instance_id, worker.worker_id, worker.epoch
        ),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExactIdentity {
    pub worktree_id: String,
    pub root: String,
    pub patch_digest: String,
    pub source_digest: String,
}

struct Snapshot {
    identity: ExactIdentity,
    files: BTreeMap<String, Vec<u8>>,
}

fn snapshot_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    let mut bytes = 0u64;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let name = entry.file_name();
            if [".git", ".refact", "target", "node_modules"]
                .iter()
                .any(|skip| name == *skip)
            {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                return Err("candidate snapshot contains symlink".into());
            }
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if !meta.is_file() {
                return Err("candidate snapshot contains special file".into());
            }
            bytes = bytes.saturating_add(meta.len());
            if bytes > MAX_BYTES || files.len() >= MAX_FILES {
                return Err("candidate snapshot exceeds fixed resource bounds".into());
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("non-UTF8 candidate path")?
                .to_string();
            use std::io::Read;
            let mut content = Vec::new();
            std::fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(MAX_BYTES + 1)
                .read_to_end(&mut content)
                .map_err(|e| e.to_string())?;
            bytes = bytes
                .saturating_sub(meta.len())
                .saturating_add(content.len() as u64);
            if bytes > MAX_BYTES {
                return Err("candidate changed beyond fixed byte bounds".into());
            }
            files.insert(relative, content);
        }
    }
    Ok(files)
}

async fn snapshot(
    service: &WorktreeService,
    id: &str,
    targets: &[String],
) -> Result<Snapshot, String> {
    service.validate_registered_worktree_checkout(id).await?;
    let view = service.get_worktree(id).await?;
    if view.meta.root == service.source_workspace_root()
        || view.meta.source_workspace_root != service.source_workspace_root()
        || !view.status.path_exists
        || !view.status.is_git_worktree
        || view.status.conflicted
        || view.status.error.is_some()
    {
        return Err("candidate must be a healthy native isolated worktree".into());
    }
    let diff = service.diff_worktree(id).await?;
    if diff.patch_truncated
        || diff
            .files
            .iter()
            .any(|file| !targets.contains(&file.path) || !relative(&file.path))
    {
        return Err("candidate diff is truncated or contains unrelated changes".into());
    }
    let root = view.meta.root.clone();
    let files = tokio::task::spawn_blocking(move || snapshot_files(&root))
        .await
        .map_err(|e| e.to_string())??;
    for file in &diff.files {
        if file
            .path
            .split('/')
            .any(|p| [".git", ".refact", "target", "node_modules"].contains(&p))
        {
            return Err("candidate edits excluded evaluator/runtime state".into());
        }
    }
    let identity = ExactIdentity {
        worktree_id: id.into(),
        root: view.meta.root.to_string_lossy().into_owned(),
        patch_digest: digest(&diff).map_err(|e| e.to_string())?,
        source_digest: digest(&files).map_err(|e| e.to_string())?,
    };
    Ok(Snapshot { identity, files })
}

pub async fn inspect_source(
    service: &WorktreeService,
    id: &str,
    targets: &[String],
) -> Result<ExactIdentity, String> {
    Ok(snapshot(service, id, targets).await?.identity)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationGrant {
    pub candidate_digest: String,
    pub evaluation_digest: String,
    pub run_id: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FixedRun {
    pub schema_version: u16,
    pub project: refact_buddy_core::operator_types::ProjectIdentity,
    pub grant: EvaluationGrant,
    pub requester: NativeSourceIdentity,
    pub evaluator: NativeSourceIdentity,
    pub candidate_id: String,
    #[serde(default)]
    pub evaluation_spec_id: Option<String>,
    pub state: String,
    pub process_id: Option<String>,
    pub cancel_requested: bool,
    pub exact: Option<ExactIdentity>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EvaluationStatus {
    pub run: Stored<FixedRun>,
    pub outcome: Option<Stored<refact_buddy_core::operator_types::RunOutcome>>,
    pub evaluation_spec_id: Option<String>,
    pub proof_status: &'static str,
    pub proof: Option<Stored<EvaluationProof>>,
}

pub fn evaluation_status(
    store: &OperatorStore,
    id: &str,
) -> Result<Option<EvaluationStatus>, OperatorStoreError> {
    let Some(run) = store.get_document::<FixedRun>(RUN_NAMESPACE, id)? else {
        return Ok(None);
    };
    if run.value.grant.run_id != id {
        return Err(invalid("evaluation run identity mismatch"));
    }
    let outcome = store.get_run_outcome(id)?;
    let candidate = store.get_document::<CandidateRecord>(
        improvement::CANDIDATE_NAMESPACE,
        &run.value.candidate_id,
    )?;
    let candidate = candidate.filter(|candidate| {
        candidate.value.candidate.candidate_id == run.value.candidate_id
            && digest(&candidate.value).ok().as_ref() == Some(&run.value.grant.candidate_digest)
    });
    let evaluation_spec_id = run.value.evaluation_spec_id.clone().or_else(|| {
        candidate
            .as_ref()
            .map(|candidate| candidate.value.candidate.evaluation_spec_version.clone())
    });
    let mut proof_status = "not_finalized";
    let mut proof = None;
    if run.value.state == "finished_no_activation" {
        proof_status = "missing";
        if let Some(stored) =
            store.get_document::<EvaluationProof>(PROOF_NAMESPACE, &format!("fixed:{id}"))?
        {
            proof_status = "withheld_stale_or_mismatched";
            if let Some(candidate) = candidate {
                if let Some(spec) = store.get_document::<EvaluationSpec>(
                    SPEC_NAMESPACE,
                    &candidate.value.candidate.evaluation_spec_version,
                )? {
                    let draft_current = match &candidate.value.candidate.config_draft_ref {
                        Some(id) => store
                            .get_document::<serde_json::Value>("resource_draft", id)?
                            .is_some_and(|draft| {
                                digest(&draft.value).ok().as_ref()
                                    == Some(&candidate.value.content_digest)
                            }),
                        None => true,
                    };
                    if draft_current
                        && evaluation_spec_id.as_deref() == Some(spec.value.id.as_str())
                        && stored.value.id == format!("fixed:{id}")
                        && stored.value.authority == run.value.evaluator
                        && stored.value.checks.iter().all(|check| check.run_id == id)
                        && super::improvement_evaluation::validate_proof(
                            store,
                            &spec.value,
                            &stored.value,
                            &run.value.grant.candidate_digest,
                            chrono::Utc::now(),
                            false,
                        )
                        .is_ok()
                    {
                        proof_status = "current_no_activation";
                        proof = Some(stored);
                    }
                }
            }
        }
    }
    Ok(Some(EvaluationStatus {
        run,
        outcome,
        evaluation_spec_id,
        proof_status,
        proof,
    }))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandResult {
    pub result: CheckResult,
    pub status: String,
    pub output_digest: String,
    pub output_bytes: usize,
}

#[async_trait::async_trait]
trait Executor: Send + Sync {
    async fn execute(
        &self,
        command: &FixedCommand,
        root: &Path,
        scratch: &Path,
        limit: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<CommandResult, String>;
}

struct NativeExecutor {
    registry: ExecRegistry,
    runtime: OutcomeRuntime,
    run_id: String,
}

fn native_request(
    command: &FixedCommand,
    root: &Path,
    scratch: &Path,
    limit: usize,
    cancel: Arc<AtomicBool>,
) -> ExecSpawnRequest {
    let mut argv = vec![
        "/usr/bin/bwrap".into(),
        "--unshare-all".into(),
        "--die-with-parent".into(),
        "--new-session".into(),
        "--tmpfs".into(),
        "/".into(),
    ];
    for system in ["/usr", "/bin", "/lib", "/lib64"] {
        if Path::new(system).exists() {
            argv.extend(["--ro-bind".into(), system.into(), system.into()]);
        }
    }
    argv.extend([
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--ro-bind".into(),
        root.to_string_lossy().into_owned(),
        "/candidate".into(),
        "--bind".into(),
        scratch.to_string_lossy().into_owned(),
        "/output".into(),
        "--chdir".into(),
        if command.cwd == "." {
            "/candidate".into()
        } else {
            format!("/candidate/{}", command.cwd)
        },
        "--clearenv".into(),
        "--setenv".into(),
        "PATH".into(),
        "/usr/bin:/bin".into(),
        "--setenv".into(),
        "HOME".into(),
        "/output".into(),
        "--setenv".into(),
        "CARGO_TARGET_DIR".into(),
        "/output/target".into(),
        "--".into(),
    ]);
    argv.extend(command.argv.clone());
    ExecSpawnRequest::argv(ExecMode::Background, argv)
        .with_cwd(scratch)
        .with_tty(false)
        .with_sandbox(crate::exec::types::ExecSandboxSpec {
            mode: crate::exec::types::ExecSandboxMode::WorkspaceWrite,
            ro_paths: vec![root.into()],
            rw_paths: vec![scratch.into(), root.into()],
            allow_network: false,
        })
        .with_timeout(Duration::from_secs(u64::from(command.timeout_seconds)))
        .with_transcript_limit(limit)
        .with_abort_flag(cancel)
        .with_output_drain_timeout(Duration::from_secs(2))
        .with_short_description("Run preauthorized fixed candidate check")
}

#[async_trait::async_trait]
impl Executor for NativeExecutor {
    async fn execute(
        &self,
        command: &FixedCommand,
        root: &Path,
        scratch: &Path,
        limit: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<CommandResult, String> {
        let status = tokio::task::spawn_blocking(refact_sandbox::sandbox_status)
            .await
            .map_err(|e| e.to_string())?;
        if status.provider != "bwrap"
            || status.enforcement != refact_sandbox::Enforcement::Full
            || !Path::new("/usr/bin/bwrap").is_file()
        {
            return Err("required whole-process bwrap confinement unavailable; no fallback".into());
        }
        let mut request = native_request(command, root, scratch, limit, cancel.clone());
        request.owner = ExecOwnerMeta {
            chat_id: None,
            tool_call_id: Some(self.run_id.clone()),
            service_name: None,
            workspace: Some(scratch.into()),
        };
        let started = self.registry.spawn(request).await?;
        let process = started.snapshot.meta.process_id;
        let id = self.run_id.clone();
        let process_text = process.to_string();
        if let Err(error) = self
            .runtime
            .with_store(move |store| {
                let current: Stored<FixedRun> = store
                    .get_document(RUN_NAMESPACE, &id)?
                    .ok_or(invalid("evaluation claim missing"))?;
                let mut run = current.value;
                run.process_id = Some(process_text);
                store.put_reserved_document(
                    FIXED_EVALUATION_PRODUCER,
                    &id,
                    RUN_NAMESPACE,
                    &id,
                    &run,
                    Some(current.revision),
                )
            })
            .await
        {
            let _ = self.registry.remove(&process).await;
            return Err(error.to_string());
        }
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        let terminal = loop {
            tokio::select! {
                result = self.registry.wait(&process) => break result,
                _ = tick.tick() => {
                    let id = self.run_id.clone();
                    let revoked = self.runtime.with_store(move |store| {
                        Ok(store.get_document::<FixedRun>(RUN_NAMESPACE, &id)?.is_none_or(|r| r.value.cancel_requested))
                    }).await.unwrap_or(true);
                    if revoked { cancel.store(true, Ordering::Release); }
                    let read = self.registry.read(&process, 0, Some(0)).await;
                    if read.total_bytes_appended > limit { cancel.store(true, Ordering::Release); }
                }
            }
        };
        let terminal = match terminal {
            Ok(terminal) => terminal,
            Err(error) => {
                cancel.store(true, Ordering::Release);
                self.registry.remove(&process).await?;
                return Err(error);
            }
        };
        let read = self.registry.read(&process, 0, None).await;
        let output_digest = digest(&read.chunks).map_err(|e| e.to_string());
        let result = if read.is_truncated || read.total_bytes_appended > limit {
            CheckResult::Inconclusive
        } else {
            match terminal.status {
                ExecStatus::Exited {
                    exit_code: Some(code),
                } if code == command.expected_exit => CheckResult::Passed,
                ExecStatus::Exited { .. } => CheckResult::Failed,
                _ => CheckResult::Unavailable,
            }
        };
        let status = format!("{:?}", terminal.status.kind());
        self.registry.remove(&process).await?;
        Ok(CommandResult {
            result,
            status,
            output_digest: output_digest?,
            output_bytes: read.total_bytes_appended,
        })
    }
}

fn check_grant(
    grant: &EvaluationGrant,
    record: &CandidateRecord,
    spec: &EvaluationSpec,
) -> Result<(), OperatorStoreError> {
    let expires = chrono::DateTime::parse_from_rfc3339(&grant.expires_at)
        .map_err(|_| invalid("invalid evaluation grant expiry"))?;
    let now = chrono::Utc::now();
    if grant.run_id.is_empty()
        || grant.run_id.len() > 128
        || grant.candidate_digest != digest(record)?
        || grant.evaluation_digest != digest(spec)?
        || expires <= now
        || expires > now + chrono::Duration::hours(1)
    {
        return Err(invalid(
            "grant must bind exact candidate, fixed spec and bounded expiry",
        ));
    }
    Ok(())
}

pub async fn evaluate(
    runtime: OutcomeRuntime,
    service: WorktreeService,
    registry: ExecRegistry,
    candidate_id: String,
    grant: EvaluationGrant,
    requester: NativeSourceIdentity,
    shutdown: Arc<AtomicBool>,
) -> Result<EvaluationProof, String> {
    let _cancel_on_drop = CancelOnDrop(shutdown.clone());
    let id = candidate_id.clone();
    let grant_copy = grant.clone();
    let evaluator = runner_origin(&runtime.worker());
    let evaluator_copy = evaluator.clone();
    let (record, spec) = runtime
        .with_store(move |store| {
            let record = improvement::candidate(store, &id)?.value;
            let spec: Stored<EvaluationSpec> = store
                .get_document(SPEC_NAMESPACE, &record.candidate.evaluation_spec_version)?
                .ok_or(invalid("fixed evaluation unavailable"))?;
            check_grant(&grant_copy, &record, &spec.value)?;
            spec.value
                .execution
                .as_ref()
                .ok_or(invalid("spec contains no fixed executor"))?
                .validate(&spec.value)?;
            if requester != spec.value.authority {
                return Err(invalid(
                    "only authenticated fixed-spec owner can grant execution",
                ));
            }
            let claim = FixedRun {
                schema_version: 1,
                project: record.project.clone(),
                grant: grant_copy,
                requester,
                evaluator: evaluator_copy,
                candidate_id: id,
                evaluation_spec_id: Some(spec.value.id.clone()),
                state: "claimed_outcome_unknown_until_finalized".into(),
                process_id: None,
                cancel_requested: false,
                exact: None,
            };
            // A duplicate run never re-enters the executor. Reserve before the first
            // durable claim and before workflow/process effects, including cancellation.
            if store
                .get_document::<FixedRun>(RUN_NAMESPACE, &claim.grant.run_id)?
                .is_some()
            {
                return Err(invalid(
                    "evaluation already claimed; inspect retained outcome, no replay",
                ));
            }
            ensure_fixed_evaluation_reservation(store, &claim.grant.run_id, false)?;
            store.put_reserved_document(
                FIXED_EVALUATION_PRODUCER,
                &claim.grant.run_id,
                RUN_NAMESPACE,
                &claim.grant.run_id,
                &claim,
                None,
            )?;
            Ok((record, spec.value))
        })
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(test)]
    let test_done = {
        let gate = CLAIM_GATES
            .lock()
            .unwrap()
            .remove(&(runtime.project().canonical_root, grant.run_id.clone()));
        if let Some((claimed, release, done)) = gate {
            let _ = claimed.send(());
            let _ = release.await;
            Some(done)
        } else {
            None
        }
    };
    let executor = NativeExecutor {
        registry,
        runtime: runtime.clone(),
        run_id: grant.run_id.clone(),
    };
    let result = evaluate_claimed(
        &runtime,
        &service,
        &record,
        &spec,
        &grant,
        &evaluator,
        &executor,
        shutdown.clone(),
    )
    .await;
    if shutdown.load(Ordering::Acquire) || result.is_err() {
        let id = grant.run_id.clone();
        let cancelled = shutdown.load(Ordering::Acquire);
        runtime
            .with_store(move |store| {
                ensure_fixed_evaluation_reservation(store, &id, true)?;
                let current: Stored<FixedRun> = store
                    .get_document(RUN_NAMESPACE, &id)?
                    .ok_or(invalid("evaluation claim missing"))?;
                let mut run = current.value;
                run.cancel_requested |= cancelled;
                if run.state != "finished_no_activation" {
                    run.state = "stopped_outcome_unknown_no_replay".into();
                }
                store.put_reserved_document(
                    FIXED_EVALUATION_PRODUCER,
                    &id,
                    RUN_NAMESPACE,
                    &id,
                    &run,
                    Some(current.revision),
                )
            })
            .await
            .map_err(|e| e.to_string())?;
    }
    #[cfg(test)]
    if let Some(done) = test_done {
        let _ = done.send(());
    }
    result
}

async fn evaluate_claimed(
    runtime: &OutcomeRuntime,
    service: &WorktreeService,
    record: &CandidateRecord,
    spec: &EvaluationSpec,
    grant: &EvaluationGrant,
    evaluator: &NativeSourceIdentity,
    executor: &dyn Executor,
    cancel: Arc<AtomicBool>,
) -> Result<EvaluationProof, String> {
    let fixed = spec
        .execution
        .as_ref()
        .ok_or("fixed executor unavailable")?;
    let cancel_id = grant.run_id.clone();
    if runtime
        .with_store(move |store| {
            Ok(store
                .get_document::<FixedRun>(RUN_NAMESPACE, &cancel_id)?
                .is_none_or(|r| r.value.cancel_requested))
        })
        .await
        .unwrap_or(true)
    {
        cancel.store(true, Ordering::Release);
    }
    let mut start = runtime.start_workflow(
        "fixed_candidate_evaluation",
        chrono::Utc::now().to_rfc3339(),
    );
    start.run_id = grant.run_id.clone();
    start.parent_run_id = Some(record.original_run_id.clone());
    start.input_fingerprint = grant.candidate_digest.clone();
    start.coverage = format!(
        "Exact candidate {} fixed spec {}. External review and human quality remain pending.",
        grant.candidate_digest, grant.evaluation_digest
    );
    start.origin = evaluator.source_id.clone();
    runtime
        .begin_workflow(start.clone())
        .await
        .map_err(|e| e.to_string())?;
    let mut checks = Vec::new();
    let mut comparison = Vec::new();
    let mut exact = None;
    let mut drift = false;
    let failure = {
        let execution = async {
            let temp =
                tempfile::tempdir_in(service.source_workspace_root()).map_err(|e| e.to_string())?;
            let root = temp.path().join("source");
            let scratch = temp.path().join("output");
            std::fs::create_dir(&root).map_err(|e| e.to_string())?;
            std::fs::create_dir(&scratch).map_err(|e| e.to_string())?;
            if !fixed.commands.is_empty() {
                let id = record
                    .candidate
                    .worktree_ref
                    .as_deref()
                    .ok_or("native isolated worktree required")?;
                let captured = snapshot(service, id, &record.target_files).await?;
                if captured.identity.patch_digest != record.content_digest
                    || captured.identity.source_digest != record.source_identity
                {
                    return Err(
                        "candidate bytes differ from registered patch/source identity".to_string(),
                    );
                }
                exact = Some(captured.identity.clone());
                for (name, bytes) in captured.files {
                    let path = root.join(name);
                    std::fs::create_dir_all(path.parent().ok_or("invalid snapshot path")?)
                        .map_err(|e| e.to_string())?;
                    std::fs::write(path, bytes).map_err(|e| e.to_string())?;
                }
                for command in &fixed.commands {
                    if cancel.load(Ordering::Acquire) {
                        return Err("evaluation cancelled".into());
                    }
                    let id = grant.run_id.clone();
                    let revoked = runtime
                        .with_store(move |store| {
                            Ok(store
                                .get_document::<FixedRun>(RUN_NAMESPACE, &id)?
                                .is_none_or(|r| r.value.cancel_requested))
                        })
                        .await
                        .unwrap_or(true);
                    if revoked
                        || chrono::DateTime::parse_from_rfc3339(&grant.expires_at)
                            .map_or(true, |expires| expires <= chrono::Utc::now())
                    {
                        cancel.store(true, Ordering::Release);
                        return Err("evaluation grant cancelled or expired".into());
                    }
                    let result = executor
                        .execute(
                            command,
                            &root,
                            &scratch,
                            fixed.output_limit_bytes,
                            cancel.clone(),
                        )
                        .await?;
                    checks.push((command.criterion_id.clone(), result));
                }
                drift = snapshot(service, id, &record.target_files)
                    .await
                    .map(|s| Some(s.identity) != exact)
                    .unwrap_or(true);
            }
            if let Some(offline) = &fixed.offline {
                if record.target != DeploymentTarget::Configuration {
                    return Err("offline profile requires configuration draft candidate".into());
                }
                let record = record.clone();
                let profile = runtime
                    .with_store(move |store| {
                        let id = record
                            .candidate
                            .config_draft_ref
                            .as_deref()
                            .ok_or(invalid("offline draft missing"))?;
                        let draft: Stored<serde_json::Value> = store
                            .get_document("resource_draft", id)?
                            .ok_or(invalid("offline draft unavailable"))?;
                        if digest(&draft.value)? != record.content_digest {
                            return Err(invalid("offline candidate draft drifted"));
                        }
                        serde_json::from_value::<OfflineProfileDraft>(draft.value)
                            .map(|draft| draft.profile)
                            .map_err(OperatorStoreError::from)
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                comparison = compare_profiles(offline, &profile).map_err(|e| e.to_string())?;
                checks.push((
                    offline.criterion_id.clone(),
                    CommandResult {
                        result: if comparison.iter().all(|c| c.candidate_pass) {
                            CheckResult::Passed
                        } else {
                            CheckResult::Failed
                        },
                        status: "offline deterministic profile comparison; not human judgment"
                            .into(),
                        output_digest: digest(&comparison).map_err(|e| e.to_string())?,
                        output_bytes: 0,
                    },
                ));
            }
            Ok::<(), String>(())
        };
        // Do not drop a running native execution at the total deadline. Signal cancellation
        // and await its owning cleanup before deleting scratch files or minting evidence.
        tokio::pin!(execution);
        match tokio::time::timeout(
            Duration::from_secs(u64::from(fixed.total_timeout_seconds)),
            &mut execution,
        )
        .await
        {
            Ok(Ok(())) => None,
            Ok(Err(_)) => Some("fixed evaluation unavailable or candidate drifted"),
            Err(_) => {
                cancel.store(true, Ordering::Release);
                let _ = execution.await;
                Some("fixed evaluation total time limit reached")
            }
        }
    };
    let checked_record = record.clone();
    let checked_spec = spec.clone();
    let binding_current = runtime
        .with_store(move |store| {
            let current =
                improvement::candidate(store, &checked_record.candidate.candidate_id)?.value;
            let current_spec: Stored<EvaluationSpec> = store
                .get_document(SPEC_NAMESPACE, &checked_spec.id)?
                .ok_or(invalid("spec disappeared"))?;
            if digest(&current)? != digest(&checked_record)?
                || digest(&current_spec.value)? != digest(&checked_spec)?
            {
                return Ok(false);
            }
            if let Some(id) = &checked_record.candidate.config_draft_ref {
                let draft: Stored<serde_json::Value> = store
                    .get_document("resource_draft", id)?
                    .ok_or(invalid("draft disappeared"))?;
                if digest(&draft.value)? != checked_record.content_digest {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .await
        .unwrap_or(false);
    drift |= !binding_current;
    let now = chrono::Utc::now().to_rfc3339();
    let cancel_id = grant.run_id.clone();
    if runtime
        .with_store(move |store| {
            Ok(store
                .get_document::<FixedRun>(RUN_NAMESPACE, &cancel_id)?
                .is_none_or(|r| r.value.cancel_requested))
        })
        .await
        .unwrap_or(true)
    {
        cancel.store(true, Ordering::Release);
    }
    let cancelled = cancel.load(Ordering::Acquire);
    let mut proof = EvaluationProof {
        schema_version: 1,
        project: record.project.clone(),
        id: format!("fixed:{}", grant.run_id),
        candidate_digest: grant.candidate_digest.clone(),
        evaluation_digest: grant.evaluation_digest.clone(),
        authority: evaluator.clone(),
        observed_at: now.clone(),
        checks: vec![],
    };
    for criterion in &spec.criteria {
        let found = checks.iter().find(|(id, _)| id == &criterion.id);
        let (result, observed) = if drift || failure.is_some() || cancelled {
            (
                CheckResult::Unavailable,
                failure
                    .unwrap_or("candidate drift or cancellation invalidated result")
                    .into(),
            )
        } else if criterion.external_review {
            (CheckResult::Unavailable, "Authenticated external independent review pending; runner never invokes review tools".into())
        } else if let Some((_, result)) = found {
            (
                result.result.clone(),
                format!(
                    "{}; output bytes {}; digest {}",
                    result.status, result.output_bytes, result.output_digest
                ),
            )
        } else {
            (
                CheckResult::Unavailable,
                "Fixed human quality/other independent gate pending".into(),
            )
        };
        let evidence = EvidenceRef {
            evidence_id: format!("{}:{}", grant.run_id, criterion.id),
            source: evaluator.clone(),
            locator: format!("buddy-evaluation://{}", grant.run_id),
            observed_at: now.clone(),
            digest: Some(
                digest(&(grant, &exact, &criterion.id, &result, &observed))
                    .map_err(|e| e.to_string())?,
            ),
        };
        proof.checks.push(CheckObservation {
            criterion_id: criterion.id.clone(),
            command_or_protocol: criterion.command_or_protocol.clone(),
            result,
            observed_result: observed,
            evidence,
            run_id: grant.run_id.clone(),
        });
    }
    let report = format!("# Fixed candidate evaluation\n\nCandidate `{}`\n\nPatch/config `{}`\n\nFixed specification `{}`\n\n{}\n\nOffline baseline/candidate output pairs retained: {}. Deterministic checks do not certify safety, usefulness, taste or human quality. External review remains pending. No merge, install, deployment or activation performed.", grant.candidate_digest, record.content_digest, grant.evaluation_digest, proof.checks.iter().map(|c| format!("- {}: {:?} — {}", c.criterion_id, c.result, c.observed_result)).collect::<Vec<_>>().join("\n"), comparison.len());
    let terminal = if cancelled {
        WorkflowTerminal::Cancelled {
            failure_class: Some("cancelled_or_limit".into()),
        }
    } else if failure.is_some() || drift {
        WorkflowTerminal::Blocked {
            failure_class: Some("unavailable_or_drift".into()),
        }
    } else {
        WorkflowTerminal::Completed
    };
    runtime
        .publish_report(
            start,
            "Candidate evaluation — independent fixed checks".into(),
            report,
            proof.checks.iter().map(|c| c.evidence.clone()).collect(),
            terminal,
        )
        .await?;
    let proof_copy = proof.clone();
    let id = grant.run_id.clone();
    let candidate_copy = record.clone();
    runtime
        .with_store(move |store| {
            ensure_fixed_evaluation_reservation(store, &id, true)?;
            if digest(
                &improvement::candidate(store, &candidate_copy.candidate.candidate_id)?.value,
            )? != proof_copy.candidate_digest
            {
                return Err(invalid("candidate registry drifted after evaluation"));
            }
            if let Some(action) = &candidate_copy.activation_action {
                improvement::validate_draft_binding(store, &candidate_copy, action)?;
            }
            let current: Stored<FixedRun> = store
                .get_document(RUN_NAMESPACE, &id)?
                .ok_or(invalid("run claim missing"))?;
            let mut run = current.value;
            if run.cancel_requested && !cancelled {
                return Err(invalid("evaluation cancelled before proof commit"));
            }
            run.state = "finished_no_activation".into();
            run.exact = exact;
            run.process_id = None;
            let comparison = EvaluationComparison {
                schema_version: 1,
                project: proof_copy.project.clone(),
                run_id: id.clone(),
                candidate_digest: proof_copy.candidate_digest.clone(),
                evaluation_digest: proof_copy.evaluation_digest.clone(),
                cases: comparison,
            };
            store.put_reserved_document(
                FIXED_EVALUATION_PRODUCER,
                &id,
                COMPARISON_NAMESPACE,
                &id,
                &comparison,
                None,
            )?;
            store.put_reserved_document(
                FIXED_EVALUATION_PRODUCER,
                &id,
                PROOF_NAMESPACE,
                &proof_copy.id,
                &proof_copy,
                None,
            )?;
            store.put_terminal_document(
                FIXED_EVALUATION_PRODUCER,
                &id,
                RUN_NAMESPACE,
                &id,
                &run,
                Some(current.revision),
            )?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(proof)
}

pub fn cancel_run(
    store: &mut OperatorStore,
    id: &str,
) -> Result<Stored<FixedRun>, OperatorStoreError> {
    let current: Stored<FixedRun> = store
        .get_document(RUN_NAMESPACE, id)?
        .ok_or(invalid("evaluation run unavailable"))?;
    if current.value.state == "finished_no_activation" {
        return Ok(current);
    }
    ensure_fixed_evaluation_reservation(store, id, true)?;
    let mut run = current.value;
    run.cancel_requested = true;
    store.put_reserved_document(
        FIXED_EVALUATION_PRODUCER,
        id,
        RUN_NAMESPACE,
        id,
        &run,
        Some(current.revision),
    )
}
