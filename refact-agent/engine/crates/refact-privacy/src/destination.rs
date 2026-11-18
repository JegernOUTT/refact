use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DestinationKind {
    Provider,
    Mcp,
    SubagentModel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct DestinationId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Destination {
    pub id: DestinationId,
    #[serde(deserialize_with = "deserialize_legacy_destination_kind")]
    pub kind: DestinationKind,
    pub display_name: String,
}

fn deserialize_legacy_destination_kind<'de, D>(deserializer: D) -> Result<DestinationKind, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    match value.as_str() {
        "completion" | "provider" => Ok(DestinationKind::Provider),
        "mcp" => Ok(DestinationKind::Mcp),
        "subagent_model" => Ok(DestinationKind::SubagentModel),
        _ => Err(serde::de::Error::unknown_variant(
            &value,
            &["provider", "mcp", "subagent_model"],
        )),
    }
}

impl Destination {
    pub fn matches_send_to(&self, send_to: &[String]) -> bool {
        send_to
            .iter()
            .any(|allowed| allowed == "*" || allowed == &self.id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_completion_destination_preserves_allowlist_identity() {
        let destination: Destination =
            serde_json::from_str(r#"{"id":"local","kind":"completion","display_name":"Local"}"#)
                .unwrap();
        assert_eq!(destination.kind, DestinationKind::Provider);
        assert!(destination.matches_send_to(&["local".into()]));
        assert!(!destination.matches_send_to(&["remote".into()]));
        assert_eq!(
            serde_json::to_value(destination).unwrap()["kind"],
            "provider"
        );
        assert!(serde_json::from_str::<Destination>(
            r#"{"id":"local","kind":"unknown","display_name":"Local"}"#,
        )
        .is_err());
    }
}
