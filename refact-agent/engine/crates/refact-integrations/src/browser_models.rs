use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::browser_types::{ConsoleEntry, NetworkEntry};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocatorRegex {
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flags: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AriaCheckedState {
    Bool(bool),
    Mixed(AriaMixedState),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AriaMixedState {
    #[serde(rename = "mixed")]
    Mixed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocatorStrategy {
    Ref {
        value: String,
    },
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
        exact: Option<bool>,
        regex: Option<LocatorRegex>,
        attribute: Option<String>,
    },
    Placeholder {
        value: String,
        exact: Option<bool>,
        regex: Option<LocatorRegex>,
    },
    AltText {
        value: String,
        exact: Option<bool>,
        regex: Option<LocatorRegex>,
    },
    Title {
        value: String,
        exact: Option<bool>,
        regex: Option<LocatorRegex>,
    },
    Autocomplete {
        value: String,
    },
    Text {
        value: String,
        exact: bool,
        regex: Option<LocatorRegex>,
    },
    Label {
        value: String,
        exact: Option<bool>,
        regex: Option<LocatorRegex>,
    },
    Role {
        role: String,
        name: Option<String>,
        description: Option<String>,
        exact: Option<bool>,
        name_regex: Option<LocatorRegex>,
        description_regex: Option<LocatorRegex>,
        checked: Option<AriaCheckedState>,
        pressed: Option<AriaCheckedState>,
        selected: Option<bool>,
        expanded: Option<bool>,
        disabled: Option<bool>,
        level: Option<u32>,
        include_hidden: Option<bool>,
    },
    Xpath {
        value: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
enum LocatorWire {
    Ref {
        value: String,
    },
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exact: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        regex: Option<LocatorRegex>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribute: Option<String>,
    },
    Placeholder {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exact: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        regex: Option<LocatorRegex>,
    },
    AltText {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exact: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        regex: Option<LocatorRegex>,
    },
    Title {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exact: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        regex: Option<LocatorRegex>,
    },
    Autocomplete {
        value: String,
    },
    Text {
        value: String,
        #[serde(default, skip_serializing_if = "is_false")]
        exact: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        regex: Option<LocatorRegex>,
    },
    Label {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exact: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        regex: Option<LocatorRegex>,
    },
    Role {
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exact: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name_regex: Option<LocatorRegex>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description_regex: Option<LocatorRegex>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checked: Option<AriaCheckedState>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pressed: Option<AriaCheckedState>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expanded: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        disabled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        include_hidden: Option<bool>,
    },
    Xpath {
        value: String,
    },
}

pub fn locator_strategy_from_wire(
    value: serde_json::Value,
) -> Result<LocatorStrategy, serde_json::Error> {
    serde_json::from_value(value)
}

impl Serialize for LocatorStrategy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = match self {
            Self::Ref { value } => LocatorWire::Ref {
                value: value.clone(),
            },
            Self::Css { value } => LocatorWire::Css {
                value: value.clone(),
            },
            Self::Id { value } => LocatorWire::Id {
                value: value.clone(),
            },
            Self::Name { value } => LocatorWire::Name {
                value: value.clone(),
            },
            Self::TestId {
                value,
                exact,
                regex,
                attribute,
            } => LocatorWire::TestId {
                value: value.clone(),
                exact: *exact,
                regex: regex.clone(),
                attribute: attribute.clone(),
            },
            Self::Placeholder {
                value,
                exact,
                regex,
            } => LocatorWire::Placeholder {
                value: value.clone(),
                exact: *exact,
                regex: regex.clone(),
            },
            Self::AltText {
                value,
                exact,
                regex,
            } => LocatorWire::AltText {
                value: value.clone(),
                exact: *exact,
                regex: regex.clone(),
            },
            Self::Title {
                value,
                exact,
                regex,
            } => LocatorWire::Title {
                value: value.clone(),
                exact: *exact,
                regex: regex.clone(),
            },
            Self::Autocomplete { value } => LocatorWire::Autocomplete {
                value: value.clone(),
            },
            Self::Text {
                value,
                exact,
                regex,
            } => LocatorWire::Text {
                value: value.clone(),
                exact: *exact,
                regex: regex.clone(),
            },
            Self::Label {
                value,
                exact,
                regex,
            } => LocatorWire::Label {
                value: value.clone(),
                exact: *exact,
                regex: regex.clone(),
            },
            Self::Role {
                role,
                name,
                description,
                exact,
                name_regex,
                description_regex,
                checked,
                pressed,
                selected,
                expanded,
                disabled,
                level,
                include_hidden,
            } => LocatorWire::Role {
                role: role.clone(),
                name: name.clone(),
                description: description.clone(),
                exact: *exact,
                name_regex: name_regex.clone(),
                description_regex: description_regex.clone(),
                checked: *checked,
                pressed: *pressed,
                selected: *selected,
                expanded: *expanded,
                disabled: *disabled,
                level: *level,
                include_hidden: *include_hidden,
            },
            Self::Xpath { value } => LocatorWire::Xpath {
                value: value.clone(),
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LocatorStrategy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = LocatorWire::deserialize(deserializer)?;
        Ok(match wire {
            LocatorWire::Ref { value } => Self::Ref { value },
            LocatorWire::Css { value } => Self::Css { value },
            LocatorWire::Id { value } => Self::Id { value },
            LocatorWire::Name { value } => Self::Name { value },
            LocatorWire::TestId {
                value,
                exact,
                regex,
                attribute,
            } => Self::TestId {
                value,
                exact,
                regex,
                attribute,
            },
            LocatorWire::Placeholder {
                value,
                exact,
                regex,
            } => Self::Placeholder {
                value,
                exact,
                regex,
            },
            LocatorWire::AltText {
                value,
                exact,
                regex,
            } => Self::AltText {
                value,
                exact,
                regex,
            },
            LocatorWire::Title {
                value,
                exact,
                regex,
            } => Self::Title {
                value,
                exact,
                regex,
            },
            LocatorWire::Autocomplete { value } => Self::Autocomplete { value },
            LocatorWire::Text {
                value,
                exact,
                regex,
            } => Self::Text {
                value,
                exact,
                regex,
            },
            LocatorWire::Label {
                value,
                exact,
                regex,
            } => Self::Label {
                value,
                exact,
                regex,
            },
            LocatorWire::Role {
                role,
                name,
                description,
                exact,
                name_regex,
                description_regex,
                checked,
                pressed,
                selected,
                expanded,
                disabled,
                level,
                include_hidden,
            } => Self::Role {
                role,
                name,
                description,
                exact,
                name_regex,
                description_regex,
                checked,
                pressed,
                selected,
                expanded,
                disabled,
                level,
                include_hidden,
            },
            LocatorWire::Xpath { value } => Self::Xpath { value },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum LocatorTextMatcher {
    Text(String),
    Regex(LocatorRegex),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LocatorFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has: Option<Box<BrowserLocator>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_not: Option<Box<BrowserLocator>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_text: Option<LocatorTextMatcher>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_not_text: Option<LocatorTextMatcher>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserLocator {
    #[serde(flatten)]
    pub strategy: LocatorStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nth: Option<isize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<Box<BrowserLocator>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<LocatorFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub and: Option<Box<BrowserLocator>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub or: Option<Box<BrowserLocator>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<bool>,
}

impl BrowserLocator {
    pub fn reference(reference: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Ref {
                value: reference.to_string(),
            },
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

    pub fn css(selector: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Css {
                value: selector.to_string(),
            },
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

    #[allow(dead_code)]
    pub fn id(id: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Id {
                value: id.to_string(),
            },
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

    #[allow(dead_code)]
    pub fn name(name: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Name {
                value: name.to_string(),
            },
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

    #[allow(dead_code)]
    pub fn label(label: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Label {
                value: label.to_string(),
                exact: None,
                regex: None,
            },
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

    #[allow(dead_code)]
    pub fn placeholder(ph: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Placeholder {
                value: ph.to_string(),
                exact: None,
                regex: None,
            },
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

    #[allow(dead_code)]
    pub fn role(role: &str, name: Option<&str>) -> Self {
        Self {
            strategy: LocatorStrategy::Role {
                role: role.to_string(),
                name: name.map(|s| s.to_string()),
                description: None,
                exact: None,
                name_regex: None,
                description_regex: None,
                checked: None,
                pressed: None,
                selected: None,
                expanded: None,
                disabled: None,
                level: None,
                include_hidden: None,
            },
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

    #[allow(dead_code)]
    pub fn test_id(tid: &str) -> Self {
        Self {
            strategy: LocatorStrategy::TestId {
                value: tid.to_string(),
                exact: None,
                regex: None,
                attribute: None,
            },
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilitySnapshotMode {
    #[default]
    Ai,
    Default,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AccessibilitySnapshotOptions {
    #[serde(default)]
    pub mode: AccessibilitySnapshotMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<bool>,
    #[serde(default)]
    pub boxes: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<BrowserLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
}

impl AccessibilitySnapshotOptions {
    pub fn refs_enabled(&self) -> bool {
        self.refs
            .unwrap_or(matches!(self.mode, AccessibilitySnapshotMode::Ai))
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
    SetInputFiles {
        locator: BrowserLocator,
        paths: Vec<String>,
    },
    ExpectFileChooser {
        paths: Vec<String>,
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
    WaitForLoadState {
        state: BrowserLoadState,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitForRequest {
        url_or_pattern: UrlPattern,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitForResponse {
        url_or_pattern: UrlPattern,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    WaitForDownload {
        #[serde(default)]
        timeout_ms: Option<u64>,
        #[serde(default)]
        save_as: Option<String>,
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
    AccessibilitySnapshot {
        #[serde(flatten)]
        options: AccessibilitySnapshotOptions,
    },
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

    AddLocatorHandler {
        name: String,
        locator: BrowserLocator,
        handler: LocatorHandlerAction,
        #[serde(default)]
        times: Option<u32>,
        #[serde(default)]
        no_wait_after: bool,
    },
    RemoveLocatorHandler {
        name: String,
    },
    HandleDialog {
        accept: bool,
        #[serde(default)]
        prompt_text: Option<String>,
    },

    DismissOverlays,
    HighlightElement {
        locator: BrowserLocator,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocatorHandlerAction {
    Click,
    Steps { steps: Vec<BrowserStep> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLoadState {
    Domcontentloaded,
    Load,
    Networkidle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum UrlPattern {
    Text(String),
    Regex { source: String, flags: String },
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
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
    #[serde(default)]
    pub attach_screenshot: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionabilityDiagnostics {
    pub call_log: Vec<String>,
    pub timed_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attached: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receives_events: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intercepting_element: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actionability: Option<ActionabilityDiagnostics>,
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
            actionability: None,
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
            actionability: None,
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
    #[serde(default)]
    pub stabilized: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub console: Vec<ConsoleEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub page_errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<NetworkEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locator_handlers: Vec<LocatorHandlerFiring>,
    pub dialogs: Vec<DialogInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uploads: Vec<UploadInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downloads: Vec<DownloadInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<BrowserScreenshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    InProgress,
    Completed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadInfo {
    pub guid: String,
    pub url: String,
    pub frame_id: String,
    pub suggested_filename: String,
    pub local_path: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub state: DownloadState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadInfo {
    pub paths: Vec<String>,
    pub source: String,
    pub in_memory_payloads: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocatorHandlerFiring {
    pub name: String,
    pub action: String,
    pub outcome: String,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DialogType {
    Alert,
    Confirm,
    Prompt,
    Beforeunload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DialogAction {
    Accepted,
    Dismissed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DialogInfo {
    #[serde(rename = "type")]
    pub dialog_type: DialogType,
    pub message: String,
    pub default_value: String,
    pub action: DialogAction,
    pub automatic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserScreenshot {
    pub mime: String,
    pub data: String,
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
    fn ref_locator_round_trips() {
        for reference in ["e12", "f2e7"] {
            let locator = BrowserLocator::reference(reference);
            let value = serde_json::to_value(&locator).unwrap();
            assert_eq!(value, serde_json::json!({"by": "ref", "value": reference}));
            assert_eq!(
                serde_json::from_value::<BrowserLocator>(value).unwrap(),
                locator
            );
        }
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
            LocatorStrategy::Text { value, exact, .. } => {
                assert_eq!(value, "Submit Form");
                assert!(*exact);
            }
            _ => panic!("Expected Text"),
        }
    }

    #[test]
    fn old_locator_wire_shapes_still_round_trip() {
        for json in [
            r#"{"by":"test_id","value":"save"}"#,
            r#"{"by":"placeholder","value":"Search"}"#,
            r#"{"by":"text","value":"Submit"}"#,
            r#"{"by":"label","value":"Email"}"#,
            r#"{"by":"role","role":"button","name":"Save"}"#,
        ] {
            let locator: BrowserLocator = serde_json::from_str(json).unwrap();
            let round_trip = serde_json::to_value(&locator).unwrap();
            let expected: serde_json::Value = serde_json::from_str(json).unwrap();
            assert_eq!(round_trip, expected);
        }
    }

    #[test]
    fn get_by_locator_options_round_trip() {
        let json = serde_json::json!({
            "by": "role",
            "role": "checkbox",
            "name_regex": {"source": "save\\s+item", "flags": "i"},
            "description": "primary action",
            "description_regex": {"source": "primary", "flags": "u"},
            "exact": true,
            "checked": "mixed",
            "pressed": false,
            "selected": true,
            "expanded": false,
            "disabled": true,
            "level": 3,
            "include_hidden": true,
            "nth": 1,
            "within": "#dialog"
        });
        let locator: BrowserLocator = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(locator).unwrap(), json);
    }

    #[test]
    fn attribute_and_regex_locator_options_round_trip() {
        for json in [
            serde_json::json!({
                "by": "test_id",
                "value": "ignored for regex",
                "regex": {"source": "save-\\d+", "flags": "i"},
                "exact": true,
                "attribute": "data-qa"
            }),
            serde_json::json!({"by": "alt_text", "value": "logo", "exact": false}),
            serde_json::json!({"by": "title", "value": "Help", "exact": true}),
        ] {
            let locator: BrowserLocator = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(locator).unwrap(), json);
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
    fn composed_locator_wire_round_trips() {
        let json = serde_json::json!({
            "by": "css",
            "value": ".card",
            "locator": {"by": "role", "role": "button", "name": "Open"},
            "filter": {
                "has": {"by": "css", "value": ".ready"},
                "has_not": {"by": "css", "value": ".disabled"},
                "has_text": "Alpha",
                "has_not_text": {"source": "archived", "flags": "i"},
                "visible": true
            },
            "and": {"by": "css", "value": ".featured"},
            "or": {"by": "test_id", "value": "fallback"},
            "last": true
        });
        let locator: BrowserLocator = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(locator).unwrap(), json);
    }

    #[test]
    fn first_last_and_signed_nth_round_trip() {
        for json in [
            serde_json::json!({"by":"css","value":"li","first":true}),
            serde_json::json!({"by":"css","value":"li","last":true}),
            serde_json::json!({"by":"css","value":"li","nth":-1}),
        ] {
            let locator: BrowserLocator = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(locator).unwrap(), json);
        }
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
    fn test_network_wait_steps_serde() {
        let load: BrowserStep = serde_json::from_str(
            r#"{"action":"wait_for_load_state","state":"networkidle","timeout_ms":7000}"#,
        )
        .unwrap();
        assert!(matches!(
            load,
            BrowserStep::WaitForLoadState {
                state: BrowserLoadState::Networkidle,
                timeout_ms: Some(7000)
            }
        ));

        let response: BrowserStep = serde_json::from_str(
            r#"{"action":"wait_for_response","url_or_pattern":{"source":"/api/.*","flags":"i"}}"#,
        )
        .unwrap();
        assert!(matches!(
            response,
            BrowserStep::WaitForResponse {
                url_or_pattern: UrlPattern::Regex { source, flags },
                timeout_ms: None
            } if source == "/api/.*" && flags == "i"
        ));
    }

    #[test]
    fn test_file_transfer_steps_serde() {
        let upload: BrowserStep = serde_json::from_str(
            r##"{"action":"set_input_files","locator":{"by":"css","value":"#file"},"paths":["/workspace/a.txt"]}"##,
        )
        .unwrap();
        assert!(matches!(
            upload,
            BrowserStep::SetInputFiles { paths, .. } if paths == vec!["/workspace/a.txt"]
        ));

        let chooser: BrowserStep = serde_json::from_str(
            r#"{"action":"expect_file_chooser","paths":["/workspace/a.txt"]}"#,
        )
        .unwrap();
        assert!(matches!(chooser, BrowserStep::ExpectFileChooser { paths } if paths.len() == 1));

        let download: BrowserStep = serde_json::from_str(
            r#"{"action":"wait_for_download","timeout_ms":9000,"save_as":"saved.txt"}"#,
        )
        .unwrap();
        assert!(matches!(
            download,
            BrowserStep::WaitForDownload {
                timeout_ms: Some(9000),
                save_as: Some(name)
            } if name == "saved.txt"
        ));
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
    fn accessibility_snapshot_options_default_to_ai_refs_and_round_trip() {
        let step: BrowserStep =
            serde_json::from_str(r#"{"action":"accessibility_snapshot"}"#).unwrap();
        match step {
            BrowserStep::AccessibilitySnapshot { options } => {
                assert_eq!(options.mode, AccessibilitySnapshotMode::Ai);
                assert!(options.refs_enabled());
                assert!(!options.boxes);
                assert!(options.root.is_none());
                assert!(options.max_chars.is_none());
            }
            _ => panic!("Expected AccessibilitySnapshot"),
        }

        let value = serde_json::json!({
            "action": "accessibility_snapshot",
            "mode": "default",
            "refs": true,
            "boxes": true,
            "root": {"by": "css", "value": "main"},
            "max_chars": 4096
        });
        let step: BrowserStep = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(step).unwrap(), value);
    }

    #[test]
    fn test_step_handle_dialog_serde() {
        let json_str = r#"{"action": "handle_dialog", "accept": true, "prompt_text": "answer"}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::HandleDialog {
                accept,
                prompt_text,
            } => {
                assert!(accept);
                assert_eq!(prompt_text.as_deref(), Some("answer"));
            }
            _ => panic!("Expected HandleDialog"),
        }
    }

    #[test]
    fn test_step_dismiss_overlays_serde() {
        let json_str = r#"{"action": "dismiss_overlays"}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        assert!(matches!(step, BrowserStep::DismissOverlays));
    }

    #[test]
    fn test_locator_handler_steps_serde() {
        let json_str = r##"{
            "action": "add_locator_handler",
            "name": "cookie",
            "locator": {"by": "css", "value": "#accept"},
            "handler": {"type": "click"},
            "times": 2,
            "no_wait_after": true
        }"##;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::AddLocatorHandler {
                name,
                handler,
                times,
                no_wait_after,
                ..
            } => {
                assert_eq!(name, "cookie");
                assert!(matches!(handler, LocatorHandlerAction::Click));
                assert_eq!(times, Some(2));
                assert!(no_wait_after);
            }
            _ => panic!("expected add_locator_handler"),
        }

        let remove: BrowserStep =
            serde_json::from_str(r#"{"action":"remove_locator_handler","name":"cookie"}"#).unwrap();
        assert!(matches!(
            remove,
            BrowserStep::RemoveLocatorHandler { name } if name == "cookie"
        ));
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
        assert!(!req.attach_screenshot);
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
    fn step_result_actionability_round_trips_all_none_states() {
        let mut result = StepResult::failure(2, "Click failed", "Timed out");
        result.actionability = Some(ActionabilityDiagnostics {
            call_log: vec![
                "waiting for css=#submit".to_string(),
                "element is not stable".to_string(),
            ],
            timed_out: true,
            elapsed_ms: Some(5_000),
            attempts: Some(3),
            attached: None,
            visible: None,
            stable: None,
            enabled: None,
            editable: None,
            receives_events: None,
            intercepting_element: None,
        });

        let json = serde_json::to_value(&result).unwrap();
        let actionability = json["actionability"].as_object().unwrap();
        assert_eq!(actionability["call_log"].as_array().unwrap().len(), 2);
        assert_eq!(actionability["timed_out"], true);
        assert_eq!(actionability["elapsed_ms"], 5_000);
        assert_eq!(actionability["attempts"], 3);
        for omitted in [
            "attached",
            "visible",
            "stable",
            "enabled",
            "editable",
            "receives_events",
            "intercepting_element",
        ] {
            assert!(!actionability.contains_key(omitted), "{omitted}");
        }

        let parsed: StepResult = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.actionability, result.actionability);
    }

    #[test]
    fn step_result_without_actionability_keeps_the_legacy_wire_shape() {
        let json = serde_json::to_value(StepResult::success(0, "Clicked")).unwrap();

        assert!(json.get("actionability").is_none());
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
            stabilized: true,
            console: vec![],
            page_errors: vec![],
            network: vec![],
            locator_handlers: vec![],
            dialogs: vec![DialogInfo {
                dialog_type: DialogType::Prompt,
                message: "Enter password=[REDACTED]".to_string(),
                default_value: "default".to_string(),
                action: DialogAction::Accepted,
                automatic: false,
            }],
            uploads: vec![UploadInfo {
                paths: vec!["/workspace/fixture.txt".to_string()],
                source: "direct".to_string(),
                in_memory_payloads: false,
            }],
            downloads: vec![DownloadInfo {
                guid: "download-guid".to_string(),
                url: "https://example.com/fixture.txt".to_string(),
                frame_id: "frame".to_string(),
                suggested_filename: "fixture.txt".to_string(),
                local_path: "/runtime/download-guid".to_string(),
                received_bytes: 7,
                total_bytes: 7,
                state: DownloadState::Completed,
            }],
            screenshot: None,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert!(json["ok"].as_bool().unwrap());
        assert_eq!(json["steps"].as_array().unwrap().len(), 2);
        assert_eq!(json["dialogs"][0]["type"], "prompt");
        assert_eq!(json["dialogs"][0]["action"], "accepted");
        assert_eq!(json["dialogs"][0]["automatic"], false);
        assert_eq!(json["uploads"][0]["source"], "direct");
        assert_eq!(json["downloads"][0]["state"], "completed");
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
