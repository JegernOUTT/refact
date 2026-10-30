use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::types::{
    CacheComponentReport, CacheOutcome, CachePreparationReport, CacheReason, CacheTrustBoundary,
    WorktreeMeta, WorktreeRegistry,
};

#[cfg(target_os = "linux")]
#[path = "cache_linux.rs"]
mod platform;

const REPORT_NAME: &str = "refact-cache-report.json";
const OWNER_NAME: &str = ".refact-cache-owner.json";
const STORE: &str = "/var/lib/refact/worktree-cache";
const MAX_FILES: usize = 100_000;
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CONTEXT_BYTES: u64 = 17 * 1024 * 1024;
const MAX_PATH: usize = 1024;
const MAX_DEPTH: usize = 48;

pub struct CacheExecutionContext {
    meta: WorktreeMeta,
    registry: PathBuf,
    head: String,
    #[cfg(target_os = "linux")]
    checkout: platform::Directory,
    #[cfg(target_os = "linux")]
    gitdir: platform::Directory,
    #[cfg(target_os = "linux")]
    controller_pin: Result<String, CacheReason>,
}

pub(crate) struct PreparationCancellation(Arc<AtomicBool>);

impl PreparationCancellation {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub(crate) fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

impl Drop for PreparationCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct Budget {
    started: Instant,
    cancelled: Arc<AtomicBool>,
    files: usize,
    bytes: u64,
    max_bytes: u64,
}

impl Budget {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self::with_byte_limit(cancelled, MAX_BYTES)
    }

    fn with_byte_limit(cancelled: Arc<AtomicBool>, max_bytes: u64) -> Self {
        Self {
            started: Instant::now(),
            cancelled,
            files: 0,
            bytes: 0,
            max_bytes: max_bytes.min(MAX_BYTES),
        }
    }

    fn metadata() -> Self {
        Self::with_byte_limit(Arc::new(AtomicBool::new(false)), MAX_CONTEXT_BYTES)
    }

    fn check(&self) -> Result<(), CacheReason> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(CacheReason::Cancelled);
        }
        if self.started.elapsed() > Duration::from_secs(30) {
            return Err(CacheReason::LimitExceeded);
        }
        Ok(())
    }

    fn read_limit(&self, requested: usize) -> Result<usize, CacheReason> {
        self.check()?;
        let available = self.max_bytes.saturating_sub(self.bytes);
        if available == 0 || self.files >= MAX_FILES {
            return Err(CacheReason::LimitExceeded);
        }
        Ok((requested as u64).min(available) as usize)
    }

    fn charge(&mut self, bytes: u64) -> Result<(), CacheReason> {
        self.check()?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(CacheReason::LimitExceeded)?;
        self.files += 1;
        if self.bytes > self.max_bytes || self.files > MAX_FILES {
            return Err(CacheReason::LimitExceeded);
        }
        Ok(())
    }
}

fn component(
    outcome: CacheOutcome,
    reason: CacheReason,
    key_ref: Option<String>,
) -> CacheComponentReport {
    CacheComponentReport {
        outcome,
        reason,
        key_ref,
    }
}

