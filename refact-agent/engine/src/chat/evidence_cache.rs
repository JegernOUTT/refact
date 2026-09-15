//! Replaying verification instead of re-running it, without letting a replay pass
//! for a fresh execution.
//!
//! Three rules hold this together:
//!
//! * **Only declared-deterministic evidence is cached.** The key carries the
//!   contents of every declared input file (never an mtime), every file found
//!   under a declared input directory including untracked ones, the argv, the
//!   allowlisted environment values, the declared toolchain fingerprint and the
//!   verification policy in force. A spec that declares no inputs is never cached:
//!   a key that omits an input is a wrong answer waiting to happen.
//! * **A replay is visibly a replay.** [`EvidenceRecord::from_cache`] is set, the
//!   original execution timestamp is carried through untouched, and the replay
//!   time is recorded separately. A replayed *failure* reads as a past failure,
//!   never as a command that just ran again.
//! * **The merged tree is evidence nobody has yet.** Post-merge revalidation runs
//!   with [`CacheMode::Revalidate`], which neither reads nor writes the cache.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::chat::evidence::{
    unexecuted_record, EvidenceContext, EvidenceRecord, EvidenceRunner, EvidenceService,
    EvidenceVerdict, VerificationSpec,
};
use crate::chat::verify_cmd::VerifyCommandPolicy;
use crate::coordination::merge_gate::{MergeVerification, VerificationStage};

const ABSENT: &str = "<absent>";
const REFUSED: &str = "<refused>";
const UNREADABLE: &str = "<unreadable>";
const FIELD: char = '\u{1e}';

/// What a spec declares about its own determinism. Nothing is cached unless the
/// spec names the inputs its result depends on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CacheDeclaration {
    /// Files and directories whose contents decide the result.
    pub deterministic_inputs: Vec<String>,
    pub network_dependent: bool,
    pub time_dependent: bool,
    /// Explicit opt-in for verification that reaches the network or reads the
    /// clock. Off by default, because such a result describes a moment, not a tree.
    pub cache_non_deterministic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonCacheable {
    NoDeclaredInputs,
    NetworkDependent,
    TimeDependent,
}

impl NonCacheable {
    pub fn label(&self) -> String {
        match self {
            NonCacheable::NoDeclaredInputs => "no deterministic inputs declared".to_string(),
            NonCacheable::NetworkDependent => {
                "network-dependent without an explicit opt-in".to_string()
            }
            NonCacheable::TimeDependent => "time-dependent without an explicit opt-in".to_string(),
        }
    }
}

/// Whether this contract may ever be answered from cache.
pub fn spec_is_cacheable(spec: &VerificationSpec) -> Result<(), NonCacheable> {
    if spec.cache.deterministic_inputs.is_empty() {
        return Err(NonCacheable::NoDeclaredInputs);
    }
    if !spec.cache.cache_non_deterministic {
        if spec.cache.network_dependent {
            return Err(NonCacheable::NetworkDependent);
        }
        if spec.cache.time_dependent {
            return Err(NonCacheable::TimeDependent);
        }
    }
    Ok(())
}

/// Only an executed pass or an executed failure is a result. `blocked`,
/// `unavailable`, `not_run` and `superseded` say the evidence was never obtained,
/// and storing them would replay a non-answer as an answer.
pub fn verdict_is_cacheable(verdict: EvidenceVerdict) -> bool {
    matches!(verdict, EvidenceVerdict::Passed | EvidenceVerdict::Failed)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EvidenceCacheKey(String);

impl EvidenceCacheKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Every input the key is computed from, kept apart so each one can be mutated on
/// its own and the key proven to move.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CacheKeyInputs {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub file_contents: BTreeMap<String, String>,
    pub toolchain: Vec<String>,
    pub policy: String,
    pub expected_exit_codes: Vec<i32>,
}

