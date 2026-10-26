use headless_chrome::Tab;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ElementHandle, HandleError, WorldManager};

const GENERATE_LOCATOR_FUNCTION: &str = "function(options) { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.generateLocator(this, options); }";

fn default_test_id_attribute_name() -> String {
    "data-testid".to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LocatorGenerationOptions {
    pub test_id_attribute_name: String,
}

impl Default for LocatorGenerationOptions {
    fn default() -> Self {
        Self {
            test_id_attribute_name: default_test_id_attribute_name(),
        }
    }
}

impl WorldManager {
    pub fn generate_locator(
        &self,
        tab: &Tab,
        handle: &ElementHandle,
        options: LocatorGenerationOptions,
    ) -> Result<String, HandleError> {
        let options = serde_json::to_value(options).map_err(|error| {
            HandleError::Protocol(format!(
                "Failed to serialize locator generation options: {error}"
            ))
        })?;
        let value = self.call_function_on(tab, handle, GENERATE_LOCATOR_FUNCTION, vec![options])?;
        parse_generated_locator(value)
    }
}

fn parse_generated_locator(value: Value) -> Result<String, HandleError> {
    serde_json::from_value(value).map_err(|error| {
        HandleError::Protocol(format!("Failed to parse generated locator: {error}"))
    })
}
