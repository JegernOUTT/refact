use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct PrivacyPolicy {
    pub blocked: Vec<String>,
    pub zones: Vec<Zone>,
    pub subagents: SubagentPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Zone {
    pub name: String,
    pub patterns: Vec<String>,
    pub send_to: Vec<String>,
    pub on_shell_read: ShellBehavior,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShellBehavior {
    #[default]
    Withhold,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SubagentPolicy {
    pub report_declassifies: bool,
}

impl Default for SubagentPolicy {
    fn default() -> Self {
        Self {
            report_declassifies: true,
        }
    }
}