impl CacheKeyInputs {
    pub async fn collect(worktree: &Path, spec: &VerificationSpec, policy: &str) -> Self {
        Self {
            argv: spec.argv.clone(),
            cwd: spec
                .cwd
                .as_ref()
                .map(|cwd| cwd.to_string_lossy().replace('\\', "/")),
            env: spec
                .env
                .iter()
                .filter(|(name, _)| spec.env_allowlist.contains(name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            file_contents: input_digests(worktree, &spec.cache.deterministic_inputs).await,
            toolchain: spec.toolchain_inputs.clone(),
            policy: policy.to_string(),
            expected_exit_codes: spec.expected_exit_codes.clone(),
        }
    }

    pub fn key(&self) -> EvidenceCacheKey {
        let mut canonical = String::new();
        canonical.push_str("argv:");
        for argument in &self.argv {
            canonical.push_str(argument);
            canonical.push(FIELD);
        }
        canonical.push_str("\ncwd:");
        canonical.push_str(self.cwd.as_deref().unwrap_or(""));
        canonical.push_str("\nenv:");
        for (name, value) in &self.env {
            canonical.push_str(name);
            canonical.push('=');
            canonical.push_str(value);
            canonical.push(FIELD);
        }
        canonical.push_str("\nfiles:");
        for (path, digest) in &self.file_contents {
            canonical.push_str(path);
            canonical.push('=');
            canonical.push_str(digest);
            canonical.push(FIELD);
        }
        canonical.push_str("\ntoolchain:");
        for input in &self.toolchain {
            canonical.push_str(input);
            canonical.push(FIELD);
        }
        canonical.push_str("\npolicy:");
        canonical.push_str(&self.policy);
        canonical.push_str("\nexit:");
        for code in &self.expected_exit_codes {
            canonical.push_str(&code.to_string());
            canonical.push(FIELD);
        }
        EvidenceCacheKey(digest_bytes(canonical.as_bytes()))
    }
}

fn digest_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn confined_input_path(worktree: &Path, declared: &str) -> Result<PathBuf, String> {
    let relative = Path::new(declared);
    if declared.trim().is_empty() {
        return Err("input path is empty".to_string());
    }
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err("input path must stay within the worktree".to_string());
    }
    Ok(worktree.join(relative))
}

/// Content digests for every declared input. A declared directory contributes every
/// file under it, so an untracked file appearing there moves the key.
pub(crate) async fn input_digests(
    worktree: &Path,
    declared: &[String],
) -> BTreeMap<String, String> {
    let mut digests = BTreeMap::new();
    for entry in declared {
        let Ok(root) = confined_input_path(worktree, entry) else {
            digests.insert(entry.clone(), REFUSED.to_string());
            continue;
        };
        let Ok(metadata) = tokio::fs::metadata(&root).await else {
            digests.insert(entry.clone(), ABSENT.to_string());
            continue;
        };
        if metadata.is_file() {
            digests.insert(entry.clone(), digest_file(&root).await);
            continue;
        }
        let mut stack = vec![root];
        while let Some(directory) = stack.pop() {
            let Ok(mut listing) = tokio::fs::read_dir(&directory).await else {
                continue;
            };
            while let Ok(Some(child)) = listing.next_entry().await {
                let path = child.path();
                match child.file_type().await {
                    Ok(kind) if kind.is_dir() => stack.push(path),
                    Ok(kind) if kind.is_file() => {
                        let relative = path
                            .strip_prefix(worktree)
                            .unwrap_or(path.as_path())
                            .to_string_lossy()
                            .replace('\\', "/");
                        digests.insert(relative, digest_file(&path).await);
                    }
                    _ => {}
                }
            }
        }
    }
    digests
}

async fn digest_file(path: &Path) -> String {
    match tokio::fs::read(path).await {
        Ok(bytes) => digest_bytes(&bytes),
        Err(_) => UNREADABLE.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceCacheLimits {
    pub max_entries: usize,
    pub max_bytes: usize,
    pub ttl_secs: u64,
}

impl EvidenceCacheLimits {
    pub fn from_settings() -> Self {
        let settings = crate::runtime_settings::current();
        Self {
            max_entries: settings.verification_evidence_cache_max_entries,
            max_bytes: settings.verification_evidence_cache_max_bytes,
            ttl_secs: settings.verification_evidence_cache_ttl_secs,
        }
    }
}

#[derive(Debug, Clone)]
struct CacheEntry {
    record: EvidenceRecord,
    stored_at_ms: i64,
    used_seq: u64,
    bytes: usize,
}

/// Bounded store of replayable evidence. Eviction is deterministic: the entry
/// untouched for longest goes first, and use order is a strictly increasing
/// sequence, so there are no ties to break arbitrarily.
#[derive(Debug)]
pub struct EvidenceCache {
    entries: HashMap<String, CacheEntry>,
    limits: EvidenceCacheLimits,
    seq: u64,
    bytes: usize,
}

impl EvidenceCache {
    pub fn new(limits: EvidenceCacheLimits) -> Self {
        Self {
            entries: HashMap::new(),
            limits,
            seq: 0,
            bytes: 0,
        }
    }

    pub fn set_limits(&mut self, limits: EvidenceCacheLimits) {
        self.limits = limits;
        self.enforce_bounds();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// A hit hands back a record marked as replayed, carrying the original
    /// execution timestamps untouched.
    pub fn lookup(&mut self, key: &EvidenceCacheKey, now_ms: i64) -> Option<EvidenceRecord> {
        self.expire(now_ms);
        self.seq += 1;
        let seq = self.seq;
        let entry = self.entries.get_mut(key.as_str())?;
        entry.used_seq = seq;
        let mut replay = entry.record.clone();
        replay.from_cache = true;
        replay.replayed_at = Some(rfc3339(now_ms));
        Some(replay)
    }

    /// Store an executed result. A replay is never re-stored, and a verdict that
    /// is not a result is refused.
    pub fn store(&mut self, key: EvidenceCacheKey, record: &EvidenceRecord, now_ms: i64) -> bool {
        if record.from_cache || !verdict_is_cacheable(record.verdict) {
            return false;
        }
        self.expire(now_ms);
        self.seq += 1;
        let bytes = record_bytes(record);
        if let Some(previous) = self.entries.remove(key.as_str()) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        self.entries.insert(
            key.0,
            CacheEntry {
                record: record.clone(),
                stored_at_ms: now_ms,
                used_seq: self.seq,
                bytes,
            },
        );
        self.bytes = self.bytes.saturating_add(bytes);
        self.enforce_bounds();
        true
    }

    fn expire(&mut self, now_ms: i64) {
        if self.limits.ttl_secs == 0 {
            return;
        }
        let ttl_ms = self.limits.ttl_secs.saturating_mul(1_000) as i64;
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| now_ms.saturating_sub(entry.stored_at_ms) >= ttl_ms)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            self.remove(&key);
        }
    }

    fn enforce_bounds(&mut self) {
        while self.entries.len() > self.limits.max_entries || self.bytes > self.limits.max_bytes {
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used_seq)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.remove(&victim);
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes = self.bytes.saturating_sub(entry.bytes);
        }
    }
}

#[cfg(test)]
impl EvidenceCache {
    pub fn limits(&self) -> EvidenceCacheLimits {
        self.limits
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, key: &EvidenceCacheKey) -> bool {
        self.entries.contains_key(key.as_str())
    }
}

fn record_bytes(record: &EvidenceRecord) -> usize {
    record.command.len()
        + record.output_tail.len()
        + record.spec_id.len()
        + record.reason.as_ref().map(String::len).unwrap_or_default()
}

fn rfc3339(now_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(now_ms)
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Replay when every keyed input matches.
    Replay,
    /// Neither read nor write: the tree under test is not the tree the cache
    /// describes.
    Revalidate,
}

/// The one process-wide evidence cache.
pub fn evidence_cache() -> &'static Mutex<EvidenceCache> {
    static CACHE: LazyLock<Mutex<EvidenceCache>> =
        LazyLock::new(|| Mutex::new(EvidenceCache::new(EvidenceCacheLimits::from_settings())));
    &CACHE
}

