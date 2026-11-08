use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use headless_chrome::Tab;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::json;

use crate::{ElementHandle, HandleError, WorldManager};

const UNKNOWN_REF_PREFIX: &str = "REF_UNKNOWN:";
const DETACHED_REF_PREFIX: &str = "REF_DETACHED:";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ref {
    frame_sequence: Option<u64>,
    element_sequence: u64,
}

impl Ref {
    pub fn new(element_sequence: u64) -> Result<Self, RefParseError> {
        if element_sequence == 0 {
            return Err(RefParseError);
        }
        Ok(Self {
            frame_sequence: None,
            element_sequence,
        })
    }

    pub fn in_frame(frame_sequence: u64, element_sequence: u64) -> Result<Self, RefParseError> {
        if frame_sequence == 0 || element_sequence == 0 {
            return Err(RefParseError);
        }
        Ok(Self {
            frame_sequence: Some(frame_sequence),
            element_sequence,
        })
    }

    pub fn frame_sequence(&self) -> Option<u64> {
        self.frame_sequence
    }

    pub fn element_sequence(&self) -> u64 {
        self.element_sequence
    }
}

impl Display for Ref {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(frame_sequence) = self.frame_sequence {
            write!(formatter, "f{frame_sequence}e{}", self.element_sequence)
        } else {
            write!(formatter, "e{}", self.element_sequence)
        }
    }
}

impl FromStr for Ref {
    type Err = RefParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (frame_sequence, element_sequence) = if let Some(rest) = value.strip_prefix('f') {
            let (frame, element) = rest.split_once('e').ok_or(RefParseError)?;
            if frame.is_empty() || element.is_empty() {
                return Err(RefParseError);
            }
            (Some(parse_sequence(frame)?), parse_sequence(element)?)
        } else if let Some(element) = value.strip_prefix('e') {
            if element.is_empty() {
                return Err(RefParseError);
            }
            (None, parse_sequence(element)?)
        } else {
            return Err(RefParseError);
        };
        Ok(Self {
            frame_sequence,
            element_sequence,
        })
    }
}

fn parse_sequence(value: &str) -> Result<u64, RefParseError> {
    if value.starts_with('0') || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RefParseError);
    }
    value.parse().map_err(|_| RefParseError)
}

impl Serialize for Ref {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Ref {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefParseError;

impl Display for RefParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("browser ref must use e<n> or f<frameSeq>e<n> with positive integers")
    }
}

impl Error for RefParseError {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementHandleInfo {
    pub role: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotGeneration {
    pub document_generation: u64,
    pub frame_generation: u64,
    pub refs: HashMap<Ref, ElementHandleInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefError {
    Unknown {
        #[serde(rename = "ref")]
        reference: Ref,
    },
    Stale {
        #[serde(rename = "ref")]
        reference: Ref,
    },
    Detached {
        #[serde(rename = "ref")]
        reference: Ref,
    },
    GenerationMismatch {
        #[serde(rename = "ref")]
        reference: Ref,
        snapshot_document_generation: u64,
        current_document_generation: u64,
        snapshot_frame_generation: u64,
        current_frame_generation: u64,
    },
    Protocol {
        message: String,
    },
}

impl Display for RefError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown { reference } => write!(
                formatter,
                "ref {reference} is unknown; take a fresh AI snapshot and use a listed ref"
            ),
            Self::Stale { reference } => write!(
                formatter,
                "ref {reference} is stale because a newer snapshot replaced it; take a fresh AI snapshot"
            ),
            Self::Detached { reference } => write!(
                formatter,
                "ref {reference} is detached from the document; take a fresh AI snapshot"
            ),
            Self::GenerationMismatch {
                reference,
                snapshot_document_generation,
                current_document_generation,
                ..
            } => {
                if snapshot_document_generation != current_document_generation {
                    write!(
                        formatter,
                        "ref {reference} belongs to an earlier document generation (the page navigated since that snapshot); take a fresh AI snapshot"
                    )
                } else {
                    write!(
                        formatter,
                        "ref {reference} belongs to an earlier snapshot generation (the page's script context was recreated since that snapshot); take a fresh AI snapshot"
                    )
                }
            }
            Self::Protocol { message } => formatter.write_str(message),
        }
    }
}

impl Error for RefError {}

#[derive(Default)]
struct TabRefState {
    document_generation: u64,
    frame_generation: u64,
    latest: Option<SnapshotGeneration>,
    known_refs: HashSet<Ref>,
    bindings: HashMap<Ref, RefFrameBinding>,
    next_frame_prefix: u64,
}

