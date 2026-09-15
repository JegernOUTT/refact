use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Trajectory,
    BackgroundAgent,
}

impl RuntimeKind {
    pub const ALL: [RuntimeKind; 2] = [RuntimeKind::Trajectory, RuntimeKind::BackgroundAgent];

    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeKind::Trajectory => "trajectory",
            RuntimeKind::BackgroundAgent => "background_agent",
        }
    }

    pub fn parse(raw: &str) -> Option<RuntimeKind> {
        RuntimeKind::ALL.into_iter().find(|k| k.as_str() == raw)
    }
}

impl fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkKey {
    pub runtime: RuntimeKind,
    pub id: String,
}

impl WorkKey {
    pub fn new(runtime: RuntimeKind, id: impl Into<String>) -> Self {
        WorkKey {
            runtime,
            id: id.into(),
        }
    }

    pub fn trajectory(id: impl Into<String>) -> Self {
        WorkKey::new(RuntimeKind::Trajectory, id)
    }

    pub fn background_agent(id: impl Into<String>) -> Self {
        WorkKey::new(RuntimeKind::BackgroundAgent, id)
    }

    pub fn run(&self, attempt: u64) -> RunKey {
        RunKey {
            work: self.clone(),
            attempt,
        }
    }
}

impl fmt::Display for WorkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.runtime.as_str(), self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RunKey {
    pub work: WorkKey,
    pub attempt: u64,
}

impl RunKey {
    pub fn new(work: WorkKey, attempt: u64) -> Self {
        RunKey { work, attempt }
    }

    pub fn runtime(&self) -> RuntimeKind {
        self.work.runtime
    }
}

impl fmt::Display for RunKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.work, self.attempt)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectKey(pub String);

impl ProjectKey {
    pub fn new(value: impl Into<String>) -> Self {
        ProjectKey(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CheckoutKey(pub String);

impl CheckoutKey {
    pub fn new(value: impl Into<String>) -> Self {
        CheckoutKey(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CheckoutKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkScope {
    pub project: ProjectKey,
    pub root: CheckoutKey,
}

impl WorkScope {
    pub fn new(project: ProjectKey, root: CheckoutKey) -> Self {
        WorkScope { project, root }
    }

    pub fn single(project: impl Into<String>) -> Self {
        let project = project.into();
        WorkScope {
            root: CheckoutKey::new(project.clone()),
            project: ProjectKey::new(project),
        }
    }
}