/// Execute a contract, or replay the result of an identical earlier execution.
pub async fn execute_cached<R: EvidenceRunner>(
    service: &EvidenceService,
    cache: &Mutex<EvidenceCache>,
    spec: VerificationSpec,
    runner: &mut R,
    mode: CacheMode,
    now_ms: i64,
) -> EvidenceRecord {
    let accepted = match spec.is_accepted() {
        true => spec,
        false => {
            let proposal = spec.clone();
            match service.accept(spec) {
                Ok(accepted) => accepted,
                Err(rejection) => {
                    return unexecuted_record(
                        &proposal,
                        rejection.verdict,
                        rejection.outcome,
                        rejection.reason,
                    )
                }
            }
        }
    };
    let key = match mode {
        CacheMode::Revalidate => None,
        CacheMode::Replay => match spec_is_cacheable(&accepted) {
            Ok(()) => Some(
                CacheKeyInputs::collect(
                    service.worktree(),
                    &accepted,
                    &service.policy_fingerprint(),
                )
                .await
                .key(),
            ),
            Err(reason) => {
                tracing::debug!(
                    spec_id = %accepted.spec_id,
                    reason = %reason.label(),
                    "verification evidence is not cacheable"
                );
                None
            }
        },
    };
    if let Some(key) = key.as_ref() {
        if let Some(replay) = cache.lock().await.lookup(key, now_ms) {
            return replay;
        }
    }
    let record = service.execute(&accepted, runner).await;
    if let Some(key) = key {
        cache.lock().await.store(key, &record, now_ms);
    }
    record
}

