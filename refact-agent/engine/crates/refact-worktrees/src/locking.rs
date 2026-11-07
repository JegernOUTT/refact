use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::OwnedMutexGuard;

use crate::git;

const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_POLL: Duration = Duration::from_millis(25);

pub struct PhysicalGuard {
    _file: fd_lock::RwLock<File>,
}

impl std::fmt::Debug for PhysicalGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicalGuard").finish_non_exhaustive()
    }
}

impl PhysicalGuard {
    pub async fn acquire(path: &Path) -> Result<Self, String> {
        let parent = path.parent().ok_or("Lock directory missing")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("Cannot open cooperating-local-process lock: {e}"))?;
        let mut lock = fd_lock::RwLock::new(file);
        let deadline = tokio::time::Instant::now() + LOCK_TIMEOUT;
        #[cfg(test)]
        let mut contention_reported = false;
        loop {
            match lock.try_write().map(std::mem::forget) {
                Ok(()) => {
                    return Ok(Self {
                        _file: lock,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    #[cfg(test)]
                    if !contention_reported && std::env::var_os("REFACT_REGISTRY_CHILD").is_some() {
                        use std::io::Write;
                        println!("registry-lock-contended");
                        std::io::stdout().flush().unwrap();
                        contention_reported = true;
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "Cooperating-local-process locking unsupported: {e}"
                    ))
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "Timed out waiting for cooperating-local-process lock {}",
                    path.display()
                ));
            }
            tokio::time::sleep_until((tokio::time::Instant::now() + LOCK_POLL).min(deadline)).await;
        }
    }
}

pub type PhaseValidator = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub struct MergeAuthority(pub PhaseValidator);

impl std::fmt::Debug for MergeAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergeAuthority").finish_non_exhaustive()
    }
}

thread_local! {
    static GIT_PHASE: std::cell::RefCell<Option<MergeAuthority>> = const { std::cell::RefCell::new(None) };
}

pub fn with_git_phase<T>(authority: Option<MergeAuthority>, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<MergeAuthority>);
    impl Drop for Restore {
        fn drop(&mut self) {
            GIT_PHASE.with(|phase| *phase.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(GIT_PHASE.with(|phase| phase.replace(authority)));
    operation()
}

pub fn checkpoint_git_phase() -> Result<(), String> {
    let authority = GIT_PHASE.with(|phase| phase.borrow().clone());
    if let Some(authority) = authority {
        with_git_phase(None, || (authority.0)())
            .map_err(|e| format!("Git phase authority lost; state retained, do not replay: {e}"))?;
    }
    Ok(())
}

pub struct MergePermit {
    repository: PathBuf,
    _process: OwnedMutexGuard<()>,
    _physical: PhysicalGuard,
}

impl std::fmt::Debug for MergePermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergePermit")
            .field("repository", &self.repository)
            .finish()
    }
}

pub fn repository_identity(checkout: &Path) -> Result<PathBuf, String> {
    let common = git::run_git(checkout, &["rev-parse", "--git-common-dir"])?;
    dunce::canonicalize(checkout.join(common.trim())).map_err(|e| e.to_string())
}

impl MergePermit {
    pub async fn acquire(checkout: &Path) -> Result<Arc<Self>, String> {
        let repository = repository_identity(checkout)?;
        let process = crate::service::merge_lock_for(checkout);
        let process = tokio::time::timeout(LOCK_TIMEOUT, process.lock_owned())
            .await
            .map_err(|_| "Timed out waiting for repository merge gate".to_string())?;
        let physical =
            PhysicalGuard::acquire(&repository.join("refact-managed-merge.lock")).await?;
        Ok(Arc::new(Self {
            repository,
            _process: process,
            _physical: physical,
        }))
    }

    pub fn validate(&self, checkout: &Path) -> Result<(), String> {
        if repository_identity(checkout)? != self.repository {
            return Err("Managed merge repository identity changed; no writes authorized".into());
        }
        Ok(())
    }
}
