use refact_integrations::browser_models::{BrowserLocator, locator_strategy_from_wire};

pub const DEFAULT_TEST_ID_ATTRIBUTE: &str = "data-testid";

pub fn test_id_locator(value: impl Into<String>, attribute: impl Into<String>) -> BrowserLocator {
    let attribute = attribute.into();
    let mut wire = serde_json::json!({"by": "test_id", "value": value.into()});
    if attribute != DEFAULT_TEST_ID_ATTRIBUTE {
        wire["attribute"] = attribute.into();
    }
    BrowserLocator {
        strategy: locator_strategy_from_wire(wire).expect("test id locator wire is valid"),
        nth: None,
        within: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_test_id_attribute_stays_implicit() {
        assert_eq!(test_id_locator("save", DEFAULT_TEST_ID_ATTRIBUTE), BrowserLocator::test_id("save"));
    }

    #[test]
    fn custom_test_id_attribute_is_serialized() {
        let locator = test_id_locator("save", "data-qa");
        let value = serde_json::to_value(locator).unwrap();
        assert_eq!(value["attribute"], "data-qa");
    }
}