pub async fn execute_cached_all<R: EvidenceRunner>(
    service: &EvidenceService,
    cache: &Mutex<EvidenceCache>,
    specs: Vec<VerificationSpec>,
    runner: &mut R,
    mode: CacheMode,
) -> Vec<EvidenceRecord> {
    let mut records = Vec::with_capacity(specs.len());
    for spec in specs {
        records.push(execute_cached(service, cache, spec, runner, mode, now_ms()).await);
    }
    records
}

/// Fresh evidence about the merged checkout, for [`merge_under_lease`]'s
/// `verify_merged_tree` seam. A branch proved something about itself in isolation;
/// this proves something about the tree that actually exists now, so nothing here
/// may come from cache.
///
/// [`merge_under_lease`]: crate::coordination::merge_gate::merge_under_lease
pub async fn revalidate_merged_tree<R: EvidenceRunner>(
    checkout: PathBuf,
    specs: Vec<VerificationSpec>,
    policy: VerifyCommandPolicy,
    scope: String,
    cache: &Mutex<EvidenceCache>,
    runner: &mut R,
) -> Option<MergeVerification> {
    if specs.is_empty() {
        return None;
    }
    let service = EvidenceService::new(
        policy,
        EvidenceContext {
            commit: crate::chat::verifier_diff::git_head_commit(&checkout).await,
            worktree: checkout,
            scope,
        },
    );
    let records = execute_cached_all(&service, cache, specs, runner, CacheMode::Revalidate).await;
    let decisive = records
        .iter()
        .find(|record| !record.verdict.is_passed())
        .or_else(|| records.last());
    let verification = match decisive {
        Some(record) => MergeVerification {
            stage: VerificationStage::PostMerge,
            command: record.command.clone(),
            passed: record.verdict.is_passed(),
            exit_code: record.exit_code,
            output_tail: record.output_tail.clone(),
        },
        None => MergeVerification {
            stage: VerificationStage::PostMerge,
            command: String::new(),
            passed: false,
            exit_code: None,
            output_tail: "no verification contract ran against the merged tree".to_string(),
        },
    };
    Some(verification)
}

