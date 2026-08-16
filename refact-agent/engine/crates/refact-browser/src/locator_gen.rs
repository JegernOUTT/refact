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
            HandleError::Protocol(format!("Failed to serialize locator generation options: {error}"))
        })?;
        let value = self.call_function_on(
            tab,
            handle,
            GENERATE_LOCATOR_FUNCTION,
            vec![options],
        )?;
        parse_generated_locator(value)
    }
}

fn parse_generated_locator(value: Value) -> Result<String, HandleError> {
    serde_json::from_value(value).map_err(|error| {
        HandleError::Protocol(format!("Failed to parse generated locator: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locator_generation_options_use_injected_wire_names() {
        let options = LocatorGenerationOptions::default();
        assert_eq!(options.test_id_attribute_name, "data-testid");
        assert_eq!(
            serde_json::to_value(options).unwrap(),
            serde_json::json!({ "testIdAttributeName": "data-testid" })
        );
        assert_eq!(
            serde_json::from_value::<LocatorGenerationOptions>(serde_json::json!({})).unwrap(),
            LocatorGenerationOptions::default()
        );
    }

    #[test]
    fn generated_locator_wrapper_deserializes_string_results() {
        assert_eq!(
            parse_generated_locator(serde_json::json!(
                "internal:role=button[name=\"Save\"s]"
            ))
            .unwrap(),
            "internal:role=button[name=\"Save\"s]"
        );
        assert!(matches!(
            parse_generated_locator(serde_json::json!({ "selector": "button" })),
            Err(HandleError::Protocol(message)) if message.contains("generated locator")
        ));
        assert!(GENERATE_LOCATOR_FUNCTION.contains("generateLocator(this, options)"));
    }
}
