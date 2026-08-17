use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::browser_types::{ConsoleEntry, NetworkEntry};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketRouteMode {
    Mock,
    ObserveAndModify,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketEventKind {
    Created,
    HandshakeResponse,
    FrameSent,
    FrameReceived,
    Closed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSocketEvent {
    pub sequence: u64,
    pub socket_id: String,
    pub url: String,
    pub kind: WebSocketEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opcode: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub routed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarMode {
    Full,
    Minimal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarContentPolicy {
    Omit,
    Embed,
    Attach,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarNotFound {
    Abort,
    Fallback,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAuthenticatorProtocol {
    U2f,
    #[default]
    Ctap2,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAuthenticatorTransport {
    #[default]
    Usb,
    Nfc,
    Ble,
    Cable,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserWebAuthnCredential {
    pub credential_id: String,
    #[serde(default)]
    pub is_resident_credential: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rp_id: Option<String>,
    pub private_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_handle: Option<String>,
    #[serde(default)]
    pub sign_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub large_blob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_eligibility: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_state: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_display_name: Option<String>,
}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<BrowserLocator>,
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

    pub fn css(selector: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Css {
                value: selector.to_string(),
            },
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

    #[allow(dead_code)]
    pub fn id(id: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Id {
                value: id.to_string(),
            },
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

    #[allow(dead_code)]
    pub fn name(name: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Name {
                value: name.to_string(),
            },
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

    #[allow(dead_code)]
    pub fn label(label: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Label {
                value: label.to_string(),
                exact: None,
                regex: None,
            },
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

    #[allow(dead_code)]
    pub fn placeholder(ph: &str) -> Self {
        Self {
            strategy: LocatorStrategy::Placeholder {
                value: ph.to_string(),
                exact: None,
                regex: None,
            },
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

    #[allow(dead_code)]
    pub fn test_id(tid: &str) -> Self {
        Self {
            strategy: LocatorStrategy::TestId {
                value: tid.to_string(),
                exact: None,
                regex: None,
                attribute: None,
            },
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

    pub fn in_frames(mut self, frames: Vec<BrowserLocator>) -> Self {
        self.frames = frames;
        self
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum BrowserExpectedText {
    Text(String),
    Regex(LocatorRegex),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserExpectation {
    ToBeAttached,
    ToBeVisible,
    ToBeHidden,
    ToBeEnabled,
    ToBeDisabled,
    ToBeEditable,
    ToBeChecked,
    ToBeFocused,
    ToBeEmpty,
    ToBeInViewport,
    ToHaveText {
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToContainText {
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveValue {
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveValues {
        expected: Vec<BrowserExpectedText>,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveAttribute {
        name: String,
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveClass {
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToContainClass {
        expected: String,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveCount {
        expected: usize,
    },
    ToHaveCss {
        name: String,
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveId {
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveJsProperty {
        name: String,
        expected: serde_json::Value,
    },
    ToHaveRole {
        expected: String,
    },
    ToHaveAccessibleName {
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveAccessibleDescription {
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveUrl {
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToHaveTitle {
        expected: BrowserExpectedText,
        #[serde(default)]
        ignore_case: bool,
    },
    ToMatchAriaSnapshot {
        expected: String,
    },
}

impl BrowserExpectation {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ToBeAttached => "to_be_attached",
            Self::ToBeVisible => "to_be_visible",
            Self::ToBeHidden => "to_be_hidden",
            Self::ToBeEnabled => "to_be_enabled",
            Self::ToBeDisabled => "to_be_disabled",
            Self::ToBeEditable => "to_be_editable",
            Self::ToBeChecked => "to_be_checked",
            Self::ToBeFocused => "to_be_focused",
            Self::ToBeEmpty => "to_be_empty",
            Self::ToBeInViewport => "to_be_in_viewport",
            Self::ToHaveText { .. } => "to_have_text",
            Self::ToContainText { .. } => "to_contain_text",
            Self::ToHaveValue { .. } => "to_have_value",
            Self::ToHaveValues { .. } => "to_have_values",
            Self::ToHaveAttribute { .. } => "to_have_attribute",
            Self::ToHaveClass { .. } => "to_have_class",
            Self::ToContainClass { .. } => "to_contain_class",
            Self::ToHaveCount { .. } => "to_have_count",
            Self::ToHaveCss { .. } => "to_have_css",
            Self::ToHaveId { .. } => "to_have_id",
            Self::ToHaveJsProperty { .. } => "to_have_js_property",
            Self::ToHaveRole { .. } => "to_have_role",
            Self::ToHaveAccessibleName { .. } => "to_have_accessible_name",
            Self::ToHaveAccessibleDescription { .. } => "to_have_accessible_description",
            Self::ToHaveUrl { .. } => "to_have_url",
            Self::ToHaveTitle { .. } => "to_have_title",
            Self::ToMatchAriaSnapshot { .. } => "to_match_aria_snapshot",
        }
    }

    pub fn requires_locator(&self) -> bool {
        !matches!(self, Self::ToHaveUrl { .. } | Self::ToHaveTitle { .. })
    }

    pub fn is_multi_element(&self) -> bool {
        matches!(self, Self::ToHaveCount { .. } | Self::ToHaveValues { .. })
    }
}

impl AccessibilitySnapshotOptions {
    pub fn refs_enabled(&self) -> bool {
        self.refs
            .unwrap_or(matches!(self.mode, AccessibilitySnapshotMode::Ai))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserScreenshotType {
    #[default]
    Png,
    Jpeg,
    Webp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserScreenshotScale {
    Css,
    #[default]
    Device,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserScreenshotAnimations {
    #[default]
    Allow,
    Disabled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserScreenshotCaret {
    #[default]
    Hide,
    Initial,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct BrowserScreenshotClip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BrowserScreenshotOptions {
    #[serde(default, skip_serializing_if = "is_false")]
    pub full_page: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<BrowserScreenshotClip>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub image_type: Option<BrowserScreenshotType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<BrowserScreenshotScale>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub omit_background: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animations: Option<BrowserScreenshotAnimations>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caret: Option<BrowserScreenshotCaret>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mask: Vec<BrowserLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BrowserPdfMargin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BrowserPdfOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landscape: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_background: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margins: Option<BrowserPdfMargin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_ranges: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_css_page_size: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tagged: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct BrowserPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMouseButton {
    #[default]
    Left,
    Middle,
    Right,
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
        #[serde(default)]
        url: Option<String>,
    },
    CloseTab {
        #[serde(default)]
        tab: Option<TabTarget>,
    },
    SwitchTab {
        tab: TabTarget,
    },
    ListTabs,
    WaitForPopup {
        #[serde(default)]
        timeout_ms: Option<u64>,
    },

    Route {
        pattern: UrlPattern,
        handler: RouteHandler,
    },
    Unroute {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<UrlPattern>,
    },
    ListRoutes,

    RouteWebSocket {
        pattern: UrlPattern,
        mode: WebSocketRouteMode,
    },
    UnrouteWebSocket {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<UrlPattern>,
    },
    SendWebSocketMessage {
        url_pattern: UrlPattern,
        data: String,
    },
    WaitForWebSocketFrame {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<UrlPattern>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },

    StartHarRecording {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        mode: HarMode,
        content: HarContentPolicy,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url_filter: Option<UrlPattern>,
    },
    StopHarRecording,
    RouteFromHar {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url_filter: Option<UrlPattern>,
        not_found: HarNotFound,
    },

    StartCoverage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        js: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        css: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reset_on_navigation: Option<bool>,
    },
    StopCoverage,

    AddVirtualAuthenticator {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        protocol: Option<BrowserAuthenticatorProtocol>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transport: Option<BrowserAuthenticatorTransport>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        has_resident_key: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        has_user_verification: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_user_verified: Option<bool>,
    },
    RemoveVirtualAuthenticator {
        id: String,
    },
    ListCredentials {
        id: String,
    },
    AddCredential {
        id: String,
        credential: BrowserWebAuthnCredential,
    },
    ClearCredentials {
        id: String,
    },
    SetUserVerified {
        id: String,
        verified: bool,
    },

    SetViewport {
        width: u32,
        height: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_scale_factor: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_mobile: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        has_touch: Option<bool>,
    },
    EmulateMedia {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color_scheme: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reduced_motion: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        forced_colors: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contrast: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<String>,
    },
    SetLocale {
        locale: String,
    },
    SetTimezone {
        timezone: String,
    },
    SetUserAgent {
        user_agent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accept_language: Option<String>,
    },
    SetGeolocation {
        latitude: f64,
        longitude: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accuracy: Option<f64>,
    },
    SetOffline {
        offline: bool,
    },
    SetExtraHttpHeaders {
        headers: BTreeMap<String, String>,
    },
    GetCookies {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        urls: Option<Vec<String>>,
    },
    SetCookies {
        cookies: Vec<BrowserCookie>,
    },
    ClearCookies {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        domain: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    GetStorage {
        kind: BrowserStorageKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<String>,
    },
    SetStorage {
        kind: BrowserStorageKind,
        items: Vec<BrowserStorageItem>,
    },
    ClearStorage {
        kind: BrowserStorageKind,
    },
    StorageState {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_as: Option<String>,
    },
    SetStorageState {
        state: BrowserStorageState,
    },
    GrantPermissions {
        permissions: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<String>,
    },
    ClearPermissions,
    SetHttpCredentials {
        username: String,
        password: String,
    },

    Expect {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locator: Option<BrowserLocator>,
        matcher: BrowserExpectation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default)]
        soft: bool,
    },

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
    DragAndDrop {
        source: BrowserLocator,
        target: BrowserLocator,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_position: Option<BrowserPosition>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_position: Option<BrowserPosition>,
    },
    DropFiles {
        target: BrowserLocator,
        paths: Vec<String>,
    },
    MouseMove {
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<usize>,
    },
    MouseDown {
        #[serde(default)]
        button: BrowserMouseButton,
    },
    MouseUp {
        #[serde(default)]
        button: BrowserMouseButton,
    },
    MouseClickXy {
        x: f64,
        y: f64,
        #[serde(default)]
        button: BrowserMouseButton,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        click_count: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delay: Option<u64>,
    },
    MouseDragXy {
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
    },
    MouseWheel {
        delta_x: f64,
        delta_y: f64,
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
        #[serde(default)]
        limit: Option<usize>,
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
    Screenshot {
        #[serde(flatten)]
        options: BrowserScreenshotOptions,
    },
    ScreenshotElement {
        locator: BrowserLocator,
        #[serde(flatten)]
        options: BrowserScreenshotOptions,
    },
    Pdf {
        #[serde(flatten)]
        options: BrowserPdfOptions,
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
    Highlight {
        locator: BrowserLocator,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    HideHighlight,
    Annotate {
        locator: BrowserLocator,
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocatorHandlerAction {
    Click,
    Steps { steps: Vec<BrowserStep> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RouteHandler {
    Fulfill {
        status: u16,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_type: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        body_base64: bool,
    },
    Abort {
        reason: String,
    },
    Continue {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        method: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        headers: Option<BTreeMap<String, String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        post_data: Option<String>,
    },
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
    Regex {
        source: String,
        #[serde(default)]
        flags: String,
    },
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl BrowserStep {
    pub const ACTION_NAMES: &'static [&'static str] = &[
        "navigate",
        "reload",
        "go_back",
        "go_forward",
        "open_tab",
        "close_tab",
        "switch_tab",
        "list_tabs",
        "wait_for_popup",
        "route",
        "unroute",
        "list_routes",
        "route_web_socket",
        "unroute_web_socket",
        "send_web_socket_message",
        "wait_for_web_socket_frame",
        "start_har_recording",
        "stop_har_recording",
        "route_from_har",
        "start_coverage",
        "stop_coverage",
        "add_virtual_authenticator",
        "remove_virtual_authenticator",
        "list_credentials",
        "add_credential",
        "clear_credentials",
        "set_user_verified",
        "set_viewport",
        "emulate_media",
        "set_locale",
        "set_timezone",
        "set_user_agent",
        "set_geolocation",
        "set_offline",
        "set_extra_http_headers",
        "get_cookies",
        "set_cookies",
        "clear_cookies",
        "get_storage",
        "set_storage",
        "clear_storage",
        "storage_state",
        "set_storage_state",
        "grant_permissions",
        "clear_permissions",
        "set_http_credentials",
        "expect",
        "click",
        "click_if_exists",
        "hover",
        "focus",
        "blur",
        "scroll_to",
        "press_key",
        "drag_and_drop",
        "drop_files",
        "mouse_move",
        "mouse_down",
        "mouse_up",
        "mouse_click_xy",
        "mouse_drag_xy",
        "mouse_wheel",
        "fill",
        "clear",
        "select_option",
        "check",
        "uncheck",
        "set_input_files",
        "expect_file_chooser",
        "wait_for_selector",
        "wait_for_navigation",
        "wait_for_url",
        "wait_for_text",
        "wait_for_network_idle",
        "wait_for_load_state",
        "wait_for_request",
        "wait_for_response",
        "wait_for_download",
        "wait_for_element_hidden",
        "wait_for_element_stable",
        "wait_seconds",
        "get_text",
        "get_html",
        "get_attribute",
        "extract_links",
        "extract_table",
        "dom_snapshot",
        "accessibility_snapshot",
        "screenshot",
        "screenshot_element",
        "pdf",
        "eval",
        "styles",
        "tab_log",
        "add_locator_handler",
        "remove_locator_handler",
        "handle_dialog",
        "dismiss_overlays",
        "highlight_element",
        "highlight",
        "hide_highlight",
        "annotate",
    ];
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
    pub attach_screenshot: Option<bool>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserAssertionResult {
    pub matcher: String,
    pub passed: bool,
    pub soft: bool,
    pub expected: serde_json::Value,
    pub received: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    pub attempts: u32,
    pub elapsed_ms: u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assertion: Option<BrowserAssertionResult>,
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
            assertion: None,
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
            assertion: None,
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
    pub websockets: Vec<WebSocketEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locator_handlers: Vec<LocatorHandlerFiring>,
    pub dialogs: Vec<DialogInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uploads: Vec<UploadInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downloads: Vec<DownloadInfo>,
    pub new_tabs: Vec<TabInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_routes: Vec<RouteInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intercepted_requests: Vec<RouteInterception>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<BrowserContextSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<BrowserScreenshot>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserContextSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub cookie_count: usize,
    #[serde(default)]
    pub local_storage_count: usize,
    #[serde(default)]
    pub session_storage_count: usize,
    #[serde(default)]
    pub offline: bool,
    #[serde(default)]
    pub http_credentials: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserStorageKind {
    Local,
    Session,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserStorageItem {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BrowserCookieSameSite {
    Strict,
    Lax,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default = "default_cookie_path")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<f64>,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub secure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_site: Option<BrowserCookieSameSite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

fn default_cookie_path() -> String {
    "/".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserStorageOrigin {
    pub origin: String,
    #[serde(default)]
    pub local_storage: Vec<BrowserStorageItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BrowserStorageState {
    #[serde(default)]
    pub cookies: Vec<BrowserCookie>,
    #[serde(default)]
    pub origins: Vec<BrowserStorageOrigin>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteInfo {
    pub pattern: UrlPattern,
    pub handler: RouteHandler,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteInterception {
    pub url: String,
    pub method: String,
    pub pattern: UrlPattern,
    pub action: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub request_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_body_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub redirect_hop: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TabOpener {
    pub tab_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TabInfo {
    pub id: String,
    pub target_id: String,
    pub url: String,
    pub title: String,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opener: Option<TabOpener>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_by_step: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_cookie_and_storage_state_serde_round_trip() {
        let state = BrowserStorageState {
            cookies: vec![BrowserCookie {
                name: "session".to_string(),
                value: "secret".to_string(),
                domain: "example.test".to_string(),
                path: "/".to_string(),
                expires: Some(1_900_000_000.0),
                http_only: true,
                secure: true,
                same_site: Some(BrowserCookieSameSite::Lax),
                url: None,
            }],
            origins: vec![BrowserStorageOrigin {
                origin: "https://example.test".to_string(),
                local_storage: vec![BrowserStorageItem {
                    name: "logged_in".to_string(),
                    value: "true".to_string(),
                }],
            }],
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["origins"][0]["local_storage"][0]["name"], "logged_in");
        assert_eq!(
            serde_json::from_value::<BrowserStorageState>(json).unwrap(),
            state
        );
    }

    #[test]
    fn context_step_serde_round_trip_masks_nothing_in_typed_input() {
        let step = BrowserStep::SetHttpCredentials {
            username: "user".to_string(),
            password: "secret".to_string(),
        };
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["action"], "set_http_credentials");
        assert!(matches!(
            serde_json::from_value::<BrowserStep>(json).unwrap(),
            BrowserStep::SetHttpCredentials { .. }
        ));
    }

    fn all_browser_steps() -> Vec<BrowserStep> {
        let locator = || BrowserLocator::reference("e1");
        vec![
            BrowserStep::Navigate {
                url: "https://example.com".to_string(),
            },
            BrowserStep::Reload,
            BrowserStep::GoBack,
            BrowserStep::GoForward,
            BrowserStep::OpenTab {
                device: Some("desktop".to_string()),
                url: Some("https://example.com/new".to_string()),
            },
            BrowserStep::CloseTab { tab: None },
            BrowserStep::SwitchTab {
                tab: TabTarget::Active,
            },
            BrowserStep::ListTabs,
            BrowserStep::WaitForPopup {
                timeout_ms: Some(1_000),
            },
            BrowserStep::Route {
                pattern: UrlPattern::Text("**/api/**".to_string()),
                handler: RouteHandler::Abort {
                    reason: "blockedbyclient".to_string(),
                },
            },
            BrowserStep::Unroute { pattern: None },
            BrowserStep::ListRoutes,
            BrowserStep::RouteWebSocket {
                pattern: UrlPattern::Text("wss://example.com/**".to_string()),
                mode: WebSocketRouteMode::Mock,
            },
            BrowserStep::UnrouteWebSocket { pattern: None },
            BrowserStep::SendWebSocketMessage {
                url_pattern: UrlPattern::Text("wss://example.com/**".to_string()),
                data: "hello".to_string(),
            },
            BrowserStep::WaitForWebSocketFrame {
                pattern: None,
                timeout_ms: Some(1_000),
            },
            BrowserStep::StartHarRecording {
                path: Some("page.har".to_string()),
                mode: HarMode::Full,
                content: HarContentPolicy::Embed,
                url_filter: None,
            },
            BrowserStep::StopHarRecording,
            BrowserStep::RouteFromHar {
                path: "page.har".to_string(),
                url_filter: None,
                not_found: HarNotFound::Abort,
            },
            BrowserStep::StartCoverage {
                js: Some(true),
                css: Some(true),
                reset_on_navigation: Some(true),
            },
            BrowserStep::StopCoverage,
            BrowserStep::AddVirtualAuthenticator {
                protocol: Some(BrowserAuthenticatorProtocol::Ctap2),
                transport: Some(BrowserAuthenticatorTransport::Usb),
                has_resident_key: Some(true),
                has_user_verification: Some(true),
                is_user_verified: Some(true),
            },
            BrowserStep::RemoveVirtualAuthenticator {
                id: "authenticator".to_string(),
            },
            BrowserStep::ListCredentials {
                id: "authenticator".to_string(),
            },
            BrowserStep::AddCredential {
                id: "authenticator".to_string(),
                credential: BrowserWebAuthnCredential {
                    credential_id: "credential".to_string(),
                    is_resident_credential: true,
                    rp_id: Some("example.com".to_string()),
                    private_key: "private".to_string(),
                    user_handle: Some("user".to_string()),
                    sign_count: 0,
                    large_blob: None,
                    backup_eligibility: None,
                    backup_state: None,
                    user_name: None,
                    user_display_name: None,
                },
            },
            BrowserStep::ClearCredentials {
                id: "authenticator".to_string(),
            },
            BrowserStep::SetUserVerified {
                id: "authenticator".to_string(),
                verified: true,
            },
            BrowserStep::SetViewport {
                width: 390,
                height: 844,
                device_scale_factor: Some(3.0),
                is_mobile: Some(true),
                has_touch: Some(true),
            },
            BrowserStep::EmulateMedia {
                color_scheme: Some("dark".to_string()),
                reduced_motion: None,
                forced_colors: None,
                contrast: None,
                media: None,
            },
            BrowserStep::SetLocale {
                locale: "ja-JP".to_string(),
            },
            BrowserStep::SetTimezone {
                timezone: "Asia/Tokyo".to_string(),
            },
            BrowserStep::SetUserAgent {
                user_agent: "agent".to_string(),
                accept_language: None,
            },
            BrowserStep::SetGeolocation {
                latitude: 1.0,
                longitude: 2.0,
                accuracy: None,
            },
            BrowserStep::SetOffline { offline: false },
            BrowserStep::SetExtraHttpHeaders {
                headers: BTreeMap::new(),
            },
            BrowserStep::GetCookies { urls: None },
            BrowserStep::SetCookies {
                cookies: Vec::new(),
            },
            BrowserStep::ClearCookies {
                name: None,
                domain: None,
                path: None,
            },
            BrowserStep::GetStorage {
                kind: BrowserStorageKind::Local,
                origin: None,
            },
            BrowserStep::SetStorage {
                kind: BrowserStorageKind::Local,
                items: Vec::new(),
            },
            BrowserStep::ClearStorage {
                kind: BrowserStorageKind::Session,
            },
            BrowserStep::StorageState {
                save_as: Some("auth.json".to_string()),
            },
            BrowserStep::SetStorageState {
                state: BrowserStorageState::default(),
            },
            BrowserStep::GrantPermissions {
                permissions: vec!["geolocation".to_string()],
                origin: None,
            },
            BrowserStep::ClearPermissions,
            BrowserStep::SetHttpCredentials {
                username: "user".to_string(),
                password: "secret".to_string(),
            },
            BrowserStep::Expect {
                locator: Some(locator()),
                matcher: BrowserExpectation::ToBeVisible,
                timeout_ms: Some(1_000),
                soft: false,
            },
            BrowserStep::Click { locator: locator() },
            BrowserStep::ClickIfExists { locator: locator() },
            BrowserStep::Hover { locator: locator() },
            BrowserStep::Focus { locator: locator() },
            BrowserStep::Blur { locator: locator() },
            BrowserStep::ScrollTo { locator: locator() },
            BrowserStep::PressKey {
                key: "Enter".to_string(),
                modifiers: vec!["Ctrl".to_string()],
            },
            BrowserStep::DragAndDrop {
                source: locator(),
                target: locator(),
                source_position: Some(BrowserPosition { x: 1.0, y: 2.0 }),
                target_position: Some(BrowserPosition { x: 3.0, y: 4.0 }),
            },
            BrowserStep::DropFiles {
                target: locator(),
                paths: vec!["/tmp/file".to_string()],
            },
            BrowserStep::MouseMove {
                x: 10.0,
                y: 20.0,
                steps: Some(2),
            },
            BrowserStep::MouseDown {
                button: BrowserMouseButton::Left,
            },
            BrowserStep::MouseUp {
                button: BrowserMouseButton::Left,
            },
            BrowserStep::MouseClickXy {
                x: 10.0,
                y: 20.0,
                button: BrowserMouseButton::Left,
                click_count: Some(2),
                delay: Some(10),
            },
            BrowserStep::MouseDragXy {
                start_x: 1.0,
                start_y: 2.0,
                end_x: 3.0,
                end_y: 4.0,
            },
            BrowserStep::MouseWheel {
                delta_x: 0.0,
                delta_y: 100.0,
            },
            BrowserStep::Fill {
                locator: locator(),
                text: "hi".to_string(),
                clear_first: true,
                verify: true,
            },
            BrowserStep::Clear {
                locator: locator(),
                verify: true,
            },
            BrowserStep::SelectOption {
                locator: locator(),
                value: "one".to_string(),
            },
            BrowserStep::Check { locator: locator() },
            BrowserStep::Uncheck { locator: locator() },
            BrowserStep::SetInputFiles {
                locator: locator(),
                paths: vec!["/tmp/file".to_string()],
            },
            BrowserStep::ExpectFileChooser {
                paths: vec!["/tmp/file".to_string()],
            },
            BrowserStep::WaitForSelector {
                locator: locator(),
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForNavigation {
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForUrl {
                contains: "/done".to_string(),
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForText {
                text: "done".to_string(),
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForNetworkIdle {
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForLoadState {
                state: BrowserLoadState::Load,
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForRequest {
                url_or_pattern: UrlPattern::Text("/api".to_string()),
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForResponse {
                url_or_pattern: UrlPattern::Regex {
                    source: "/api".to_string(),
                    flags: "i".to_string(),
                },
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForDownload {
                timeout_ms: Some(1_000),
                save_as: Some("file.txt".to_string()),
            },
            BrowserStep::WaitForElementHidden {
                locator: locator(),
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitForElementStable {
                locator: locator(),
                timeout_ms: Some(1_000),
            },
            BrowserStep::WaitSeconds { seconds: 0.1 },
            BrowserStep::GetText { locator: locator() },
            BrowserStep::GetHtml { locator: locator() },
            BrowserStep::GetAttribute {
                locator: locator(),
                attribute: "href".to_string(),
            },
            BrowserStep::ExtractLinks {
                locator: Some(locator()),
                limit: Some(10),
            },
            BrowserStep::ExtractTable {
                locator: locator(),
                limit: Some(10),
            },
            BrowserStep::DomSnapshot {
                selector: "main".to_string(),
                max_chars: Some(1_000),
            },
            BrowserStep::AccessibilitySnapshot {
                options: AccessibilitySnapshotOptions::default(),
            },
            BrowserStep::Screenshot {
                options: BrowserScreenshotOptions::default(),
            },
            BrowserStep::ScreenshotElement {
                locator: locator(),
                options: BrowserScreenshotOptions::default(),
            },
            BrowserStep::Pdf {
                options: BrowserPdfOptions::default(),
            },
            BrowserStep::Eval {
                expression: "document.title".to_string(),
            },
            BrowserStep::Styles {
                locator: locator(),
                property_filter: Some("color".to_string()),
            },
            BrowserStep::TabLog,
            BrowserStep::AddLocatorHandler {
                name: "overlay".to_string(),
                locator: locator(),
                handler: LocatorHandlerAction::Click,
                times: Some(1),
                no_wait_after: false,
            },
            BrowserStep::RemoveLocatorHandler {
                name: "overlay".to_string(),
            },
            BrowserStep::HandleDialog {
                accept: true,
                prompt_text: Some("answer".to_string()),
            },
            BrowserStep::DismissOverlays,
            BrowserStep::HighlightElement { locator: locator() },
            BrowserStep::Highlight {
                locator: locator(),
                style: Some("outline: 2px solid red".to_string()),
                label: Some("Target".to_string()),
            },
            BrowserStep::HideHighlight,
            BrowserStep::Annotate {
                locator: locator(),
                text: "Review".to_string(),
            },
        ]
    }

    #[test]
    fn browser_step_action_names_cover_every_variant() {
        let serialized = all_browser_steps()
            .into_iter()
            .map(|step| {
                serde_json::to_value(step).unwrap()["action"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<std::collections::BTreeSet<_>>();
        let declared = BrowserStep::ACTION_NAMES
            .iter()
            .map(|action| action.to_string())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(declared, serialized);
    }

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
    fn frame_chain_round_trips_outermost_first() {
        let locator = BrowserLocator::role("button", Some("Save")).in_frames(vec![
            BrowserLocator::css("#outer"),
            BrowserLocator::role("iframe", Some("Editor")),
        ]);
        let value = serde_json::to_value(&locator).unwrap();

        assert_eq!(
            value["frames"],
            serde_json::json!([
                {"by": "css", "value": "#outer"},
                {"by": "role", "role": "iframe", "name": "Editor"}
            ])
        );
        assert_eq!(
            serde_json::from_value::<BrowserLocator>(value).unwrap(),
            locator
        );
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

        let request: BrowserStep = serde_json::from_str(
            r#"{"action":"wait_for_request","url_or_pattern":{"source":"/api/.*"}}"#,
        )
        .unwrap();
        assert!(matches!(
            request,
            BrowserStep::WaitForRequest {
                url_or_pattern: UrlPattern::Regex { source, flags },
                timeout_ms: None
            } if source == "/api/.*" && flags.is_empty()
        ));
    }

    #[test]
    fn route_handlers_and_patternless_unroute_round_trip() {
        for value in [
            serde_json::json!({
                "action": "route",
                "pattern": "**/api/**",
                "handler": {
                    "type": "fulfill",
                    "status": 200,
                    "content_type": "application/json",
                    "body": "{\"ok\":true}",
                    "headers": {"X-Mocked": "yes"}
                }
            }),
            serde_json::json!({
                "action": "route",
                "pattern": "**/*.png",
                "handler": {"type": "abort", "reason": "blockedbyclient"}
            }),
            serde_json::json!({
                "action": "route",
                "pattern": {"source": "/api/", "flags": "i"},
                "handler": {
                    "type": "continue",
                    "url": "https://example.com/api/other",
                    "method": "POST",
                    "headers": {"X-Test": "route"},
                    "post_data": "password=secret"
                }
            }),
            serde_json::json!({"action": "unroute"}),
        ] {
            let step: BrowserStep = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(step).unwrap(), value);
        }
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
    fn test_step_extract_table_serde() {
        let json_str = r#"{"action": "extract_table", "locator": {"by": "css", "value": "table"}, "limit": 4}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::ExtractTable { limit, .. } => assert_eq!(limit, Some(4)),
            _ => panic!("Expected ExtractTable"),
        }
    }

    #[test]
    fn test_step_extract_table_limit_defaults_to_none() {
        let json_str = r#"{"action": "extract_table", "locator": {"by": "css", "value": "table"}}"#;
        let step: BrowserStep = serde_json::from_str(json_str).unwrap();
        match step {
            BrowserStep::ExtractTable { limit, .. } => assert_eq!(limit, None),
            _ => panic!("Expected ExtractTable"),
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
        assert!(matches!(step, BrowserStep::Screenshot { .. }));
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
        assert!(req.attach_screenshot.is_none());
    }

    #[test]
    fn test_request_attach_screenshot_is_tri_state() {
        let omitted: BrowserActionRequest =
            serde_json::from_str(r#"{"steps": []}"#).unwrap();
        assert_eq!(omitted.attach_screenshot, None);

        let enabled: BrowserActionRequest =
            serde_json::from_str(r#"{"attach_screenshot": true, "steps": []}"#).unwrap();
        assert_eq!(enabled.attach_screenshot, Some(true));

        let suppressed: BrowserActionRequest =
            serde_json::from_str(r#"{"attach_screenshot": false, "steps": []}"#).unwrap();
        assert_eq!(suppressed.attach_screenshot, Some(false));
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
            websockets: vec![],
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
            new_tabs: vec![],
            active_routes: vec![],
            intercepted_requests: vec![],
            context: None,
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
            id: "1".to_string(),
            target_id: "ABC123".to_string(),
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            active: true,
            opener: Some(TabOpener {
                tab_id: "opener".to_string(),
                frame_id: Some("frame".to_string()),
            }),
            opened_by_step: Some(2),
        };
        let json = serde_json::to_value(&ti).unwrap();
        assert_eq!(json["id"], "1");
        assert!(json["active"].as_bool().unwrap());
        assert_eq!(json["opener"]["tab_id"], "opener");
    }
}