#[cfg(test)]
pub(crate) fn test_cache() -> Mutex<EvidenceCache> {
    Mutex::new(EvidenceCache::new(EvidenceCacheLimits {
        max_entries: 64,
        max_bytes: 1_048_576,
        ttl_secs: 3_600,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::evidence::{
        evaluate_gates, EvidenceContext, EvidenceDefaults, SpecAuthority, VerificationSpec,
    };
    use crate::tasks::types::{VerificationOutcome, VerificationResult};
    use async_trait::async_trait;
    use std::sync::Arc;

    #[derive(Default)]
    struct CountingRunner {
        runs: usize,
        exit_code: i32,
    }

    #[async_trait]
    impl EvidenceRunner for CountingRunner {
        async fn run(&mut self, _worktree: &Path, spec: &VerificationSpec) -> VerificationResult {
            self.runs += 1;
            VerificationResult {
                command: spec.command.clone(),
                exit_code: Some(self.exit_code),
                passed: self.exit_code == 0,
                output_tail: format!("run {}", self.runs),
                outcome: match self.exit_code {
                    0 => VerificationOutcome::Passed,
                    _ => VerificationOutcome::CommandFailed,
                },
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

    fn spec(command: &str) -> VerificationSpec {
        let mut spec =
            VerificationSpec::propose("S-1", command, SpecAuthority::CardInstructions, &defaults());
        spec.cache.deterministic_inputs = vec!["src".to_string()];
        spec
    }

    async fn accepted(service: &EvidenceService, spec: VerificationSpec) -> VerificationSpec {
        service.accept(spec).unwrap()
    }

    async fn key_of(service: &EvidenceService, spec: &VerificationSpec) -> EvidenceCacheKey {
        CacheKeyInputs::collect(service.worktree(), spec, &service.policy_fingerprint())
            .await
            .key()
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

    #[tokio::test]
    async fn every_keyed_input_moves_the_key_when_it_changes() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let baseline_spec = accepted(&service, spec("RUST_BACKTRACE=1 cargo test --lib")).await;
        let baseline = key_of(&service, &baseline_spec).await;

        tokio::fs::write(
            temp.path().join("src").join("lib.rs"),
            "fn main() { todo!() }",
        )
        .await
        .unwrap();
        let changed_file = key_of(&service, &baseline_spec).await;
        assert_ne!(baseline, changed_file, "file content is a keyed input");
        tokio::fs::write(temp.path().join("src").join("lib.rs"), "fn main() {}")
            .await
            .unwrap();
        assert_eq!(
            baseline,
            key_of(&service, &baseline_spec).await,
            "restoring the content restores the key"
        );

        tokio::fs::write(temp.path().join("src").join("scratch.rs"), "// untracked")
            .await
            .unwrap();
        let untracked = key_of(&service, &baseline_spec).await;
        assert_ne!(
            baseline, untracked,
            "an untracked input under a declared directory is a keyed input"
        );
        tokio::fs::remove_file(temp.path().join("src").join("scratch.rs"))
            .await
            .unwrap();

        let changed_env = accepted(&service, spec("RUST_BACKTRACE=full cargo test --lib")).await;
        assert_ne!(
            baseline,
            key_of(&service, &changed_env).await,
            "an environment value is a keyed input"
        );

        let changed_argv = accepted(&service, spec("RUST_BACKTRACE=1 cargo test --doc")).await;
        assert_ne!(
            baseline,
            key_of(&service, &changed_argv).await,
            "argv is a keyed input"
        );

        let mut changed_toolchain = baseline_spec.clone();
        changed_toolchain.toolchain_inputs = vec!["cargo 1.80.0".to_string()];
        assert_ne!(
            baseline,
            key_of(&service, &changed_toolchain).await,
            "the toolchain fingerprint is a keyed input"
        );

        let other_policy = EvidenceService::new(
            VerifyCommandPolicy::from_shell_policy(
                &crate::tools::shell_gate::ShellGatePolicy::default(),
            ),
            EvidenceContext {
                worktree: temp.path().to_path_buf(),
                commit: Some("abc1234".to_string()),
                scope: "T-1".to_string(),
            },
        );
        assert_ne!(
            baseline,
            key_of(&other_policy, &baseline_spec).await,
            "the verification policy in force is a keyed input"
        );
    }

    #[tokio::test]
    async fn each_keyed_input_change_is_a_miss_that_runs_the_command_again() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let cache = test_cache();
        let mut runner = CountingRunner::default();

        let first = execute_cached(
            &service,
            &cache,
            spec("RUST_BACKTRACE=1 cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        assert!(!first.from_cache);
        assert_eq!(runner.runs, 1);

        let mutations: Vec<(&str, Box<dyn Fn(&mut VerificationSpec)>)> = vec![
            (
                "argv",
                Box::new(|spec: &mut VerificationSpec| {
                    spec.command = "RUST_BACKTRACE=1 cargo test --doc".to_string()
                }),
            ),
            (
                "env",
                Box::new(|spec: &mut VerificationSpec| {
                    spec.command = "RUST_BACKTRACE=full cargo test --lib".to_string()
                }),
            ),
            (
                "toolchain",
                Box::new(|spec: &mut VerificationSpec| {
                    spec.toolchain_inputs = vec!["cargo 1.80.0".to_string()]
                }),
            ),
        ];
        let mut expected_runs = 1;
        for (label, mutate) in mutations {
            let mut mutated = spec("RUST_BACKTRACE=1 cargo test --lib");
            mutate(&mut mutated);
            let record = execute_cached(
                &service,
                &cache,
                mutated,
                &mut runner,
                CacheMode::Replay,
                1_000,
            )
            .await;
            expected_runs += 1;
            assert!(!record.from_cache, "{label} must miss");
            assert_eq!(runner.runs, expected_runs, "{label} must run the command");
        }

        tokio::fs::write(temp.path().join("src").join("lib.rs"), "fn main() { 1; }")
            .await
            .unwrap();
        let after_edit = execute_cached(
            &service,
            &cache,
            spec("RUST_BACKTRACE=1 cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        assert!(!after_edit.from_cache, "a file edit must miss");
        assert_eq!(runner.runs, expected_runs + 1);

        let other_policy = EvidenceService::new(
            VerifyCommandPolicy::from_shell_policy(
                &crate::tools::shell_gate::ShellGatePolicy::default(),
            ),
            EvidenceContext {
                worktree: temp.path().to_path_buf(),
                commit: Some("abc1234".to_string()),
                scope: "T-1".to_string(),
            },
        );
        let after_policy = execute_cached(
            &other_policy,
            &cache,
            spec("RUST_BACKTRACE=1 cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        assert!(!after_policy.from_cache, "a policy change must miss");
        assert_eq!(runner.runs, expected_runs + 2);
    }

    #[tokio::test]
    async fn identical_inputs_replay_the_original_execution() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let cache = test_cache();
        let mut runner = CountingRunner::default();

        let executed = execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        let replayed = execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            9_000,
        )
        .await;

        assert_eq!(runner.runs, 1, "the second call must not run the command");
        assert!(!executed.from_cache);
        assert!(executed.replayed_at.is_none());
        assert!(replayed.from_cache, "a hit must be marked as cached");
        assert_eq!(
            replayed.started_at, executed.started_at,
            "a replay carries the original execution timestamp"
        );
        assert_eq!(replayed.finished_at, executed.finished_at);
        assert_eq!(
            replayed.replayed_at.as_deref(),
            Some(rfc3339(9_000).as_str()),
            "the replay time is recorded apart from the execution time"
        );
        assert_eq!(replayed.verdict, executed.verdict);
    }

    #[tokio::test]
    async fn from_cache_reaches_the_consumer() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let cache = test_cache();
        let mut runner = CountingRunner::default();

        let executed = execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        let replayed = execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            2_000,
        )
        .await;

        let fresh_result = executed.to_verification_result();
        let replayed_result = replayed.to_verification_result();
        assert!(!fresh_result.from_cache);
        assert!(
            replayed_result.from_cache,
            "the consumer-facing result must say it was replayed"
        );
        assert_eq!(
            replayed_result.executed_at.as_deref(),
            Some(executed.started_at.as_str()),
            "the consumer sees when the command actually ran"
        );
        let json = serde_json::to_value(&replayed_result).unwrap();
        assert_eq!(json["from_cache"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn a_replayed_failure_is_not_reported_as_a_fresh_run() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let cache = test_cache();
        let mut runner = CountingRunner {
            exit_code: 1,
            ..CountingRunner::default()
        };

        let failed = execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        let replayed = execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            5_000,
        )
        .await;

        assert_eq!(failed.verdict, EvidenceVerdict::Failed);
        assert_eq!(runner.runs, 1, "a cached failure must not re-run");
        assert!(replayed.from_cache);
        let gates = evaluate_gates(&[spec("cargo test --lib")], &[replayed.clone()]);
        assert!(!gates.completion_allowed());
        assert!(gates.blocks[0].from_cache);
        assert!(
            gates.blocks[0].reason.contains("replayed from cache"),
            "a replayed failure must not read as a command that just ran: {}",
            gates.blocks[0].reason
        );
        assert!(gates.blocks[0].reason.contains(&failed.started_at));
    }

    #[tokio::test]
    async fn network_and_time_dependent_verification_is_not_cached_unless_opted_in() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let cache = test_cache();
        let mut runner = CountingRunner::default();

        let mut networked = spec("cargo test --lib");
        networked.cache.network_dependent = true;
        assert_eq!(
            spec_is_cacheable(&networked).unwrap_err(),
            NonCacheable::NetworkDependent
        );
        for _ in 0..2 {
            let record = execute_cached(
                &service,
                &cache,
                networked.clone(),
                &mut runner,
                CacheMode::Replay,
                1_000,
            )
            .await;
            assert!(!record.from_cache);
        }
        assert_eq!(runner.runs, 2, "network-dependent evidence is re-run");
        assert!(cache.lock().await.is_empty());

        let mut timed = spec("cargo test --lib");
        timed.cache.time_dependent = true;
        assert_eq!(
            spec_is_cacheable(&timed).unwrap_err(),
            NonCacheable::TimeDependent
        );

        let mut opted_in = networked.clone();
        opted_in.cache.cache_non_deterministic = true;
        assert!(spec_is_cacheable(&opted_in).is_ok());
        execute_cached(
            &service,
            &cache,
            opted_in.clone(),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        let replayed = execute_cached(
            &service,
            &cache,
            opted_in,
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        assert_eq!(runner.runs, 3, "an explicit opt-in caches");
        assert!(replayed.from_cache);
    }

    #[tokio::test]
    async fn a_spec_that_declares_no_inputs_is_never_cached() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let cache = test_cache();
        let mut runner = CountingRunner::default();
        let mut undeclared = spec("cargo test --lib");
        undeclared.cache.deterministic_inputs.clear();

        for _ in 0..2 {
            let record = execute_cached(
                &service,
                &cache,
                undeclared.clone(),
                &mut runner,
                CacheMode::Replay,
                1_000,
            )
            .await;
            assert!(!record.from_cache);
        }

        assert_eq!(
            spec_is_cacheable(&undeclared).unwrap_err(),
            NonCacheable::NoDeclaredInputs
        );
        assert_eq!(runner.runs, 2);
        assert!(cache.lock().await.is_empty());
    }

    #[tokio::test]
    async fn blocked_unavailable_and_not_run_are_never_stored_as_results() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let mut cache = EvidenceCache::new(EvidenceCacheLimits {
            max_entries: 8,
            max_bytes: 1_048_576,
            ttl_secs: 3_600,
        });
        let accepted = accepted(&service, spec("cargo test --lib")).await;
        let key = key_of(&service, &accepted).await;

        for verdict in [
            EvidenceVerdict::Blocked,
            EvidenceVerdict::Unavailable,
            EvidenceVerdict::NotRun,
            EvidenceVerdict::Superseded,
        ] {
            let record = unexecuted_record(
                &accepted,
                verdict,
                VerificationOutcome::Rejected,
                "not a result".to_string(),
            );
            assert!(!verdict_is_cacheable(verdict));
            assert!(!cache.store(key.clone(), &record, 1_000));
            assert!(cache.is_empty(), "{verdict:?} must not be stored");
        }

        let mut passed = unexecuted_record(
            &accepted,
            EvidenceVerdict::Passed,
            VerificationOutcome::Passed,
            "result".to_string(),
        );
        assert!(cache.store(key.clone(), &passed, 1_000));
        passed.from_cache = true;
        assert!(
            !cache.store(key, &passed, 1_000),
            "a replay must never be stored back as an execution"
        );
    }

    #[tokio::test]
    async fn entries_expire_after_the_configured_ttl() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let cache = Mutex::new(EvidenceCache::new(EvidenceCacheLimits {
            max_entries: 8,
            max_bytes: 1_048_576,
            ttl_secs: 10,
        }));
        let mut runner = CountingRunner::default();

        execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        let within_ttl = execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            10_999,
        )
        .await;
        assert!(within_ttl.from_cache, "a live entry replays");
        assert_eq!(runner.runs, 1);

        let after_ttl = execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            11_001,
        )
        .await;
        assert!(!after_ttl.from_cache, "an expired entry must not replay");
        assert_eq!(runner.runs, 2);
        assert_eq!(cache.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn eviction_drops_the_least_recently_used_entry_deterministically() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let mut cache = EvidenceCache::new(EvidenceCacheLimits {
            max_entries: 2,
            max_bytes: 1_048_576,
            ttl_secs: 3_600,
        });
        let mut keys = Vec::new();
        for index in 0..3 {
            let accepted =
                accepted(&service, spec(&format!("cargo test --lib --test t{index}"))).await;
            keys.push(key_of(&service, &accepted).await);
        }
        let record = |index: usize| EvidenceRecord {
            spec_id: format!("S-{index}"),
            ..unexecuted_record(
                &VerificationSpec::propose(
                    "S-1",
                    "cargo test",
                    SpecAuthority::CardInstructions,
                    &defaults(),
                ),
                EvidenceVerdict::Passed,
                VerificationOutcome::Passed,
                "result".to_string(),
            )
        };

        cache.store(keys[0].clone(), &record(0), 1_000);
        cache.store(keys[1].clone(), &record(1), 1_000);
        cache.lookup(&keys[0], 1_000).unwrap();
        cache.store(keys[2].clone(), &record(2), 1_000);

        assert_eq!(cache.len(), 2, "the entry cap holds");
        assert!(cache.contains(&keys[0]), "the most recently used survives");
        assert!(!cache.contains(&keys[1]), "the least recently used goes");
        assert!(cache.contains(&keys[2]));

        cache.set_limits(EvidenceCacheLimits {
            max_entries: 2,
            max_bytes: 1,
            ttl_secs: 3_600,
        });
        assert!(
            cache.is_empty(),
            "a byte cap below one entry empties the cache"
        );
        assert_eq!(cache.bytes(), 0);
    }

    #[tokio::test]
    async fn revalidation_never_reads_or_writes_the_cache() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let cache = test_cache();
        let mut runner = CountingRunner::default();

        execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;
        assert_eq!(cache.lock().await.len(), 1);

        let revalidated = execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Revalidate,
            2_000,
        )
        .await;

        assert!(
            !revalidated.from_cache,
            "revalidation is always a fresh run"
        );
        assert_eq!(runner.runs, 2);
        assert_eq!(
            cache.lock().await.len(),
            1,
            "revalidation must not write the cache either"
        );
    }

    #[tokio::test]
    async fn post_merge_revalidation_runs_fresh_evidence_against_the_merged_tree() {
        let temp = worktree_with_source().await;
        let merged = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(merged.path().join("src"))
            .await
            .unwrap();
        tokio::fs::write(merged.path().join("src").join("lib.rs"), "fn main() {}")
            .await
            .unwrap();
        let cache = test_cache();
        let mut green = CountingRunner::default();

        execute_cached(
            &service(temp.path()),
            &cache,
            spec("cargo test --lib"),
            &mut green,
            CacheMode::Replay,
            1_000,
        )
        .await;
        assert_eq!(
            cache.lock().await.len(),
            1,
            "the branch proved itself green"
        );

        let mut red = CountingRunner {
            exit_code: 1,
            ..CountingRunner::default()
        };
        let verification = revalidate_merged_tree(
            merged.path().to_path_buf(),
            vec![spec("cargo test --lib")],
            VerifyCommandPolicy::permissive(),
            "T-1".to_string(),
            &cache,
            &mut red,
        )
        .await
        .expect("the merged tree is verified");

        assert_eq!(red.runs, 1, "the merged tree gets its own execution");
        assert_eq!(verification.stage, VerificationStage::PostMerge);
        assert!(
            !verification.passed,
            "a green branch cache entry must not decide the merged tree"
        );
        assert_eq!(verification.exit_code, Some(1));
    }

    #[tokio::test]
    async fn the_shared_cache_is_one_instance() {
        let first = evidence_cache() as *const Mutex<EvidenceCache>;
        let second = evidence_cache() as *const Mutex<EvidenceCache>;
        assert_eq!(first, second);
        let limits = evidence_cache().lock().await.limits();
        assert!(limits.max_entries > 0);
        assert!(limits.max_bytes > 0);
    }

    #[tokio::test]
    async fn a_refused_input_path_is_keyed_as_refused_and_never_read() {
        let temp = worktree_with_source().await;
        let service = service(temp.path());
        let mut escaping = spec("cargo test --lib");
        escaping.cache.deterministic_inputs = vec!["../outside".to_string()];
        let escaping = accepted(&service, escaping).await;

        let inputs =
            CacheKeyInputs::collect(service.worktree(), &escaping, &service.policy_fingerprint())
                .await;

        assert_eq!(
            inputs.file_contents.get("../outside").map(String::as_str),
            Some(REFUSED)
        );
    }

    #[tokio::test]
    async fn concurrent_lookups_share_one_cache() {
        let temp = worktree_with_source().await;
        let cache = Arc::new(test_cache());
        let service = Arc::new(service(temp.path()));
        let mut runner = CountingRunner::default();

        execute_cached(
            &service,
            &cache,
            spec("cargo test --lib"),
            &mut runner,
            CacheMode::Replay,
            1_000,
        )
        .await;

        let mut handles = Vec::new();
        for _ in 0..4 {
            let cache = cache.clone();
            let service = service.clone();
            handles.push(tokio::spawn(async move {
                let mut runner = CountingRunner::default();
                let record = execute_cached(
                    &service,
                    &cache,
                    spec("cargo test --lib"),
                    &mut runner,
                    CacheMode::Replay,
                    2_000,
                )
                .await;
                (record.from_cache, runner.runs)
            }));
        }
        for handle in handles {
            let (from_cache, runs) = handle.await.unwrap();
            assert!(from_cache);
            assert_eq!(runs, 0);
        }
    }
}
