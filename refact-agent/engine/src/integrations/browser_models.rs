use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum LocatorStrategy {
    Css {
        value: String,
    },
    Id {
        value: String,
    },
    Name {
        value: String,
    },
    TestId {
        value: String,
    },
    Placeholder {
        value: String,
    },
    Autocomplete {
        value: String,
    },
    Text {
        value: String,
        #[serde(default)]
        exact: bool,
    },
    Label {
        value: String,
    },
    Role {
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Xpath {
        value: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserLocator {
    #[serde(flatten)]
    pub strategy: LocatorStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nth: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
}

impl BrowserLocator {
    pub fn css(selector: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Css {
                value: selector.to_string(),
            },
            nth: None,
            within: None,
        }
    }

    #[allow(dead_code)]
    pub fn id(id: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Id {
                value: id.to_string(),
            },
            nth: None,
            within: None,
        }
    }

    #[allow(dead_code)]
    pub fn name(name: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Name {
                value: name.to_string(),
            },
            nth: None,
            within: None,
        }
    }

    #[allow(dead_code)]
    pub fn label(label: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Label {
                value: label.to_string(),
            },
            nth: None,
            within: None,
        }
    }

    #[allow(dead_code)]
    pub fn placeholder(ph: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Placeholder {
                value: ph.to_string(),
            },
            nth: None,
            within: None,
        }
    }

    #[allow(dead_code)]
    pub fn role(role: &str, name: Option<&str>) -> Self {
        Self {
            strategy: LocatorStrategy::Role {
                role: role.to_string(),
                name: name.map(|s| s.to_string()),
            },
            nth: None,
            within: None,
        }
    }

    #[allow(dead_code)]
    pub fn test_id(tid: &str) -> Self {
        Self {
            strategy: LocatorStrategy::TestId {
                value: tid.to_string(),
            },
            nth: None,
            within: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabTarget {
    Active,
    Id { id: String },
}

impl Default for TabTarget {
    fn default() -> Self {
        TabTarget::Active
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserStep {
    Navigate {
        url: String,
    },
    Reload,
    GoBack,
    GoForward,

    OpenTab {
        #[serde(default)]
        device: Option<String>,
    },
    CloseTab,
    SwitchTab {
        tab: TabTarget,
    },
    ListTabs,

    Click {
        locator: BrowserLocator,
    },
    ClickIfExists {
        locator: BrowserLocator,
    },
    Hover {
        locator: BrowserLocator,
    },
    Focus {
        locator: BrowserLocator,
    },
    Blur {
        locator: BrowserLocator,
    },
    ScrollTo {
        locator: BrowserLocator,
    },
    PressKey {
        key: String,
        #[serde(default)]
        modifiers: Vec<String>,
    },

    Fill {
        locator: BrowserLocator,
        text: String,
        #[serde(default = "default_true")]
        clear_first: bool,
        #[serde(default = "default_true")]
        verify: bool,
    },
    Clear {
        locator: BrowserLocator,
        #[serde(default = "default_true")]
        verify: bool,
    },
    SelectOption {
        locator: BrowserLocator,
        value: String,
    },
    Check {
        locator: BrowserLocator,
    },
    Uncheck {
        locator: BrowserLocator,
    },

    WaitForSelector {
        locator: BrowserLocator,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitForNavigation {
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitForUrl {
        contains: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitForText {
        text: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitForNetworkIdle {
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitForElementHidden {
        locator: BrowserLocator,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitForElementStable {
        locator: BrowserLocator,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitSeconds {
        seconds: f64,
    },

    GetText {
        locator: BrowserLocator,
    },
    GetHtml {
        locator: BrowserLocator,
    },
    GetAttribute {
        locator: BrowserLocator,
        attribute: String,
    },
    ExtractLinks {
        #[serde(default)]
        locator: Option<BrowserLocator>,
        #[serde(default)]
        limit: Option<usize>,
    },
    ExtractTable {
        locator: BrowserLocator,
    },
    DomSnapshot {
        selector: String,
        #[serde(default)]
        max_chars: Option<usize>,
    },
    AccessibilitySnapshot,
    Screenshot,
    ScreenshotElement {
        locator: BrowserLocator,
    },

    Eval {
        expression: String,
    },
    Styles {
        locator: BrowserLocator,
        #[serde(default)]
        property_filter: Option<String>,
    },

    TabLog,

    DismissOverlays,
    HighlightElement {
        locator: BrowserLocator,
    },
}

fn default_true() -> bool {
    true
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionPolicy {
    SharedDefault,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        SessionPolicy::SharedDefault
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserActionRequest {
    #[serde(default)]
    pub session: SessionPolicy,
    #[serde(default)]
    pub target: TabTarget,
    pub steps: Vec<BrowserStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    TextInput,
    PasswordInput,
    EmailInput,
    SearchInput,
    NumberInput,
    TelInput,
    UrlInput,
    Textarea,
    Select,
    Checkbox,
    Radio,
    ContentEditable,
    DateInput,
    FileInput,
    HiddenInput,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FillStrategy {
    NativeTyping,
    DomValueSetter,
    NativePrototypeSetter,
    ContentEditablePath,
    ClickAndType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_index: usize,
    pub ok: bool,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_kind: Option<FieldKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_strategy: Option<FillStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(default)]
    pub retries: u32,
}

impl StepResult {
    pub fn success(step_index: usize, summary: impl Into<String>) -> Self {
        Self {
            step_index,
            ok: true,
            summary: summary.into(),
            error: None,
            data: None,
            field_kind: None,
            fill_strategy: None,
            verified: None,
            retries: 0,
        }
    }

    pub fn failure(
        step_index: usize,
        summary: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            step_index,
            ok: false,
            summary: summary.into(),
            error: Some(error.into()),
            data: None,
            field_kind: None,
            fill_strategy: None,
            verified: None,
            retries: 0,
        }
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub ok: bool,
    pub steps: Vec<StepResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementInfo {
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aria_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub visible: bool,
    pub enabled: bool,
    pub readonly: bool,
    pub content_editable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<ElementBBox>,
    pub field_kind: FieldKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementBBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub tab_id: String,
    pub target_id: String,
    pub url: String,
    pub title: String,
    pub is_active: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locator_css_serde() {
        let loc = BrowserLocator::css("#btn");
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["by"], "css");
        assert_eq!(json["value"], "#btn");
        let parsed: BrowserLocator = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, loc);
    }

    #[test]
    fn test_locator_id_serde() {
        let loc = BrowserLocator::id("email");
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["by"], "id");
        assert_eq!(json["value"], "email");
    }

    #[test]
    fn test_locator_name_serde() {
        let loc = BrowserLocator::name("q");
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["by"], "name");
        assert_eq!(json["value"], "q");
    }

    #[test]
    fn test_locator_label_serde() {
        let loc = BrowserLocator::label("Email Address");
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["by"], "label");
        assert_eq!(json["value"], "Email Address");
    }

    #[test]
    fn test_locator_placeholder_serde() {
        let loc = BrowserLocator::placeholder("Search...");
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["by"], "placeholder");
        assert_eq!(json["value"], "Search...");
    }

    #[test]
    fn test_locator_role_serde() {
        let loc = BrowserLocator::role("textbox", Some("Search"));
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["by"], "role");
        assert_eq!(json["role"], "textbox");
        assert_eq!(json["name"], "Search");
    }

    #[test]
    fn test_locator_role_without_name_serde() {
        let loc = BrowserLocator::role("button", None);
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["by"], "role");
        assert_eq!(json["role"], "button");
        assert!(json.get("name").is_none());
    }

    #[test]
    fn test_locator_test_id_serde() {
        let loc = BrowserLocator::test_id("submit-btn");
        let json = serde_json::to_value(&loc).unwrap();
        assert_eq!(json["by"], "test_id");
        assert_eq!(json["value"], "submit-btn");
    }

    #[test]
    fn test_locator_text_serde() {
        let json_str = r#"{"by": "text", "value": "Submit Form", "exact": true}"#;
        let loc: BrowserLocator = serde_json::from_str(json_str).unwrap();
        match &loc.strategy {
            LocatorStrategy::Text { value, exact } => {
                assert_eq!(value, "Submit Form");
                assert!(*exact);
            }
            _ => panic!("Expected Text"),
        }
    }

    #[test]
    fn test_locator_xpath_serde() {
        let json_str = r#"{"by": "xpath", "value": "//button[@type='submit']"}"#;
        let loc: BrowserLocator = serde_json::from_str(json_str).unwrap();
        match &loc.strategy {
            LocatorStrategy::Xpath { value } => {
                assert!(value.contains("submit"));
            }
            _ => panic!("Expected Xpath"),
        }
    }

    #[test]
    fn test_locator_autocomplete_serde() {
        let json_str = r#"{"by": "autocomplete", "value": "email"}"#;
        let loc: BrowserLocator = serde_json::from_str(json_str).unwrap();
        match &loc.strategy {
            LocatorStrategy::Autocomplete { value } => {
                assert_eq!(value, "email");
            }
            _ => panic!("Expected Autocomplete"),
        }
    }

    #[test]
    fn test_locator_with_nth_and_within() {
        let json_str = r##"{"by": "css", "value": "input", "nth": 2, "within": "#form"}"##;
        let loc: BrowserLocator = serde_json::from_str(json_str).unwrap();
        assert_eq!(loc.nth, Some(2));
        assert_eq!(loc.within.as_deref(), Some("#form"));
    }

    #[test]
    fn test_locator_nth_and_within_omitted_when_none() {
        let loc = BrowserLocator::css("div");
        let json = serde_json::to_value(&loc).unwrap();
        assert!(json.get("nth").is_none());
        assert!(json.get("within").is_none());
    }

    #[test]
    fn test_tab_target_active_serde() {
        let t = TabTarget::Active;
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["type"], "active");
    }

    #[test]
    fn test_tab_target_id_serde() {
        let t = TabTarget::Id {
            id: "main".to_string(),
        };
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["type"], "id");
        assert_eq!(json["id"], "main");
    }

    #[test]
    fn test_step_navigate_serde() {
        let step = BrowserStep::Navigate {
            url: "https://example.com".to_string(),
        };
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["action"], "navigate");
        assert_eq!(json["url"], "https://example.com");
    }

    #[test]
    fn test_step_click_serde() {
        let json_str = r##"{"action": "click", "locator": {"by": "css", "value": "#btn"}}"##;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::Click { locator } => {
                assert_eq!(
                    locator.strategy,
                    LocatorStrategy::Css {
                        value: "#btn".to_string()
                    }
                );
            }
            _ => panic!("Expected Click"),
        }
    }

    #[test]
    fn test_step_fill_serde() {
        let json_str = r#"{
            "action": "fill",
            "locator": {"by": "name", "value": "q"},
            "text": "rust tutorial",
            "clear_first": true,
            "verify": true
        }"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::Fill {
                locator,
                text,
                clear_first,
                verify,
            } => {
                assert_eq!(text, "rust tutorial");
                assert!(clear_first);
                assert!(verify);
                match &locator.strategy {
                    LocatorStrategy::Name { value } => assert_eq!(value, "q"),
                    _ => panic!("Expected Name locator"),
                }
            }
            _ => panic!("Expected Fill"),
        }
    }

    #[test]
    fn test_step_fill_defaults() {
        let json_str = r##"{
            "action": "fill",
            "locator": {"by": "css", "value": "#input"},
            "text": "hello"
        }"##;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::Fill {
                clear_first,
                verify,
                ..
            } => {
                assert!(clear_first, "clear_first should default to true");
                assert!(verify, "verify should default to true");
            }
            _ => panic!("Expected Fill"),
        }
    }

    #[test]
    fn test_step_wait_for_url_serde() {
        let json_str = r#"{"action": "wait_for_url", "contains": "/search", "timeout_ms": 5000}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::WaitForUrl {
                contains,
                timeout_ms,
            } => {
                assert_eq!(contains, "/search");
                assert_eq!(timeout_ms, Some(5000));
            }
            _ => panic!("Expected WaitForUrl"),
        }
    }

    #[test]
    fn test_step_extract_links_serde() {
        let json_str = r#"{"action": "extract_links", "limit": 10}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::ExtractLinks { locator, limit } => {
                assert!(locator.is_none());
                assert_eq!(limit, Some(10));
            }
            _ => panic!("Expected ExtractLinks"),
        }
    }

    #[test]
    fn test_step_eval_serde() {
        let json_str = r#"{"action": "eval", "expression": "document.title"}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::Eval { expression } => {
                assert_eq!(expression, "document.title");
            }
            _ => panic!("Expected Eval"),
        }
    }

    #[test]
    fn test_step_press_key_serde() {
        let json_str = r#"{"action": "press_key", "key": "Enter", "modifiers": ["Ctrl"]}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::PressKey { key, modifiers } => {
                assert_eq!(key, "Enter");
                assert_eq!(modifiers, vec!["Ctrl"]);
            }
            _ => panic!("Expected PressKey"),
        }
    }

    #[test]
    fn test_step_screenshot_serde() {
        let json_str = r#"{"action": "screenshot"}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        assert!(matches!(step, BrowserStep::Screenshot));
    }

    #[test]
    fn test_step_dismiss_overlays_serde() {
        let json_str = r#"{"action": "dismiss_overlays"}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        assert!(matches!(step, BrowserStep::DismissOverlays));
    }

    #[test]
    fn test_step_wait_seconds_serde() {
        let json_str = r#"{"action": "wait_seconds", "seconds": 2.5}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::WaitSeconds { seconds } => {
                assert!((seconds - 2.5).abs() < f64::EPSILON);
            }
            _ => panic!("Expected WaitSeconds"),
        }
    }

    #[test]
    fn test_full_request_serde() {
        let json_str = r#"{
            "session": "shared_default",
            "target": {"type": "active"},
            "steps": [
                {"action": "navigate", "url": "https://www.google.com"},
                {"action": "fill", "locator": {"by": "name", "value": "q"}, "text": "rust tokio tutorial"},
                {"action": "press_key", "key": "Enter"},
                {"action": "wait_for_url", "contains": "/search"},
                {"action": "extract_links", "limit": 10}
            ]
        }"#;
        let req: BrowserActionRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(req.session, SessionPolicy::SharedDefault);
        assert_eq!(req.target, TabTarget::Active);
        assert_eq!(req.steps.len(), 5);
    }

    #[test]
    fn test_request_defaults() {
        let json_str = r#"{"steps": [{"action": "screenshot"}]}"#;
        let req: BrowserActionRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(req.session, SessionPolicy::SharedDefault);
        assert_eq!(req.target, TabTarget::Active);
    }

    #[test]
    fn test_step_result_success() {
        let r = StepResult::success(0, "Navigated to https://example.com");
        assert!(r.ok);
        assert_eq!(r.step_index, 0);
        assert!(r.error.is_none());
    }

    #[test]
    fn test_step_result_failure() {
        let r = StepResult::failure(1, "Click failed", "Element not found: #btn");
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("Element not found: #btn"));
    }

    #[test]
    fn test_step_result_with_data() {
        let r =
            StepResult::success(0, "Extracted").with_data(serde_json::json!(["link1", "link2"]));
        assert!(r.data.is_some());
    }

    #[test]
    fn test_field_kind_serde() {
        let kinds = vec![
            FieldKind::TextInput,
            FieldKind::PasswordInput,
            FieldKind::SearchInput,
            FieldKind::Textarea,
            FieldKind::Select,
            FieldKind::ContentEditable,
        ];
        for kind in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            let parsed: FieldKind = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn test_fill_strategy_serde() {
        let strategies = vec![
            FillStrategy::NativeTyping,
            FillStrategy::DomValueSetter,
            FillStrategy::NativePrototypeSetter,
            FillStrategy::ContentEditablePath,
            FillStrategy::ClickAndType,
        ];
        for s in strategies {
            let json = serde_json::to_string(&s).unwrap();
            let parsed: FillStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, s);
        }
    }

    #[test]
    fn test_execution_report_serde() {
        let report = ExecutionReport {
            ok: true,
            steps: vec![
                StepResult::success(0, "nav ok"),
                StepResult::success(1, "click ok"),
            ],
            url: Some("https://example.com".to_string()),
            title: Some("Example".to_string()),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert!(json["ok"].as_bool().unwrap());
        assert_eq!(json["steps"].as_array().unwrap().len(), 2);
        let parsed: ExecutionReport = serde_json::from_value(json).unwrap();
        assert!(parsed.ok);
    }

    #[test]
    fn test_element_info_parse_from_js_json() {
        let json_str = r#"{
            "tag": "input",
            "input_type": "text",
            "id": "email",
            "name": "email",
            "placeholder": "Enter email",
            "aria_label": null,
            "role": null,
            "visible": true,
            "enabled": true,
            "readonly": false,
            "content_editable": false,
            "value": "",
            "inner_text": null,
            "bbox": {"x": 100.0, "y": 200.0, "width": 300.0, "height": 40.0},
            "field_kind": "text_input"
        }"#;
        let info: ElementInfo = serde_json::from_str(json_str).unwrap();
        assert_eq!(info.tag, "input");
        assert_eq!(info.input_type.as_deref(), Some("text"));
        assert!(info.visible);
        assert!(info.enabled);
        assert_eq!(info.field_kind, FieldKind::TextInput);
    }

    #[test]
    fn test_tab_info_serde() {
        let ti = TabInfo {
            tab_id: "1".to_string(),
            target_id: "ABC123".to_string(),
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            is_active: true,
        };
        let json = serde_json::to_value(&ti).unwrap();
        assert_eq!(json["tab_id"], "1");
        assert!(json["is_active"].as_bool().unwrap());
    }
}