#[derive(Default)]
struct RegistryState {
    tabs: HashMap<String, TabRefState>,
}

#[derive(Clone, Default)]
pub struct RefRegistry {
    state: Arc<Mutex<RegistryState>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefFrameBinding {
    pub frame_id: String,
    pub context_id: i64,
    pub generation: u64,
    pub local_reference: Option<Ref>,
}

impl RefFrameBinding {
    pub(crate) fn is_current(&self, tree: Option<&crate::FrameTree>) -> bool {
        tree.and_then(|tree| tree.frame(&self.frame_id))
            .is_some_and(|frame| {
                frame.generation == self.generation
                    && frame.session_id.is_none()
                    && frame
                        .utility_context
                        .as_ref()
                        .is_some_and(|context| context.context_id == self.context_id)
            })
    }
}

impl RefRegistry {
    pub(crate) fn allocate_frame_prefix(&self, target_id: &str) -> String {
        let mut state = self.state.lock().unwrap();
        let tab = state.tabs.entry(target_id.to_string()).or_default();
        tab.next_frame_prefix = tab
            .next_frame_prefix
            .checked_add(1)
            .expect("frame ref prefix exhausted");
        format!("f{}", (1u64 << 63) + tab.next_frame_prefix)
    }

    pub(crate) fn bind_snapshot_frames(
        &self,
        target_id: &str,
        bindings: HashMap<Ref, RefFrameBinding>,
    ) {
        let mut state = self.state.lock().unwrap();
        state
            .tabs
            .entry(target_id.to_string())
            .or_default()
            .bindings = bindings;
    }

    pub(crate) fn frame_binding(
        &self,
        target_id: &str,
        reference: &Ref,
    ) -> Option<RefFrameBinding> {
        self.state
            .lock()
            .unwrap()
            .tabs
            .get(target_id)?
            .bindings
            .get(reference)
            .cloned()
    }

    pub fn replace_snapshot(
        &self,
        target_id: &str,
        refs: HashMap<Ref, ElementHandleInfo>,
    ) -> SnapshotGeneration {
        let mut state = self.state.lock().unwrap();
        let tab = state.tabs.entry(target_id.to_string()).or_default();
        tab.known_refs.extend(refs.keys().cloned());
        let generation = SnapshotGeneration {
            document_generation: tab.document_generation,
            frame_generation: tab.frame_generation,
            refs,
        };
        tab.latest = Some(generation.clone());
        generation
    }

    pub fn top_level_navigation(&self, target_id: &str) {
        let mut state = self.state.lock().unwrap();
        let tab = state.tabs.entry(target_id.to_string()).or_default();
        tab.document_generation = tab.document_generation.saturating_add(1);
        tab.frame_generation = tab.frame_generation.saturating_add(1);
        tab.known_refs = tab
            .latest
            .as_ref()
            .map(|snapshot| snapshot.refs.keys().cloned().collect())
            .unwrap_or_default();
    }

    pub fn utility_world_recreated(&self, target_id: &str) {
        let mut state = self.state.lock().unwrap();
        let tab = state.tabs.entry(target_id.to_string()).or_default();
        if tab.latest.is_none() {
            return;
        }
        tab.frame_generation = tab.frame_generation.saturating_add(1);
        tab.known_refs = tab
            .latest
            .as_ref()
            .map(|snapshot| snapshot.refs.keys().cloned().collect())
            .unwrap_or_default();
    }

    pub fn tab_closed(&self, target_id: &str) {
        self.state.lock().unwrap().tabs.remove(target_id);
    }

    #[cfg(test)]
    fn known_ref_count(&self, target_id: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .tabs
            .get(target_id)
            .map(|tab| tab.known_refs.len())
            .unwrap_or(0)
    }

    pub fn ref_prefix(&self, target_id: &str) -> Option<String> {
        let frame_generation = self
            .state
            .lock()
            .unwrap()
            .tabs
            .get(target_id)
            .map(|tab| tab.frame_generation)
            .unwrap_or(0);
        (frame_generation != 0).then(|| format!("f{frame_generation}"))
    }

