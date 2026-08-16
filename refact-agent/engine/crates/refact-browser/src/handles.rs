use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex};

use headless_chrome::Tab;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::world::WorldManager;

const GET_ACCESSIBLE_NAME_FUNCTION: &str = "function(includeHidden) { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.getAccessibleName(this, includeHidden); }";
const GET_ACCESSIBLE_DESCRIPTION_FUNCTION: &str = "function() { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.getAccessibleDescription(this); }";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementStateName {
    Visible,
    Enabled,
    Editable,
    Checked,
    Unchecked,
    Mixed,
    Stable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckedState {
    Checked,
    Unchecked,
    Mixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementState {
    pub visible: bool,
    pub enabled: bool,
    pub editable: Option<bool>,
    pub checked: Option<CheckedState>,
    pub stable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementHandle {
    pub object_id: String,
    pub backend_node_id: Option<i64>,
    pub context_id: i64,
    pub frame_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandleError {
    Invalidated { object_id: String },
    Protocol(String),
    Resolution(String),
}

impl Display for HandleError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalidated { object_id } => {
                write!(formatter, "Element handle {object_id} is no longer valid")
            }
            Self::Protocol(message) | Self::Resolution(message) => formatter.write_str(message),
        }
    }
}

impl Error for HandleError {}

impl WorldManager {
    pub fn element_state(
        &self,
        tab: &Tab,
        handle: &ElementHandle,
        state: ElementStateName,
    ) -> Result<Value, HandleError> {
        let state = serde_json::to_value(state).map_err(|error| {
            HandleError::Protocol(format!(
                "Failed to serialize browser element state: {error}"
            ))
        })?;
        self.call_function_on(
            tab,
            handle,
            "function(state) { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.elementState(this, state); }",
            vec![state],
        )
    }

    pub fn element_states(
        &self,
        tab: &Tab,
        handle: &ElementHandle,
    ) -> Result<ElementState, HandleError> {
        let value = self.call_function_on(
            tab,
            handle,
            "function() { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.elementStates(this); }",
            Vec::new(),
        )?;
        serde_json::from_value(value).map_err(|error| {
            HandleError::Protocol(format!("Failed to parse browser element states: {error}"))
        })
    }

    pub fn get_accessible_name(
        &self,
        tab: &Tab,
        handle: &ElementHandle,
        include_hidden: bool,
    ) -> Result<String, HandleError> {
        let value = self.call_function_on(
            tab,
            handle,
            GET_ACCESSIBLE_NAME_FUNCTION,
            vec![Value::Bool(include_hidden)],
        )?;
        parse_accessible_text(value, "name")
    }

    pub fn get_accessible_description(
        &self,
        tab: &Tab,
        handle: &ElementHandle,
    ) -> Result<String, HandleError> {
        let value =
            self.call_function_on(tab, handle, GET_ACCESSIBLE_DESCRIPTION_FUNCTION, Vec::new())?;
        parse_accessible_text(value, "description")
    }

    pub fn expectation_values(
        &self,
        tab: &Tab,
        handle: &ElementHandle,
    ) -> Result<Value, HandleError> {
        self.call_function_on(
            tab,
            handle,
            "function() { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.expectationValues(this); }",
            Vec::new(),
        )
    }
}

fn parse_accessible_text(value: Value, kind: &str) -> Result<String, HandleError> {
    serde_json::from_value(value).map_err(|error| {
        HandleError::Protocol(format!(
            "Failed to parse browser accessible {kind}: {error}"
        ))
    })
}

#[derive(Default)]
struct RegistryState {
    tabs: HashMap<String, Vec<ElementHandle>>,
}

#[derive(Clone, Default)]
pub struct HandleRegistry {
    state: Arc<Mutex<RegistryState>>,
}

impl HandleRegistry {
    pub fn register(&self, target_id: &str, handle: ElementHandle) {
        self.state
            .lock()
            .unwrap()
            .tabs
            .entry(target_id.to_string())
            .or_default()
            .push(handle);
    }

    pub fn validate(&self, target_id: &str, handle: &ElementHandle) -> Result<(), HandleError> {
        let is_active = self
            .state
            .lock()
            .unwrap()
            .tabs
            .get(target_id)
            .map(|handles| handles.iter().any(|candidate| candidate == handle))
            .unwrap_or(false);
        if is_active {
            Ok(())
        } else {
            Err(HandleError::Invalidated {
                object_id: handle.object_id.clone(),
            })
        }
    }

    pub fn dispose(&self, target_id: &str, handle: &ElementHandle) -> bool {
        let mut state = self.state.lock().unwrap();
        let Some(handles) = state.tabs.get_mut(target_id) else {
            return false;
        };
        let Some(index) = handles.iter().position(|candidate| candidate == handle) else {
            return false;
        };
        handles.remove(index);
        if handles.is_empty() {
            state.tabs.remove(target_id);
        }
        true
    }

    pub fn context_destroyed(&self, target_id: &str, context_id: i64) -> Vec<ElementHandle> {
        self.remove_matching(target_id, |handle| handle.context_id == context_id)
    }

    pub fn frame_navigated(&self, target_id: &str, frame_id: &str) -> Vec<ElementHandle> {
        self.remove_matching(target_id, |handle| handle.frame_id == frame_id)
    }

    pub fn contexts_cleared(&self, target_id: &str) -> Vec<ElementHandle> {
        self.state
            .lock()
            .unwrap()
            .tabs
            .remove(target_id)
            .unwrap_or_default()
    }

    fn remove_matching(
        &self,
        target_id: &str,
        predicate: impl Fn(&ElementHandle) -> bool,
    ) -> Vec<ElementHandle> {
        let mut state = self.state.lock().unwrap();
        let Some(handles) = state.tabs.get_mut(target_id) else {
            return Vec::new();
        };
        let mut removed = Vec::new();
        let mut retained = Vec::with_capacity(handles.len());
        for handle in handles.drain(..) {
            if predicate(&handle) {
                removed.push(handle);
            } else {
                retained.push(handle);
            }
        }
        *handles = retained;
        if handles.is_empty() {
            state.tabs.remove(target_id);
        }
        removed
    }

    #[cfg(test)]
    fn len(&self, target_id: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .tabs
            .get(target_id)
            .map(Vec::len)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(object_id: &str, context_id: i64, frame_id: &str) -> ElementHandle {
        ElementHandle {
            object_id: object_id.to_string(),
            backend_node_id: Some(42),
            context_id,
            frame_id: frame_id.to_string(),
        }
    }

    #[test]
    fn handle_lifecycle_moves_from_active_to_invalidated() {
        let registry = HandleRegistry::default();
        let handle = handle("object-1", 7, "frame-1");
        registry.register("tab", handle.clone());
        assert_eq!(registry.validate("tab", &handle), Ok(()));
        assert!(registry.dispose("tab", &handle));
        assert!(matches!(
            registry.validate("tab", &handle),
            Err(HandleError::Invalidated { .. })
        ));
    }

    #[test]
    fn context_destruction_releases_only_matching_handles_in_registration_order() {
        let registry = HandleRegistry::default();
        registry.register("tab", handle("first", 7, "frame-1"));
        registry.register("tab", handle("second", 8, "frame-2"));
        registry.register("tab", handle("third", 7, "frame-1"));
        let removed = registry.context_destroyed("tab", 7);
        assert_eq!(
            removed
                .iter()
                .map(|handle| handle.object_id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "third"]
        );
        assert_eq!(registry.len("tab"), 1);
    }

    #[test]
    fn navigation_disposes_frame_handles_without_touching_siblings() {
        let registry = HandleRegistry::default();
        registry.register("tab", handle("main", 7, "main"));
        registry.register("tab", handle("child", 8, "child"));
        let removed = registry.frame_navigated("tab", "main");
        assert_eq!(removed, vec![handle("main", 7, "main")]);
        assert_eq!(registry.len("tab"), 1);
    }

    #[test]
    fn clearing_contexts_preserves_disposal_order() {
        let registry = HandleRegistry::default();
        registry.register("tab", handle("first", 7, "main"));
        registry.register("tab", handle("second", 8, "child"));
        let removed = registry.contexts_cleared("tab");
        assert_eq!(
            removed
                .iter()
                .map(|handle| handle.object_id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert_eq!(registry.len("tab"), 0);
    }

    #[test]
    fn element_state_serde_preserves_optional_and_mixed_values() {
        let state = ElementState {
            visible: true,
            enabled: false,
            editable: Some(false),
            checked: Some(CheckedState::Mixed),
            stable: true,
        };
        let value = serde_json::to_value(&state).unwrap();
        assert_eq!(value["checked"], "mixed");
        assert_eq!(
            serde_json::from_value::<ElementState>(value).unwrap(),
            state
        );

        let unsupported: ElementState = serde_json::from_value(serde_json::json!({
            "visible": false,
            "enabled": true,
            "editable": null,
            "checked": null,
            "stable": false
        }))
        .unwrap();
        assert_eq!(unsupported.editable, None);
        assert_eq!(unsupported.checked, None);
    }

    #[test]
    fn element_state_names_use_injected_wire_values() {
        assert_eq!(
            serde_json::to_value(ElementStateName::Unchecked).unwrap(),
            "unchecked"
        );
        assert_eq!(
            serde_json::to_value(ElementStateName::Stable).unwrap(),
            "stable"
        );
    }

    #[test]
    fn accessible_text_wrappers_deserialize_string_results() {
        assert_eq!(
            parse_accessible_text(serde_json::json!("First\u{a0}Second"), "name").unwrap(),
            "First\u{a0}Second"
        );
        assert!(matches!(
            parse_accessible_text(serde_json::json!({ "text": "wrong wire type" }), "description"),
            Err(HandleError::Protocol(message)) if message.contains("accessible description")
        ));
        assert!(GET_ACCESSIBLE_NAME_FUNCTION.contains("getAccessibleName(this, includeHidden)"));
        assert!(GET_ACCESSIBLE_DESCRIPTION_FUNCTION.contains("getAccessibleDescription(this)"));
    }
}
