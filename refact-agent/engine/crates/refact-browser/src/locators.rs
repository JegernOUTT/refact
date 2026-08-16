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
        frames: Vec::new(),
        nth: None,
        within: None,
        locator: None,
        filter: None,
        and: None,
        or: None,
        first: None,
        last: None,
    }
}

pub fn strict_mode_violation(locator: &str, count: usize, previews: &[String]) -> String {
    let mut message =
        format!("Strict mode violation: locator {locator} resolved to {count} elements");
    for (index, preview) in previews.iter().take(5).enumerate() {
        message.push_str(&format!("\n  {}) {preview}", index + 1));
    }
    if count > previews.len().min(5) {
        message.push_str("\n  ...");
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_test_id_attribute_stays_implicit() {
        assert_eq!(
            test_id_locator("save", DEFAULT_TEST_ID_ATTRIBUTE),
            BrowserLocator::test_id("save")
        );
    }

    #[test]
    fn custom_test_id_attribute_is_serialized() {
        let locator = test_id_locator("save", "data-qa");
        let value = serde_json::to_value(locator).unwrap();
        assert_eq!(value["attribute"], "data-qa");
    }

    #[test]
    fn strict_violation_names_locator_count_and_previews() {
        let message = strict_mode_violation(
            "role=button[Save]",
            7,
            &[
                "<button>Save</button>".to_string(),
                "<button>Save draft</button>".to_string(),
            ],
        );
        assert!(message.contains("role=button[Save]"));
        assert!(message.contains("resolved to 7 elements"));
        assert!(message.contains("<button>Save</button>"));
        assert!(message.contains("<button>Save draft</button>"));
        assert!(message.ends_with("..."));
    }
}