pub(crate) fn degraded(reason: CacheReason) -> CachePreparationReport {
    CachePreparationReport {
        schema_version: 1,
        cargo_target: component(CacheOutcome::Degraded, reason, None),
        compiler_cache: component(
            CacheOutcome::Degraded,
            CacheReason::UnsafeDaemonBoundary,
            None,
        ),
        dependencies: component(CacheOutcome::Degraded, reason, None),
        trust: CacheTrustBoundary::Unverified,
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_report(report: &CachePreparationReport) -> bool {
    report.schema_version == 1
        && [
            &report.cargo_target,
            &report.compiler_cache,
            &report.dependencies,
        ]
        .iter()
        .all(|entry| entry.key_ref.as_deref().map_or(true, is_digest))
}

impl CacheExecutionContext {
    pub fn checkout_root(&self) -> &Path {
        &self.meta.root
    }

    pub fn cargo_target(&self) -> Result<PathBuf, CacheReason> {
        let mut budget = Budget::metadata();
        self.validate_current_with_budget(&mut budget)?;
        #[cfg(target_os = "linux")]
        {
            platform::validate_local_target(self, &mut budget)?;
            Ok(self.meta.root.join("refact-agent/engine/target"))
        }
        #[cfg(not(target_os = "linux"))]
        Err(CacheReason::UnsupportedPlatform)
    }

    pub fn validate_current(&self) -> Result<(), CacheReason> {
        self.validate_current_with_budget(&mut Budget::metadata())
    }

    fn validate_current_with_budget(&self, budget: &mut Budget) -> Result<(), CacheReason> {
        #[cfg(target_os = "linux")]
        {
            budget.check()?;
            self.checkout.validate_path()?;
            self.gitdir.validate_path()?;
            if let Ok(pin) = &self.controller_pin {
                if platform::controller_pin(budget)? != *pin {
                    return Err(CacheReason::UntrustedController);
                }
            }
            let data = platform::read_absolute(&self.registry, MAX_JSON_BYTES * 2, false, budget)?;
            let registry: WorktreeRegistry =
                serde_json::from_slice(&data).map_err(|_| CacheReason::IdentityChanged)?;
            if registry.source_workspace_root != self.meta.source_workspace_root
                || registry.project_hash
                    != crate::service::project_hash_for_path(&self.meta.source_workspace_root)
                || registry
                    .records
                    .iter()
                    .filter(|record| record.meta.id == self.meta.id)
                    .count()
                    != 1
                || !registry
                    .records
                    .iter()
                    .any(|record| record.meta == self.meta)
            {
                return Err(CacheReason::IdentityChanged);
            }
            let repo = crate::git::discover_repo(&self.meta.root)
                .map_err(|_| CacheReason::IdentityChanged)?;
            let source = crate::git::discover_repo(&self.meta.source_workspace_root)
                .map_err(|_| CacheReason::IdentityChanged)?;
            platform::validate_checkout_backref(self, budget)?;
            if crate::git::canonical_common_dir(&repo).map_err(|_| CacheReason::IdentityChanged)?
                != crate::git::canonical_common_dir(&source)
                    .map_err(|_| CacheReason::IdentityChanged)?
                || crate::git::current_branch(&repo) != self.meta.branch
                || repo
                    .head()
                    .ok()
                    .and_then(|head| head.target())
                    .map(|oid| oid.to_string())
                    .as_deref()
                    != Some(self.head.as_str())
                || dunce::canonicalize(repo.path()).ok().as_deref() != Some(self.gitdir.path())
                || repo
                    .workdir()
                    .and_then(|path| dunce::canonicalize(path).ok())
                    .as_deref()
                    != Some(self.meta.root.as_path())
                || !repo.is_worktree()
            {
                return Err(CacheReason::IdentityChanged);
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = budget;
            Err(CacheReason::UnsupportedPlatform)
        }
    }

    pub(crate) fn from_registered(
        meta: WorktreeMeta,
        registry: PathBuf,
    ) -> Result<Self, CacheReason> {
        #[cfg(target_os = "linux")]
        {
            let mut budget = Budget::metadata();
            let checkout = platform::Directory::absolute(&meta.root, false)?;
            let repo =
                crate::git::discover_repo(&meta.root).map_err(|_| CacheReason::IdentityChanged)?;
            let gitdir_path =
                dunce::canonicalize(repo.path()).map_err(|_| CacheReason::IdentityChanged)?;
            let gitdir = platform::Directory::absolute(&gitdir_path, false)?;
            let head = repo
                .head()
                .ok()
                .and_then(|head| head.target())
                .ok_or(CacheReason::IdentityChanged)?
                .to_string();
            let controller_pin = platform::controller_pin_for_checkout(&checkout, &mut budget);
            let context = Self {
                meta,
                registry,
                head,
                checkout,
                gitdir,
                controller_pin,
            };
            context.validate_current_with_budget(&mut budget)?;
            Ok(context)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (meta, registry);
            Err(CacheReason::UnsupportedPlatform)
        }
    }

    pub(crate) fn prepare(&self, cancelled: Arc<AtomicBool>) -> CachePreparationReport {
        #[cfg(target_os = "linux")]
        {
            let mut budget = Budget::new(cancelled);
            let report = platform::prepare(self, &mut budget).unwrap_or_else(degraded);
            if let Err(reason) = self.validate_current_with_budget(&mut budget) {
                return degraded(match reason {
                    CacheReason::Cancelled | CacheReason::LimitExceeded => reason,
                    _ => CacheReason::IdentityChanged,
                });
            }
            let _ = platform::write_report(self, &report, &mut budget);
            report
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = cancelled;
            degraded(CacheReason::UnsupportedPlatform)
        }
    }
}

pub(crate) fn read_report(meta: &WorktreeMeta) -> Option<CachePreparationReport> {
    #[cfg(target_os = "linux")]
    {
        let repo = crate::git::discover_repo(&meta.root).ok()?;
        if !repo.is_worktree() {
            return None;
        }
        let directory =
            platform::Directory::absolute(&dunce::canonicalize(repo.path()).ok()?, false).ok()?;
        let mut budget = Budget::with_byte_limit(Arc::new(AtomicBool::new(false)), 4097);
        let bytes = match directory.read_optional(REPORT_NAME, 4096, &mut budget) {
            Ok(None) => return None,
            Ok(Some(bytes)) => bytes,
            Err(_) => return Some(degraded(CacheReason::CorruptBlob)),
        };
        match serde_json::from_slice::<CachePreparationReport>(&bytes) {
            Ok(report) if valid_report(&report) => Some(report),
            _ => Some(degraded(CacheReason::CorruptBlob)),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = meta;
        None
    }
}
