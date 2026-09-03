#[derive(Debug, Clone)]
pub struct GitIntelLimits {
    pub max_commits: usize,
    pub deep_walk_limit: usize,
    pub max_files_per_commit_for_cochange: usize,
    pub max_files_per_commit_for_entropy: usize,
}

impl Default for GitIntelLimits {
    fn default() -> Self {
        Self {
            max_commits: 5_000,
            deep_walk_limit: 50_000,
            max_files_per_commit_for_cochange: 1_000,
            max_files_per_commit_for_entropy: 200,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GitIntelConfig {
    pub limits: GitIntelLimits,
}

impl GitIntelConfig {
    pub fn new() -> Self {
        Self::default()
    }
}

pub static GIT_INTEL_CONFIG: std::sync::LazyLock<std::sync::RwLock<GitIntelConfig>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(GitIntelConfig::new()));

pub fn limits() -> GitIntelLimits {
    GIT_INTEL_CONFIG
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .limits
        .clone()
}

pub fn install_git_intel_limits(limits: GitIntelLimits) {
    GIT_INTEL_CONFIG
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .limits = limits;
}
