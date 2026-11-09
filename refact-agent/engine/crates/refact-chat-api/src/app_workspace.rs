use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use uuid::Uuid;

pub const APP_WORKSPACE_SCHEMA_VERSION: u32 = 1;
pub const APP_WORKSPACE_MAX_DEPTH: usize = 6;
pub const APP_WORKSPACE_MAX_LEAVES: usize = 32;
pub const APP_WORKSPACE_MAX_BINDINGS: usize = 64;
pub const APP_WORKSPACE_MAX_SPLIT_CHILDREN: usize = 8;
pub const APP_WORKSPACE_MAX_LEAF_TABS: usize = 16;
pub const APP_WORKSPACE_MAX_BINDING_BYTES: usize = 64 * 1024;
pub const APP_WORKSPACE_MAX_TOTAL_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppWorkspace {
    pub schema_version: u32,
    #[serde(default)]
    pub revision: u64,
    pub bindings: Vec<AppBinding>,
    pub layout: AppLayoutNode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppBinding {
    pub binding_id: Uuid,
    pub app_id: Uuid,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub state: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppLayoutNode {
    Split {
        direction: SplitDirection,
        sizes: Vec<f64>,
        children: Vec<AppLayoutNode>,
    },
    Leaf {
        tabs: Vec<AppTab>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active: Option<usize>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatTabRef {
    #[serde(rename = "self")]
    SelfChat,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppTab {
    Chat {
        #[serde(rename = "ref")]
        reference: ChatTabRef,
    },
    App {
        binding_id: Uuid,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppWorkspaceSetError {
    Conflict { current_revision: u64 },
    Invalid(String),
    Unavailable(String),
}

impl AppWorkspaceSetError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Conflict {
                ..
            } => "app_workspace_revision_conflict",
            Self::Invalid(_) => "invalid_app_workspace",
            Self::Unavailable(_) => "app_workspace_unavailable",
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Conflict {
                current_revision,
            } => serde_json::json!({
                "error": self.code(),
                "current_revision": current_revision,
            }),
            Self::Invalid(reason) | Self::Unavailable(reason) => serde_json::json!({
                "error": format!("{}: {}", self.code(), reason),
            }),
        }
    }
}

impl std::fmt::Display for AppWorkspaceSetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict {
                current_revision,
            } => {
                write!(f, "{} (current_revision={})", self.code(), current_revision)
            }
            Self::Invalid(reason) | Self::Unavailable(reason) => {
                write!(f, "{}: {}", self.code(), reason)
            }
        }
    }
}

impl std::error::Error for AppWorkspaceSetError {}

impl AppWorkspace {
    pub fn empty() -> Self {
        Self {
            schema_version: APP_WORKSPACE_SCHEMA_VERSION,
            revision: 0,
            bindings: Vec::new(),
            layout: AppLayoutNode::Leaf {
                tabs: vec![AppTab::Chat {
                    reference: ChatTabRef::SelfChat,
                }],
                active: Some(0),
            },
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != APP_WORKSPACE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported schema_version {}",
                self.schema_version
            ));
        }
        if self.bindings.len() > APP_WORKSPACE_MAX_BINDINGS {
            return Err(format!(
                "too many bindings ({} > {})",
                self.bindings.len(),
                APP_WORKSPACE_MAX_BINDINGS
            ));
        }
        let mut binding_ids = HashSet::new();
        for binding in &self.bindings {
            if !binding_ids.insert(binding.binding_id) {
                return Err(format!("duplicate binding_id {}", binding.binding_id));
            }
            let bytes = serde_json::to_vec(&binding.params)
                .map(|v| v.len())
                .unwrap_or(usize::MAX)
                .saturating_add(
                    serde_json::to_vec(&binding.state)
                        .map(|v| v.len())
                        .unwrap_or(usize::MAX),
                );
            if bytes > APP_WORKSPACE_MAX_BINDING_BYTES {
                return Err(format!(
                    "binding {} params/state exceed {} bytes",
                    binding.binding_id, APP_WORKSPACE_MAX_BINDING_BYTES
                ));
            }
        }
        let mut walk = LayoutWalk {
            binding_ids: &binding_ids,
            referenced: HashSet::new(),
            chat_tabs: 0,
            leaves: 0,
        };
        walk.node(&self.layout, 1)?;
        let total = serde_json::to_vec(self)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if total > APP_WORKSPACE_MAX_TOTAL_BYTES {
            return Err(format!(
                "workspace exceeds {} bytes",
                APP_WORKSPACE_MAX_TOTAL_BYTES
            ));
        }
        Ok(())
    }

    pub fn fork_with_new_binding_ids(&self) -> Self {
        let mapping: HashMap<Uuid, Uuid> = self
            .bindings
            .iter()
            .map(|binding| (binding.binding_id, Uuid::new_v4()))
            .collect();
        let bindings = self
            .bindings
            .iter()
            .map(|binding| AppBinding {
                binding_id: mapping[&binding.binding_id],
                ..binding.clone()
            })
            .collect();
        Self {
            schema_version: self.schema_version,
            revision: 1,
            bindings,
            layout: remap_layout(&self.layout, &mapping),
        }
    }
}

