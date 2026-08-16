use std::collections::HashMap;

use headless_chrome::Tab;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ElementHandle, ElementHandleInfo, HandleError, Ref, SnapshotGeneration, WorldManager};

const ARIA_SNAPSHOT_FUNCTION: &str = "function(options) { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.ariaSnapshot(this, options); }";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotMode {
    Ai,
    #[default]
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SnapshotOptions {
    pub mode: SnapshotMode,
    pub refs: bool,
    pub boxes: bool,
    pub depth: Option<u32>,
    pub do_not_render_active: bool,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            mode: SnapshotMode::Default,
            refs: false,
            boxes: false,
            depth: None,
            do_not_render_active: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotBox {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotNode {
    pub role: String,
    pub name: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    #[serde(rename = "box")]
    pub geometry: Option<SnapshotBox>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AriaSnapshot {
    pub yaml: String,
    pub nodes: Vec<SnapshotNode>,
    #[serde(default)]
    pub generation: Option<SnapshotGeneration>,
}

impl WorldManager {
    pub fn aria_snapshot(
        &self,
        tab: &Tab,
        root_handle: Option<ElementHandle>,
        options: SnapshotOptions,
    ) -> Result<AriaSnapshot, HandleError> {
        let target_id = tab.get_target_id();
        let mut options = serde_json::to_value(options).map_err(|error| {
            HandleError::Protocol(format!(
                "Failed to serialize ARIA snapshot options: {error}"
            ))
        })?;
        if let Some(prefix) = self.refs.ref_prefix(target_id) {
            options["refPrefix"] = Value::String(prefix);
        }
        let value = if let Some(root_handle) = root_handle {
            self.call_function_on(tab, &root_handle, ARIA_SNAPSHOT_FUNCTION, vec![options])?
        } else {
            self.call_injected(
                tab,
                "ariaSnapshot",
                Value::Array(vec![Value::Null, options]),
            )
            .map_err(HandleError::Resolution)?
        };
        let mut snapshot: AriaSnapshot = serde_json::from_value(value).map_err(|error| {
            HandleError::Protocol(format!("Failed to parse ARIA snapshot: {error}"))
        })?;
        let refs = snapshot
            .nodes
            .iter()
            .filter_map(|node| {
                node.reference.as_ref().map(|reference| {
                    let reference = reference.parse::<Ref>().map_err(|error| {
                        HandleError::Protocol(format!(
                            "Failed to parse ARIA snapshot ref {reference}: {error}"
                        ))
                    })?;
                    Ok((
                        reference,
                        ElementHandleInfo {
                            role: node.role.clone(),
                            name: node.name.clone(),
                        },
                    ))
                })
            })
            .collect::<Result<HashMap<_, _>, HandleError>>()?;
        snapshot.generation = Some(self.refs.replace_snapshot(target_id, refs));
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aria_snapshot_serde_preserves_yaml_and_geometry() {
        let snapshot = AriaSnapshot {
            yaml: "- button \"Save\" [box=1,2,30,40]".to_string(),
            nodes: vec![SnapshotNode {
                role: "button".to_string(),
                name: Some("Save".to_string()),
                reference: None,
                geometry: Some(SnapshotBox {
                    x: 1,
                    y: 2,
                    width: 30,
                    height: 40,
                }),
            }],
            generation: None,
        };
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(value["nodes"][0]["box"]["width"], 30);
        assert_eq!(
            serde_json::from_value::<AriaSnapshot>(value).unwrap(),
            snapshot
        );
    }

    #[test]
    fn snapshot_options_default_to_assertion_mode_without_refs() {
        let options = SnapshotOptions::default();
        assert_eq!(options.mode, SnapshotMode::Default);
        assert!(!options.refs);
        assert!(!options.boxes);
        let value = serde_json::to_value(options).unwrap();
        assert_eq!(value["mode"], "default");
        assert_eq!(value["doNotRenderActive"], false);
        assert!(ARIA_SNAPSHOT_FUNCTION.contains("ariaSnapshot(this, options)"));
    }
}
