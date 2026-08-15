use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DestinationKind {
    Provider,
    Mcp,
    SubagentModel,
    Completion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct DestinationId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Destination {
    pub id: DestinationId,
    pub kind: DestinationKind,
    pub display_name: String,
}

impl Destination {
    pub fn matches_send_to(&self, send_to: &[String]) -> bool {
        send_to
            .iter()
            .any(|allowed| allowed == "*" || allowed == &self.id.0)
    }
}
