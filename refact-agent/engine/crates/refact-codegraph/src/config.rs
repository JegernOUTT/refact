#[derive(Debug, Clone)]
pub struct CodegraphLimits {
    pub dead_code_max_results: usize,
    pub exec_flow_max_nodes: usize,
}

impl Default for CodegraphLimits {
    fn default() -> Self {
        Self {
            dead_code_max_results: 5_000,
            exec_flow_max_nodes: 5_000,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CodegraphConfig {
    pub limits: CodegraphLimits,
}

impl CodegraphConfig {
    pub fn new() -> Self {
        Self::default()
    }
}

pub static CODEGRAPH_CONFIG: std::sync::LazyLock<std::sync::RwLock<CodegraphConfig>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(CodegraphConfig::new()));

pub fn limits() -> CodegraphLimits {
    CODEGRAPH_CONFIG
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .limits
        .clone()
}

pub fn install_codegraph_limits(limits: CodegraphLimits) {
    CODEGRAPH_CONFIG
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .limits = limits;
}
