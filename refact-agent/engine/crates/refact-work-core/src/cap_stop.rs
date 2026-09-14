use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CapKind {
    #[serde(rename = "max_steps")]
    Steps,
    #[serde(rename = "goal.max_turns")]
    GoalTurns,
    #[serde(rename = "goal.max_tokens")]
    GoalTokens,
    #[serde(rename = "goal.max_minutes")]
    GoalMinutes,
    #[serde(rename = "goal.max_cost_cents")]
    GoalCostCents,
    #[serde(rename = "goal.no_progress_turns")]
    GoalNoProgress,
    #[serde(rename = "context_tokens")]
    ContextTokens,
    #[serde(rename = "max_spawns")]
    Spawns,
}

impl CapKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steps => "max_steps",
            Self::GoalTurns => "goal.max_turns",
            Self::GoalTokens => "goal.max_tokens",
            Self::GoalMinutes => "goal.max_minutes",
            Self::GoalCostCents => "goal.max_cost_cents",
            Self::GoalNoProgress => "goal.no_progress_turns",
            Self::ContextTokens => "context_tokens",
            Self::Spawns => "max_spawns",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapScope {
    #[default]
    Child,
    Root,
    Project,
    Engine,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapStop {
    pub key: CapKind,
    #[serde(default)]
    pub scope: CapScope,
    pub limit: usize,
    pub used: usize,
    pub resumable: bool,
    #[serde(default)]
    pub run_generation: u64,
}

impl CapStop {
    pub fn valid(&self) -> bool {
        self.limit > 0
            && self.used >= self.limit
            && (self.scope == CapScope::Child || self.key == CapKind::Spawns)
    }

    pub fn parent_can_raise_steps(&self) -> bool {
        self.valid()
            && self.resumable
            && self.key == CapKind::Steps
            && self.scope == CapScope::Child
    }

    pub fn from_error(error: &str) -> Option<Self> {
        let stop: Self = serde_json::from_str(error.strip_prefix("CAP_STOP:")?).ok()?;
        stop.valid().then_some(stop)
    }

    pub fn as_error(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self).map(|json| format!("CAP_STOP:{json}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_typed_caps_round_trip_but_only_child_steps_are_parent_raiseable() {
        for key in [
            CapKind::Steps,
            CapKind::GoalTurns,
            CapKind::GoalTokens,
            CapKind::GoalMinutes,
            CapKind::GoalCostCents,
            CapKind::GoalNoProgress,
            CapKind::ContextTokens,
            CapKind::Spawns,
        ] {
            let stop = CapStop {
                key,
                scope: CapScope::Child,
                limit: 10,
                used: 10,
                resumable: true,
                run_generation: 3,
            };
            assert_eq!(
                CapStop::from_error(&stop.as_error().unwrap()),
                Some(stop.clone())
            );
            assert_eq!(stop.parent_can_raise_steps(), key == CapKind::Steps);
        }
        for text in [
            "queue_full",
            "provider quota exceeded",
            "policy denied",
            "CAP_STOP:not-json",
            r#"CAP_STOP:{"key":"provider_quota","limit":1,"used":1,"resumable":true}"#,
        ] {
            assert!(CapStop::from_error(text).is_none());
        }
    }
}