    pub fn resolve_current(
        &self,
        target_id: &str,
        reference: &Ref,
    ) -> Result<ElementHandleInfo, RefError> {
        let state = self.state.lock().unwrap();
        let Some(tab) = state.tabs.get(target_id) else {
            return Err(RefError::Unknown {
                reference: reference.clone(),
            });
        };
        let Some(snapshot) = &tab.latest else {
            return Err(RefError::Unknown {
                reference: reference.clone(),
            });
        };
        if snapshot.document_generation != tab.document_generation
            || snapshot.frame_generation != tab.frame_generation
        {
            return Err(RefError::GenerationMismatch {
                reference: reference.clone(),
                snapshot_document_generation: snapshot.document_generation,
                current_document_generation: tab.document_generation,
                snapshot_frame_generation: snapshot.frame_generation,
                current_frame_generation: tab.frame_generation,
            });
        }
        if let Some(info) = snapshot.refs.get(reference) {
            return Ok(info.clone());
        }
        if tab.known_refs.contains(reference) {
            Err(RefError::Stale {
                reference: reference.clone(),
            })
        } else {
            Err(RefError::Unknown {
                reference: reference.clone(),
            })
        }
    }
}

impl WorldManager {
    pub fn resolve_ref(&self, tab: &Tab, reference: &Ref) -> Result<ElementHandle, RefError> {
        self.refs.resolve_current(tab.get_target_id(), reference)?;
        if let Some(binding) = self.refs.frame_binding(tab.get_target_id(), reference) {
            let tree = self.frame_tree(tab.get_target_id());
            if !binding.is_current(tree.as_ref()) {
                return Err(RefError::Stale {
                    reference: reference.clone(),
                });
            }
            return self
                .resolve_frame_ref_handle(
                    tab,
                    &binding.frame_id,
                    binding.context_id,
                    binding.local_reference.as_ref().unwrap_or(reference),
                )
                .map_err(|error| map_handle_error(reference, error));
        }
        // Expanded refs must never fall back to the main frame if their binding was lost.
        if reference
            .frame_sequence()
            .is_some_and(|sequence| sequence >= 1u64 << 63)
        {
            return Err(RefError::Stale {
                reference: reference.clone(),
            });
        }
        self.call_injected_handle(tab, "resolveAriaRef", json!([reference]))
            .map_err(|error| map_handle_error(reference, error))
    }
}

fn map_handle_error(reference: &Ref, error: HandleError) -> RefError {
    match error {
        HandleError::Resolution(message) if message.contains(DETACHED_REF_PREFIX) => {
            RefError::Detached {
                reference: reference.clone(),
            }
        }
        HandleError::Resolution(message) if message.contains(UNKNOWN_REF_PREFIX) => {
            RefError::Unknown {
                reference: reference.clone(),
            }
        }
        other => RefError::Protocol {
            message: format!("Failed to resolve ref {reference}: {other}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str) -> ElementHandleInfo {
        ElementHandleInfo {
            role: "button".to_string(),
            name: Some(name.to_string()),
        }
    }

    #[test]
    fn child_binding_never_rebinds_on_navigation_or_context_reuse() {
        let mut tree = crate::FrameTree::default();
        tree.attach("main".into(), None);
        tree.attach("child".into(), Some("main".into()));
        tree.set_utility_context("child", None, 7);
        let binding = RefFrameBinding {
            frame_id: "child".into(),
            context_id: 7,
            generation: 0,
            local_reference: None,
        };
        assert!(binding.is_current(Some(&tree)));
        tree.navigate("child");
        // Even a delayed/reused numeric context must not resurrect the old document's ref.
        tree.set_utility_context("child", None, 7);
        assert!(!binding.is_current(Some(&tree)));
        let fresh = RefFrameBinding {
            generation: 1,
            ..binding.clone()
        };
        assert!(fresh.is_current(Some(&tree)));
        tree.detach("child");
        assert!(!fresh.is_current(Some(&tree)));
        assert!(!fresh.is_current(None));
    }

    #[test]
    fn cross_frame_same_element_sequence_has_distinct_registry_bindings() {
        let registry = RefRegistry::default();
        let first: Ref = format!("{}e1", registry.allocate_frame_prefix("tab"))
            .parse()
            .unwrap();
        let second: Ref = format!("{}e1", registry.allocate_frame_prefix("tab"))
            .parse()
            .unwrap();
        assert_ne!(first, second);
        registry.replace_snapshot(
            "tab",
            HashMap::from([(first.clone(), info("A")), (second.clone(), info("B"))]),
        );
        let binding = RefFrameBinding {
            frame_id: "child-a".into(),
            context_id: 7,
            generation: 0,
            local_reference: None,
        };
        registry.bind_snapshot_frames(
            "tab",
            HashMap::from([
                (first.clone(), binding.clone()),
                (
                    second.clone(),
                    RefFrameBinding {
                        frame_id: "child-b".into(),
                        context_id: 8,
                        ..binding
                    },
                ),
            ]),
        );
        assert_eq!(
            registry.frame_binding("tab", &first).unwrap().frame_id,
            "child-a"
        );
        assert_eq!(
            registry.frame_binding("tab", &second).unwrap().frame_id,
            "child-b"
        );
        registry.top_level_navigation("tab");
        assert!(matches!(
            registry.resolve_current("tab", &first),
            Err(RefError::GenerationMismatch { .. })
        ));
    }

    #[test]
    fn ref_syntax_round_trips_plain_and_frame_prefixed_forms() {
        let plain = Ref::new(12).unwrap();
        let framed = Ref::in_frame(7, 42).unwrap();
        assert_eq!(plain.to_string(), "e12");
        assert_eq!(framed.to_string(), "f7e42");
        assert_eq!("e12".parse::<Ref>().unwrap(), plain);
        assert_eq!("f7e42".parse::<Ref>().unwrap(), framed);
        for invalid in ["", "e", "e0", "e01", "f1", "f0e1", "f1e0", "f1e01", "x1"] {
            assert!(invalid.parse::<Ref>().is_err(), "{invalid}");
        }
    }

    #[test]
    fn registry_replaces_latest_snapshot_and_supports_repeated_resolution() {
        let registry = RefRegistry::default();
        let first: Ref = "e1".parse().unwrap();
        let second: Ref = "e2".parse().unwrap();
        registry.replace_snapshot("tab", HashMap::from([(first.clone(), info("First"))]));
        assert_eq!(
            registry.resolve_current("tab", &first).unwrap(),
            info("First")
        );
        assert_eq!(
            registry.resolve_current("tab", &first).unwrap(),
            info("First")
        );
        registry.replace_snapshot("tab", HashMap::from([(second.clone(), info("Second"))]));
        assert!(matches!(
            registry.resolve_current("tab", &first),
            Err(RefError::Stale { .. })
        ));
        assert_eq!(
            registry.resolve_current("tab", &second).unwrap(),
            info("Second")
        );
    }

    #[test]
    fn navigation_rejects_refs_from_the_prior_generation() {
        let registry = RefRegistry::default();
        let reference: Ref = "e9".parse().unwrap();
        registry.replace_snapshot("tab", HashMap::from([(reference.clone(), info("Save"))]));
        registry.top_level_navigation("tab");
        assert!(matches!(
            registry.resolve_current("tab", &reference),
            Err(RefError::GenerationMismatch {
                snapshot_document_generation: 0,
                current_document_generation: 1,
                snapshot_frame_generation: 0,
                current_frame_generation: 1,
                ..
            })
        ));
        assert_eq!(registry.ref_prefix("tab").as_deref(), Some("f1"));
    }

    #[test]
    fn recreated_utility_world_moves_refs_to_a_new_prefix_instead_of_colliding() {
        let registry = RefRegistry::default();
        assert_eq!(registry.ref_prefix("tab"), None);
        registry.utility_world_recreated("tab");
        assert_eq!(
            registry.ref_prefix("tab"),
            None,
            "no snapshot yet: nothing to protect"
        );

        let reference: Ref = "e6".parse().unwrap();
        registry.replace_snapshot("tab", HashMap::from([(reference.clone(), info("Main"))]));
        registry.utility_world_recreated("tab");
        assert_eq!(registry.ref_prefix("tab").as_deref(), Some("f1"));
        let error = registry.resolve_current("tab", &reference).unwrap_err();
        assert!(matches!(
            error,
            RefError::GenerationMismatch {
                snapshot_document_generation: 0,
                current_document_generation: 0,
                snapshot_frame_generation: 0,
                current_frame_generation: 1,
                ..
            }
        ));
        assert!(
            error.to_string().contains("script context was recreated"),
            "{error}"
        );

        registry.top_level_navigation("tab");
        let navigated = registry.resolve_current("tab", &reference).unwrap_err();
        assert!(
            navigated.to_string().contains("the page navigated"),
            "{navigated}"
        );
    }

    #[test]
    fn scoped_snapshot_reuses_the_generation_store_of_the_page_snapshot() {
        let registry = RefRegistry::default();
        let outside: Ref = "e1".parse().unwrap();
        let inside: Ref = "e2".parse().unwrap();
        let minted_while_scoped: Ref = "e3".parse().unwrap();
        let page = registry.replace_snapshot(
            "tab",
            HashMap::from([
                (outside.clone(), info("Outside")),
                (inside.clone(), info("Inside")),
            ]),
        );
        let scoped = registry.replace_snapshot(
            "tab",
            HashMap::from([
                (inside.clone(), info("Inside")),
                (minted_while_scoped.clone(), info("Nested")),
            ]),
        );

        assert_eq!(
            (scoped.document_generation, scoped.frame_generation),
            (page.document_generation, page.frame_generation),
            "scoping must not bump generations the way navigation does"
        );
        assert_eq!(
            registry.resolve_current("tab", &inside).unwrap(),
            info("Inside"),
            "a ref re-listed by the scoped subtree stays resolvable"
        );
        assert_eq!(
            registry
                .resolve_current("tab", &minted_while_scoped)
                .unwrap(),
            info("Nested")
        );
        assert!(
            matches!(
                registry.resolve_current("tab", &outside),
                Err(RefError::Stale { .. })
            ),
            "refs outside the scoped subtree go stale, not generation-mismatched"
        );
    }

    #[test]
    fn navigation_trims_known_refs_to_the_previous_generation() {
        let registry = RefRegistry::default();
        let first: Ref = "e1".parse().unwrap();
        let second: Ref = "e2".parse().unwrap();
        let third: Ref = "e3".parse().unwrap();
        registry.replace_snapshot("tab", HashMap::from([(first.clone(), info("First"))]));
        registry.replace_snapshot("tab", HashMap::from([(second.clone(), info("Second"))]));
        assert_eq!(registry.known_ref_count("tab"), 2);

        registry.top_level_navigation("tab");
        assert_eq!(
            registry.known_ref_count("tab"),
            1,
            "only the generation being left stays addressable for stale reporting"
        );

        registry.replace_snapshot("tab", HashMap::from([(third.clone(), info("Third"))]));
        assert!(
            matches!(
                registry.resolve_current("tab", &second),
                Err(RefError::Stale { .. })
            ),
            "the previous generation's refs still report as stale, not unknown"
        );
        assert!(matches!(
            registry.resolve_current("tab", &first),
            Err(RefError::Unknown { .. })
        ));
        assert_eq!(
            registry.resolve_current("tab", &third).unwrap(),
            info("Third")
        );
    }

    #[test]
    fn repeated_navigation_does_not_grow_the_known_ref_set() {
        let registry = RefRegistry::default();
        for generation in 0..50u64 {
            let reference: Ref = format!("e{}", generation + 1).parse().unwrap();
            registry.replace_snapshot("tab", HashMap::from([(reference, info("Row"))]));
            registry.top_level_navigation("tab");
        }
        assert_eq!(registry.known_ref_count("tab"), 1);
    }

    #[test]
    fn closing_a_tab_drops_its_generation_store() {
        let registry = RefRegistry::default();
        let reference: Ref = "e1".parse().unwrap();
        registry.replace_snapshot("tab", HashMap::from([(reference.clone(), info("Save"))]));
        registry.top_level_navigation("tab");
        registry.tab_closed("tab");
        assert_eq!(registry.known_ref_count("tab"), 0);
        assert_eq!(registry.ref_prefix("tab"), None);
        assert!(matches!(
            registry.resolve_current("tab", &reference),
            Err(RefError::Unknown { .. })
        ));
    }

    #[test]
    fn error_taxonomy_serializes_with_actionable_kinds() {
        let reference: Ref = "e3".parse().unwrap();
        let errors = [
            RefError::Unknown {
                reference: reference.clone(),
            },
            RefError::Stale {
                reference: reference.clone(),
            },
            RefError::Detached {
                reference: reference.clone(),
            },
            RefError::GenerationMismatch {
                reference,
                snapshot_document_generation: 1,
                current_document_generation: 2,
                snapshot_frame_generation: 4,
                current_frame_generation: 5,
            },
        ];
        let kinds = errors
            .into_iter()
            .map(|error| {
                let value = serde_json::to_value(&error).unwrap();
                assert_eq!(
                    serde_json::from_value::<RefError>(value.clone()).unwrap(),
                    error
                );
                value["kind"].as_str().unwrap().to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            ["unknown", "stale", "detached", "generation_mismatch"]
        );
    }
}