fn remap_layout(node: &AppLayoutNode, mapping: &HashMap<Uuid, Uuid>) -> AppLayoutNode {
    match node {
        AppLayoutNode::Split {
            direction,
            sizes,
            children,
        } => AppLayoutNode::Split {
            direction: *direction,
            sizes: sizes.clone(),
            children: children
                .iter()
                .map(|child| remap_layout(child, mapping))
                .collect(),
        },
        AppLayoutNode::Leaf {
            tabs,
            active,
        } => AppLayoutNode::Leaf {
            tabs: tabs
                .iter()
                .map(|tab| match tab {
                    AppTab::App {
                        binding_id,
                    } => AppTab::App {
                        binding_id: mapping.get(binding_id).copied().unwrap_or(*binding_id),
                    },
                    other => other.clone(),
                })
                .collect(),
            active: *active,
        },
    }
}

struct LayoutWalk<'a> {
    binding_ids: &'a HashSet<Uuid>,
    referenced: HashSet<Uuid>,
    chat_tabs: usize,
    leaves: usize,
}

impl LayoutWalk<'_> {
    fn node(&mut self, node: &AppLayoutNode, depth: usize) -> Result<(), String> {
        if depth > APP_WORKSPACE_MAX_DEPTH {
            return Err(format!("layout deeper than {}", APP_WORKSPACE_MAX_DEPTH));
        }
        match node {
            AppLayoutNode::Split {
                sizes,
                children,
                ..
            } => {
                if children.len() < 2 || children.len() > APP_WORKSPACE_MAX_SPLIT_CHILDREN {
                    return Err(format!(
                        "split must have 2..={} children",
                        APP_WORKSPACE_MAX_SPLIT_CHILDREN
                    ));
                }
                if sizes.len() != children.len() {
                    return Err("split sizes must match children".to_string());
                }
                if sizes.iter().any(|size| !size.is_finite() || *size <= 0.0) {
                    return Err("split sizes must be positive finite numbers".to_string());
                }
                for child in children {
                    self.node(child, depth + 1)?;
                }
            }
            AppLayoutNode::Leaf {
                tabs,
                active,
            } => {
                self.leaves += 1;
                if self.leaves > APP_WORKSPACE_MAX_LEAVES {
                    return Err(format!(
                        "layout has more than {} leaves",
                        APP_WORKSPACE_MAX_LEAVES
                    ));
                }
                if tabs.is_empty() || tabs.len() > APP_WORKSPACE_MAX_LEAF_TABS {
                    return Err(format!(
                        "leaf must have 1..={} tabs",
                        APP_WORKSPACE_MAX_LEAF_TABS
                    ));
                }
                if active.is_some_and(|index| index >= tabs.len()) {
                    return Err("leaf active index out of range".to_string());
                }
                for tab in tabs {
                    match tab {
                        AppTab::Chat {
                            ..
                        } => {
                            self.chat_tabs += 1;
                            if self.chat_tabs > 1 {
                                return Err("chat tab may appear at most once".to_string());
                            }
                        }
                        AppTab::App {
                            binding_id,
                        } => {
                            if !self.binding_ids.contains(binding_id) {
                                return Err(format!("tab references unknown binding {binding_id}"));
                            }
                            if !self.referenced.insert(*binding_id) {
                                return Err(format!(
                                    "binding {binding_id} appears in more than one tab"
                                ));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// A persisted workspace. Anything that is not a valid schema-1 workspace is kept
/// verbatim as `Unsupported`: it is never interpreted, and it roundtrips unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum StoredAppWorkspace {
    Supported(AppWorkspace),
    Unsupported(Value),
}

impl StoredAppWorkspace {
    pub fn from_value(value: Value) -> Self {
        match serde_json::from_value::<AppWorkspace>(value.clone()) {
            Ok(workspace) if workspace.validate().is_ok() => Self::Supported(workspace),
            _ => Self::Unsupported(value),
        }
    }

    pub fn revision(&self) -> u64 {
        match self {
            Self::Supported(workspace) => workspace.revision,
            Self::Unsupported(value) => value.get("revision").and_then(Value::as_u64).unwrap_or(0),
        }
    }

    pub fn supported(&self) -> Option<&AppWorkspace> {
        match self {
            Self::Supported(workspace) => Some(workspace),
            Self::Unsupported(_) => None,
        }
    }

    /// New chats get fresh binding ids; unsupported data is never carried over.
    pub fn fork_for_new_chat(&self) -> Option<Self> {
        self.supported()
            .map(|workspace| Self::Supported(workspace.fork_with_new_binding_ids()))
    }
}

impl Serialize for StoredAppWorkspace {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Supported(workspace) => workspace.serialize(serializer),
            Self::Unsupported(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for StoredAppWorkspace {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(Self::from_value)
    }
}

pub fn app_workspace_revision(current: Option<&StoredAppWorkspace>) -> u64 {
    current.map(StoredAppWorkspace::revision).unwrap_or(0)
}

/// Compare-and-set. On any error `current` is left untouched. The stored revision
/// is assigned by the server as `current + 1`; the caller's `revision` is ignored.
pub fn apply_app_workspace_cas(
    current: &mut Option<StoredAppWorkspace>,
    expected_revision: u64,
    mut workspace: AppWorkspace,
) -> Result<AppWorkspace, AppWorkspaceSetError> {
    if matches!(current, Some(StoredAppWorkspace::Unsupported(_))) {
        return Err(AppWorkspaceSetError::Unavailable(
            "unsupported workspace schema is read-only".to_string(),
        ));
    }
    let current_revision = app_workspace_revision(current.as_ref());
    if expected_revision != current_revision {
        return Err(AppWorkspaceSetError::Conflict {
            current_revision,
        });
    }
    workspace
        .validate()
        .map_err(AppWorkspaceSetError::Invalid)?;
    workspace.revision = current_revision
        .checked_add(1)
        .ok_or_else(|| AppWorkspaceSetError::Invalid("revision overflow".to_string()))?;
    *current = Some(StoredAppWorkspace::Supported(workspace.clone()));
    Ok(workspace)
}

/// Picks the workspace to persist when two writers race: the higher revision wins,
/// ties keep `preferred`. A missing value never replaces a present one.
pub fn newest_app_workspace(
    preferred: Option<StoredAppWorkspace>,
    other: Option<StoredAppWorkspace>,
) -> Option<StoredAppWorkspace> {
    match (preferred, other) {
        (Some(preferred), Some(other)) if other.revision() > preferred.revision() => Some(other),
        (Some(preferred), _) => Some(preferred),
        (None, other) => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const APP: &str = "6f1c1d2e-0000-4000-8000-000000000001";
    const B1: &str = "6f1c1d2e-0000-4000-8000-0000000000b1";
    const B2: &str = "6f1c1d2e-0000-4000-8000-0000000000b2";

    fn populated(revision: u64) -> AppWorkspace {
        serde_json::from_value(json!({
            "schema_version": 1,
            "revision": revision,
            "bindings": [
                {"binding_id": B1, "app_id": APP, "params": {"q": 1}, "state": {"page": 2}},
                {"binding_id": B2, "app_id": APP}
            ],
            "layout": {
                "kind": "split", "direction": "horizontal", "sizes": [1.0, 2.0],
                "children": [
                    {"kind": "leaf", "tabs": [{"kind": "chat", "ref": "self"}]},
                    {"kind": "leaf", "tabs": [
                        {"kind": "app", "binding_id": B1},
                        {"kind": "app", "binding_id": B2}
                    ], "active": 1}
                ]
            }
        }))
        .unwrap()
    }

    #[test]
    fn cas_assigns_monotonic_revision_and_stale_or_invalid_writes_change_nothing() {
        let mut current = None;
        let first = apply_app_workspace_cas(&mut current, 0, populated(77)).unwrap();
        assert_eq!(first.revision, 1);
        let snapshot = current.clone();

        let stale = apply_app_workspace_cas(&mut current, 0, AppWorkspace::empty());
        assert_eq!(
            stale,
            Err(AppWorkspaceSetError::Conflict {
                current_revision: 1
            })
        );
        assert_eq!(current, snapshot);

        let mut invalid = populated(0);
        invalid.bindings.pop();
        assert!(matches!(
            apply_app_workspace_cas(&mut current, 1, invalid),
            Err(AppWorkspaceSetError::Invalid(_))
        ));
        assert_eq!(current, snapshot);

        let cleared = apply_app_workspace_cas(&mut current, 1, AppWorkspace::empty()).unwrap();
        assert_eq!(cleared.revision, 2);
        assert!(cleared.bindings.is_empty());
        assert_eq!(app_workspace_revision(current.as_ref()), 2);
        assert_eq!(
            apply_app_workspace_cas(&mut current, 0, populated(0)),
            Err(AppWorkspaceSetError::Conflict {
                current_revision: 2
            })
        );
    }

    #[test]
    fn unsupported_or_malformed_workspaces_roundtrip_verbatim_and_never_fork() {
        let future = json!({
            "schema_version": 2,
            "revision": 9,
            "bindings": [{"binding_id": B1, "app_id": APP, "grants": ["fs"]}],
            "layout": {"kind": "grid"}
        });
        let mut too_deep = json!({"kind": "leaf", "tabs": [{"kind": "chat", "ref": "self"}]});
        for _ in 0..APP_WORKSPACE_MAX_DEPTH {
            too_deep = json!({"kind": "split", "direction": "vertical", "sizes": [1.0, 1.0],
                "children": [too_deep, {"kind": "leaf", "tabs": [{"kind": "app", "binding_id": B1}]}]});
        }
        let deep = json!({"schema_version": 1, "revision": 4,
            "bindings": [{"binding_id": B1, "app_id": APP}], "layout": too_deep});
        for raw in [future, deep] {
            let stored: StoredAppWorkspace = serde_json::from_value(raw.clone()).unwrap();
            assert!(stored.supported().is_none());
            assert_eq!(serde_json::to_value(&stored).unwrap(), raw);
            assert!(stored.fork_for_new_chat().is_none());
        }
        let mut current = Some(StoredAppWorkspace::from_value(
            json!({"schema_version": 2, "revision": 9}),
        ));
        let before = current.clone();
        assert!(matches!(
            apply_app_workspace_cas(&mut current, 9, AppWorkspace::empty()),
            Err(AppWorkspaceSetError::Unavailable(_))
        ));
        assert_eq!(current, before);
    }

    #[test]
    fn fork_for_new_chat_issues_fresh_binding_ids_and_remaps_layout() {
        let source = StoredAppWorkspace::Supported(populated(5));
        let forked = source.fork_for_new_chat().unwrap();
        let forked = forked.supported().unwrap();
        forked.validate().unwrap();
        assert_eq!(forked.revision, 1);
        let old: HashSet<Uuid> = [B1, B2].iter().map(|id| id.parse().unwrap()).collect();
        let new: Vec<Uuid> = forked.bindings.iter().map(|b| b.binding_id).collect();
        assert_eq!(new.len(), 2);
        assert!(new.iter().all(|id| !old.contains(id)));
        assert_ne!(new[0], new[1]);
        assert!(forked.bindings.iter().all(|b| b.app_id.to_string() == APP));
        assert_eq!(forked.bindings[0].params, populated(0).bindings[0].params);
        assert_eq!(forked.bindings[0].state, populated(0).bindings[0].state);
        let AppLayoutNode::Split {
            children, ..
        } = &forked.layout
        else {
            panic!("layout kind changed");
        };
        assert_eq!(
            children[0],
            AppLayoutNode::Leaf {
                tabs: vec![AppTab::Chat {
                    reference: ChatTabRef::SelfChat
                }],
                active: None
            }
        );
        let AppLayoutNode::Leaf {
            tabs,
            active,
        } = &children[1]
        else {
            panic!("leaf missing");
        };
        assert_eq!(*active, Some(1));
        assert_eq!(
            tabs,
            &vec![
                AppTab::App {
                    binding_id: new[0]
                },
                AppTab::App {
                    binding_id: new[1]
                }
            ]
        );
    }
}
