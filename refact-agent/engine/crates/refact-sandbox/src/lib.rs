use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

mod bwrap;
mod landlock;
mod probe;

pub use bwrap::BwrapProvider;
pub use landlock::{run_sandbox_exec_from_env, LandlockProvider};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    Full,
    Partial,
    Unusable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecSandboxSpec {
    pub mode: SandboxMode,
    pub ro_paths: Vec<PathBuf>,
    pub rw_paths: Vec<PathBuf>,
    pub allow_network: bool,
}

impl ExecSandboxSpec {
    pub fn normalized(mut self, cwd: &Path) -> Self {
        if self.ro_paths.is_empty() {
            self.ro_paths.push(PathBuf::from("/"));
        }
        match self.mode {
            SandboxMode::ReadOnly => self.rw_paths.clear(),
            SandboxMode::WorkspaceWrite => {
                push_unique(&mut self.rw_paths, cwd.to_path_buf());
                push_unique(&mut self.rw_paths, std::env::temp_dir());
            }
            SandboxMode::FullAccess => {
                self.rw_paths.clear();
                self.rw_paths.push(PathBuf::from("/"));
            }
        }
        self
    }
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxError {
    pub provider: String,
    pub reason: String,
}

impl SandboxError {
    pub fn new(provider: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for SandboxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "sandbox: {}: {}", self.provider, self.reason)
    }
}

impl std::error::Error for SandboxError {}

pub trait SandboxProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn probe(&self) -> Enforcement;
    fn confine(
        &self,
        spec: &ExecSandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<(String, Vec<String>), SandboxError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxStatus {
    pub provider: &'static str,
    pub enforcement: Enforcement,
}

#[derive(Clone, Copy)]
enum ProviderKind {
    Bwrap,
    Landlock,
    Noop,
}

struct NoopProvider;

impl SandboxProvider for NoopProvider {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn probe(&self) -> Enforcement {
        Enforcement::Unusable
    }

    fn confine(
        &self,
        _spec: &ExecSandboxSpec,
        _program: &str,
        _args: &[String],
    ) -> Result<(String, Vec<String>), SandboxError> {
        Err(SandboxError::new(
            self.name(),
            "no usable sandbox provider is available",
        ))
    }
}

static SELECTED_PROVIDER: OnceLock<(ProviderKind, Enforcement)> = OnceLock::new();

pub fn select_provider() -> (Box<dyn SandboxProvider>, Enforcement) {
    let (kind, enforcement) = *SELECTED_PROVIDER.get_or_init(select_provider_uncached);
    (provider_for_kind(kind), enforcement)
}

pub fn sandbox_status() -> SandboxStatus {
    let (provider, enforcement) = select_provider();
    SandboxStatus {
        provider: provider.name(),
        enforcement,
    }
}

fn select_provider_uncached() -> (ProviderKind, Enforcement) {
    #[cfg(target_os = "linux")]
    {
        let bwrap_enforcement = BwrapProvider::default().probe();
        if bwrap_enforcement == Enforcement::Full {
            return (ProviderKind::Bwrap, bwrap_enforcement);
        }
        return select_from_probes(bwrap_enforcement, LandlockProvider::default().probe());
    }
    #[cfg(not(target_os = "linux"))]
    {
        (ProviderKind::Noop, Enforcement::Unusable)
    }
}

fn select_from_probes(
    bwrap_enforcement: Enforcement,
    landlock_enforcement: Enforcement,
) -> (ProviderKind, Enforcement) {
    if bwrap_enforcement == Enforcement::Full {
        return (ProviderKind::Bwrap, bwrap_enforcement);
    }
    if matches!(
        landlock_enforcement,
        Enforcement::Full | Enforcement::Partial
    ) {
        return (ProviderKind::Landlock, landlock_enforcement);
    }
    (ProviderKind::Noop, Enforcement::Unusable)
}

fn provider_for_kind(kind: ProviderKind) -> Box<dyn SandboxProvider> {
    match kind {
        ProviderKind::Bwrap => Box::new(BwrapProvider::default()),
        ProviderKind::Landlock => Box::new(LandlockProvider::default()),
        ProviderKind::Noop => Box::new(NoopProvider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_selection_prefers_full_bwrap_then_landlock() {
        assert!(matches!(
            select_from_probes(Enforcement::Full, Enforcement::Partial),
            (ProviderKind::Bwrap, Enforcement::Full)
        ));
        assert!(matches!(
            select_from_probes(Enforcement::Unusable, Enforcement::Partial),
            (ProviderKind::Landlock, Enforcement::Partial)
        ));
        assert!(matches!(
            select_from_probes(Enforcement::Unusable, Enforcement::Unusable),
            (ProviderKind::Noop, Enforcement::Unusable)
        ));
    }

    #[test]
    fn workspace_write_normalization_adds_required_paths() {
        let cwd = PathBuf::from("/workspace");
        let spec = ExecSandboxSpec {
            mode: SandboxMode::WorkspaceWrite,
            ro_paths: Vec::new(),
            rw_paths: Vec::new(),
            allow_network: false,
        }
        .normalized(&cwd);

        assert_eq!(spec.ro_paths, vec![PathBuf::from("/")]);
        assert!(spec.rw_paths.contains(&cwd));
        assert!(spec.rw_paths.contains(&std::env::temp_dir()));
    }
}
