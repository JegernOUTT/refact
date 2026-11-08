use std::collections::HashMap;

use headless_chrome::Tab;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ElementHandle, ElementHandleInfo, HandleError, Ref, SnapshotGeneration, WorldManager};

#[path = "discovery.rs"]
mod discovery;
pub use discovery::{SnapshotSearchOptions, SnapshotSearchMatch, SnapshotSearchResult};
use discovery::{bound_snapshot, bind_nodes, alias_frame};

const ARIA_SNAPSHOT_FUNCTION: &str = "function(options) { const instance = globalThis.__refact_injected__; if (!instance) throw new Error('RefactInjected is not installed'); return instance.ariaSnapshot(this, options); }";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotMode {
    Ai,
    #[default]
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SnapshotOptions {
    pub mode: SnapshotMode,
    pub refs: bool,
    pub boxes: bool,
    pub depth: Option<u32>,
    pub do_not_render_active: bool,
    pub include_frames: bool,
    pub max_frame_depth: u32,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            mode: SnapshotMode::Default,
            refs: false,
            boxes: false,
            depth: None,
            do_not_render_active: false,
            include_frames: false,
            max_frame_depth: 2,
        }
    }
}

impl SnapshotOptions {
    /// Search needs the same actionable identity list as an AI snapshot. The
    /// injected renderer deliberately ignores `refs` in default snapshot mode.
    pub fn for_search(include_frames: bool, max_frame_depth: u32) -> Self {
        Self {
            mode: SnapshotMode::Ai,
            refs: true,
            include_frames,
            max_frame_depth,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotBox {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotNode {
    pub role: String,
    pub name: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    #[serde(rename = "box")]
    pub geometry: Option<SnapshotBox>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AriaSnapshot {
    pub yaml: String,
    pub nodes: Vec<SnapshotNode>,
    #[serde(default)]
    pub generation: Option<SnapshotGeneration>,
    #[serde(default)]
    pub frames: Vec<SnapshotFrame>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub root_frame_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFrame {
    pub frame_id: String,
    pub parent_frame_id: Option<String>,
    pub depth: u32,
    pub yaml: String,
    pub nodes: Vec<SnapshotNode>,
}

impl WorldManager {
    pub fn search_snapshot(
        &self,
        tab: &Tab,
        mut options: SnapshotSearchOptions,
        include_frames: bool,
        max_frame_depth: u32,
    ) -> Result<SnapshotSearchResult, HandleError> {
        // Resolve against the caller's current registry before capture replaces it.
        // In particular, child-frame public aliases change on every capture.
        let root = options
            .subtree
            .as_deref()
            .map(|reference| {
                let reference = reference.parse::<Ref>().map_err(|error| {
                    HandleError::Resolution(format!("Invalid subtree ref: {error}"))
                })?;
                self.resolve_ref(tab, &reference)
                    .map_err(|error| HandleError::Resolution(error.to_string()))
            })
            .transpose()?;
        struct ScopedRoot<'a> {
            world: &'a WorldManager,
            tab: &'a Tab,
            handle: Option<ElementHandle>,
        }
        impl Drop for ScopedRoot<'_> {
            fn drop(&mut self) {
                if let Some(handle) = &self.handle {
                    let _ = self.world.release_handle(self.tab, handle);
                }
            }
        }
        let root = ScopedRoot {
            world: self,
            tab,
            handle: root,
        };
        let snapshot = self.aria_snapshot(
            tab,
            root.handle.clone(),
            SnapshotOptions::for_search(include_frames, max_frame_depth),
        )?;
        // The capture itself is bounded to the resolved DOM root, including that
        // root. Do not filter by its now-obsolete public alias a second time.
        options.subtree = None;
        snapshot.search(&options).map_err(HandleError::Resolution)
    }

    pub fn aria_snapshot(
        &self,
        tab: &Tab,
        root_handle: Option<ElementHandle>,
        options: SnapshotOptions,
    ) -> Result<AriaSnapshot, HandleError> {
        let target_id = tab.get_target_id();
        let include_frames = options.include_frames;
        let max_frame_depth = options.max_frame_depth.min(8);
        self.ensure_snapshot_world(tab)
            .map_err(HandleError::Resolution)?;
        let tree = self
            .frame_tree(target_id)
            .ok_or_else(|| HandleError::Protocol("Missing frame tree".into()))?;
        let root_frame_id = root_handle
            .as_ref()
            .map(|root| root.frame_id.clone())
            .or_else(|| tree.main_frame_id().map(str::to_string));
        let mut bindings = HashMap::new();
        let mut options = serde_json::to_value(options).map_err(|error| {
            HandleError::Protocol(format!(
                "Failed to serialize ARIA snapshot options: {error}"
            ))
        })?;
        if let Some(prefix) = self.refs.ref_prefix(target_id) {
            options["refPrefix"] = Value::String(prefix);
        }
        let scoped = root_handle.is_some();
        let child_root = root_frame_id.as_deref() != tree.main_frame_id();
        if child_root {
            options["refPrefix"] = Value::String(self.refs.allocate_frame_prefix(target_id));
        }
        let root_options = options.clone();
        let value = if let Some(root_handle) = root_handle {
            self.call_function_on(tab, &root_handle, ARIA_SNAPSHOT_FUNCTION, vec![options])?
        } else {
            self.eval_in_utility_frame(
                tab,
                root_frame_id.as_deref().unwrap(),
                &format!(
                    "globalThis.__refact_injected__.ariaSnapshot(null,{})",
                    options
                ),
            )
            .map_err(HandleError::Resolution)?
        };
        let mut snapshot: AriaSnapshot = serde_json::from_value(value).map_err(|error| {
            HandleError::Protocol(format!("Failed to parse ARIA snapshot: {error}"))
        })?;
        snapshot.root_frame_id = root_frame_id.clone();
        bound_snapshot(&mut snapshot);
        if let Some(frame) = root_frame_id.as_ref().and_then(|id| tree.frame(id)) {
            if child_root {
                alias_frame(
                    &mut snapshot,
                    root_options["refPrefix"].as_str().unwrap(),
                    frame,
                    &mut bindings,
                );
            } else {
                bind_nodes(&snapshot.nodes, frame, &mut bindings);
            }
        }
        if include_frames && !scoped {
            let mut pending = std::collections::VecDeque::new();
            if let Some(root) = root_frame_id.as_ref().and_then(|id| tree.frame(id)) {
                if root.children.len() > 32 {
                    snapshot
                        .warnings
                        .push("Frame count limit reached (32)".into());
                }
                for child in root.children.iter().take(32) {
                    pending.push_back((child.clone(), 1));
                }
            }
            let mut visited = std::collections::HashSet::new();
            while let Some((id, depth)) = pending.pop_front() {
                if visited.len() >= 32 {
                    snapshot
                        .warnings
                        .push("Frame count limit reached (32)".into());
                    break;
                }
                if !visited.insert(id.clone()) {
                    continue;
                }
                let Some(frame) = tree.frame(&id) else {
                    continue;
                };
                if depth > max_frame_depth {
                    snapshot.warnings.push("Frame depth limit reached".into());
                    continue;
                }
                if frame.session_id.is_some() {
                    snapshot.warnings.push(format!(
                        "Frame {id} unavailable: out-of-process iframe transport unsupported"
                    ));
                    continue;
                }
                let mut options = root_options.clone();
                options["refPrefix"] = Value::String(self.refs.allocate_frame_prefix(target_id));
                let context = self.ensure_utility_world_for_frame(tab, &id);
                let captured_frame = self
                    .frame_tree(target_id)
                    .and_then(|tree| tree.frame(&id).cloned());
                let captured = context
                    .map_err(HandleError::Resolution)
                    .and_then(|_| {
                        self.eval_in_utility_frame(
                            tab,
                            &id,
                            &format!("globalThis.__refact_injected__.ariaSnapshot(null,{options})"),
                        )
                        .map_err(HandleError::Resolution)
                    })
                    .and_then(|value| {
                        serde_json::from_value::<AriaSnapshot>(value)
                            .map_err(|e| HandleError::Protocol(e.to_string()))
                    });
                match captured {
                    Ok(mut child) => {
                        bound_snapshot(&mut child);
                        snapshot.warnings.append(&mut child.warnings);
                        let bytes = snapshot.yaml.len()
                            + snapshot.frames.iter().map(|f| f.yaml.len()).sum::<usize>();
                        let nodes = snapshot.nodes.len()
                            + snapshot.frames.iter().map(|f| f.nodes.len()).sum::<usize>();
                        if let Some(captured) = &captured_frame {
                            alias_frame(
                                &mut child,
                                options["refPrefix"].as_str().unwrap(),
                                captured,
                                &mut bindings,
                            );
                        }
                        if bytes + child.yaml.len() > 256 * 1024 || nodes + child.nodes.len() > 4096
                        {
                            for node in &child.nodes {
                                if let Some(reference) =
                                    node.reference.as_ref().and_then(|r| r.parse::<Ref>().ok())
                                {
                                    bindings.remove(&reference);
                                }
                            }
                            snapshot.warnings.push("Frame expansion stopped at aggregate 256 KiB / 4096 node limit; renderer DOM traversal is not bounded".into());
                            break;
                        }
                        snapshot.frames.push(SnapshotFrame {
                            frame_id: id.clone(),
                            parent_frame_id: frame.parent_id.clone(),
                            depth,
                            yaml: child.yaml,
                            nodes: child.nodes,
                        });
                    }
                    Err(error) => snapshot
                        .warnings
                        .push(format!("Frame {id} unavailable: {error}")),
                }
                if frame.children.len() > 32usize.saturating_sub(visited.len() + pending.len()) {
                    snapshot
                        .warnings
                        .push("Frame count limit reached (32)".into());
                }
                for child in frame
                    .children
                    .iter()
                    .take(32usize.saturating_sub(visited.len() + pending.len()))
                {
                    pending.push_back((child.clone(), depth + 1));
                }
            }
        } else if include_frames && scoped {
            snapshot.warnings.push(
                "Frame expansion omitted for scoped subtree; take a page snapshot to expand frames"
                    .into(),
            );
        }
        let refs = snapshot
            .nodes
            .iter()
            .chain(snapshot.frames.iter().flat_map(|frame| frame.nodes.iter()))
            .filter_map(|node| {
                node.reference.as_ref().map(|reference| {
                    let reference = reference.parse::<Ref>().map_err(|error| {
                        HandleError::Protocol(format!(
                            "Failed to parse ARIA snapshot ref {reference}: {error}"
                        ))
                    })?;
                    Ok((
                        reference,
                        ElementHandleInfo {
                            role: node.role.clone(),
                            name: node.name.clone(),
                        },
                    ))
                })
            })
            .collect::<Result<HashMap<_, _>, HandleError>>()?;
        snapshot.generation = Some(self.refs.replace_snapshot(target_id, refs));
        self.refs.bind_snapshot_frames(target_id, bindings);
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "launches an isolated owned Chrome; enable with REFACT_DISCOVERY_CHROME=1"]
    fn owned_chrome_prior_child_subtree_search_recaptures_exact_scope() {
        if std::env::var("REFACT_DISCOVERY_CHROME").as_deref() != Ok("1") {
            return;
        }
        let profile = tempfile::tempdir().unwrap();
        let launch = crate::BrowserLaunchOptions {
            headless: true,
            chromium_sandbox: false,
            ..Default::default()
        };
        let args = launch.chrome_args();
        let ignored = launch.ignored_default_args();
        let browser = headless_chrome::Browser::new(crate::chrome_launch_options(
            &launch,
            profile.path(),
            &args,
            &ignored,
        ))
        .unwrap();
        let tab = browser.new_tab().unwrap();
        tab.navigate_to("about:blank")
            .unwrap()
            .wait_until_navigated()
            .unwrap();
        tab.evaluate(r#"document.body.innerHTML = '<section aria-label="Main Scope"><button>Main Inside</button></section><button>Main Outside</button><iframe></iframe>'; document.querySelector('iframe').srcdoc = '<section aria-label="Child Scope"><section aria-label="Nested Scope"><button onclick="this.dataset.clicked=String(Number(this.dataset.clicked||0)+1)">Child Inside</button></section></section><button>Child Outside</button>'"#, false).unwrap();
        let world = WorldManager::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let snapshot = loop {
            let snapshot = world
                .aria_snapshot(&tab, None, SnapshotOptions::for_search(true, 2))
                .unwrap();
            if snapshot.frames.iter().any(|frame| {
                frame
                    .nodes
                    .iter()
                    .any(|node| node.name.as_deref() == Some("Nested Scope"))
            }) {
                break snapshot;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child did not become ready"
            );
            std::thread::park_timeout(std::time::Duration::from_millis(10));
        };
        let child_frame = &snapshot.frames[0].frame_id;
        let root = snapshot
            .frames
            .iter()
            .flat_map(|frame| &frame.nodes)
            .find(|node| node.name.as_deref() == Some("Nested Scope"))
            .unwrap()
            .reference
            .clone()
            .unwrap();
        // This is the shipping controller helper, not search on a fabricated snapshot.
        let search = |subtree: String, max_results| {
            world.search_snapshot(
                &tab,
                SnapshotSearchOptions {
                    pattern: ".".into(),
                    regex: true,
                    subtree: Some(subtree),
                    max_results,
                    ..Default::default()
                },
                true,
                2,
            )
        };
        let found = search(root.clone(), 50)
            .expect("prior live child subtree must resolve before recapture");
        assert_eq!(found.matches.len(), 2, "{found:?}");
        let region = &found.matches[0];
        let button = &found.matches[1];
        assert_eq!(region.name.as_deref(), Some("Nested Scope"));
        assert_eq!(button.name.as_deref(), Some("Child Inside"));
        assert_ne!(region.reference, root, "child aliases must be fresh");
        assert_eq!(button.ancestor_refs, vec![region.reference.clone()]);
        assert!(found
            .matches
            .iter()
            .all(|m| m.frame_id.as_ref() == Some(child_frame)));
        assert!(world.resolve_ref(&tab, &root.parse().unwrap()).is_err());

        // A bad supplied ref must not replace the registry or invalidate good refs.
        assert!(search("e999999999".into(), 50).is_err());
        let handle = world
            .resolve_ref(&tab, &button.reference.parse().unwrap())
            .unwrap();
        assert_eq!(
            world
                .call_function_on(
                    &tab,
                    &handle,
                    "function() { this.click(); return this.dataset.clicked; }",
                    vec![]
                )
                .unwrap(),
            Value::String("1".into())
        );
        world.release_handle(&tab, &handle).unwrap();

        let repeated = search(region.reference.clone(), 50).unwrap();
        assert_eq!(repeated.matches.len(), 2);
        assert_ne!(repeated.matches[0].reference, region.reference);
        let limited = search(repeated.matches[0].reference.clone(), 1).unwrap();
        assert_eq!(limited.matches.len(), 1);
        assert!(limited.truncated);
        let old = limited.matches[0].reference.clone();
        tab.evaluate("document.querySelector('iframe').srcdoc = '<section aria-label=\"Replacement\"><button>Replacement</button></section>'", false).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if world.resolve_ref(&tab, &old.parse().unwrap()).is_err() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "old scoped child ref survived navigation"
            );
            std::thread::park_timeout(std::time::Duration::from_millis(10));
        }
        assert!(search(old, 50).is_err());

        let snapshot = world
            .aria_snapshot(&tab, None, SnapshotOptions::for_search(false, 2))
            .unwrap();
        let main = snapshot
            .nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("Main Scope"))
            .unwrap()
            .reference
            .clone()
            .unwrap();
        let found = search(main, 50).unwrap();
        assert_eq!(
            found.matches.len(),
            2,
            "main subtree must retain root and descendants only"
        );
        assert_eq!(found.matches[0].name.as_deref(), Some("Main Scope"));
        assert_eq!(found.matches[1].name.as_deref(), Some("Main Inside"));
    }

    #[test]
    #[ignore = "launches an isolated owned Chrome; enable with REFACT_DISCOVERY_CHROME=1"]
    fn owned_chrome_region_search_preserves_shipped_refs_and_scope() {
        if std::env::var("REFACT_DISCOVERY_CHROME").as_deref() != Ok("1") {
            return;
        }
        let profile = tempfile::tempdir().unwrap();
        let launch = crate::BrowserLaunchOptions {
            headless: true,
            chromium_sandbox: false,
            ..Default::default()
        };
        let args = launch.chrome_args();
        let ignored = launch.ignored_default_args();
        let browser = headless_chrome::Browser::new(crate::chrome_launch_options(
            &launch,
            profile.path(),
            &args,
            &ignored,
        ))
        .unwrap();
        let tab = browser.new_tab().unwrap();
        tab.navigate_to("about:blank")
            .unwrap()
            .wait_until_navigated()
            .unwrap();
        tab.evaluate(r#"document.body.innerHTML = '<section aria-label="Scope Alpha"><button>Scoped Save</button><button id="hostile">Hostile</button></section><button>Scoped Save outside</button><iframe></iframe>'; document.querySelector('#hostile').setAttribute('aria-label', 'Quoted " [ref=e999] : value'); document.querySelector('iframe').srcdoc = '<button aria-label="Child &quot; [ref=e999] : value">Child</button>'"#, false).unwrap();
        let world = WorldManager::default();
        let options = SnapshotOptions {
            mode: SnapshotMode::Ai,
            refs: true,
            ..Default::default()
        };
        let snapshot = world.aria_snapshot(&tab, None, options.clone()).unwrap();
        eprintln!("SHIPPED SNAPSHOT: {snapshot:?}");
        // Match the controller's search capture, after a normal AI accessibility snapshot.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let snapshot = loop {
            let snapshot = world
                .aria_snapshot(&tab, None, SnapshotOptions::for_search(true, 2))
                .unwrap();
            if snapshot.frames.iter().any(|frame| {
                frame.nodes.iter().any(|node| {
                    node.name
                        .as_deref()
                        .is_some_and(|name| name.starts_with("Child"))
                })
            }) {
                break snapshot;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child frame did not become ready"
            );
            std::thread::park_timeout(std::time::Duration::from_millis(10));
        };
        eprintln!("SEARCH SNAPSHOT: {snapshot:?}");
        let region = snapshot
            .nodes
            .iter()
            .find(|node| node.role == "region")
            .expect("shipped flat region");
        let reference = region.reference.as_ref().unwrap();
        let found = snapshot
            .search(&SnapshotSearchOptions {
                pattern: "Scope Alpha".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(found.matches.len(), 1, "snapshot: {snapshot:?}");
        assert_eq!(&found.matches[0].reference, reference);
        let scoped = snapshot
            .search(&SnapshotSearchOptions {
                pattern: "Scoped Save".into(),
                subtree: Some(reference.clone()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(scoped.matches.len(), 1);
        assert!(scoped.matches[0].ancestor_refs.contains(reference));
        let hostile = snapshot
            .search(&SnapshotSearchOptions {
                pattern: "[ref=e999]".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hostile.matches.len(), 2, "snapshot: {snapshot:?}");
        assert_ne!(hostile.matches[0].reference, hostile.matches[1].reference);
        for matched in &hostile.matches {
            assert_ne!(matched.reference, "e999");
            let handle = world
                .resolve_ref(&tab, &matched.reference.parse().unwrap())
                .unwrap();
            let label = world
                .call_function_on(
                    &tab,
                    &handle,
                    "function() { return this.getAttribute('aria-label'); }",
                    vec![],
                )
                .unwrap();
            assert_eq!(label.as_str(), matched.name.as_deref());
        }
        let handle = world
            .resolve_ref(&tab, &reference.parse().unwrap())
            .unwrap();
        let scoped_snapshot = world.aria_snapshot(&tab, Some(handle), options).unwrap();
        assert_eq!(
            scoped_snapshot
                .search(&SnapshotSearchOptions {
                    pattern: "Scope Alpha".into(),
                    ..Default::default()
                })
                .unwrap()
                .matches
                .len(),
            1
        );
    }

    #[test]
    fn search_options_enable_renderer_identity_collection() {
        let value = serde_json::to_value(SnapshotOptions::for_search(true, 3)).unwrap();
        assert_eq!(value["mode"], "ai");
        assert_eq!(value["refs"], true);
        assert_eq!(value["includeFrames"], true);
        assert_eq!(value["maxFrameDepth"], 3);
        assert_eq!(SnapshotOptions::default().mode, SnapshotMode::Default);
        assert!(!SnapshotOptions::default().refs);
    }

    #[test]
    #[ignore = "launches an isolated owned Chrome; enable with REFACT_DISCOVERY_CHROME=1"]
    fn owned_chrome_child_ref_resolves_directly_and_rejects_navigation() {
        if std::env::var("REFACT_DISCOVERY_CHROME").as_deref() != Ok("1") {
            return;
        }
        let profile = tempfile::tempdir().unwrap();
        let options = crate::BrowserLaunchOptions {
            headless: true,
            chromium_sandbox: false,
            ..Default::default()
        };
        let args = options.chrome_args();
        let ignored = options.ignored_default_args();
        let browser = headless_chrome::Browser::new(crate::chrome_launch_options(
            &options,
            profile.path(),
            &args,
            &ignored,
        ))
        .unwrap();
        let tab = browser.new_tab().unwrap();
        tab.navigate_to("about:blank")
            .unwrap()
            .wait_until_navigated()
            .unwrap();
        tab.evaluate("document.body.innerHTML = `<button>Main</button><iframe srcdoc=\"<button onclick='this.textContent=String(Number(this.textContent)||0)+1'>Child</button>\"></iframe>`", false).unwrap();
        let world = WorldManager::default();
        let options = SnapshotOptions {
            mode: SnapshotMode::Ai,
            refs: true,
            include_frames: true,
            ..Default::default()
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let snapshot = loop {
            let snapshot = world.aria_snapshot(&tab, None, options.clone()).unwrap();
            if snapshot.frames.iter().any(|frame| {
                frame
                    .nodes
                    .iter()
                    .any(|node| node.name.as_deref() == Some("Child"))
            }) {
                break snapshot;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child frame did not become ready"
            );
            std::thread::park_timeout(std::time::Duration::from_millis(10));
        };
        let child = snapshot
            .frames
            .iter()
            .flat_map(|frame| &frame.nodes)
            .find(|node| node.name.as_deref() == Some("Child"))
            .unwrap();
        let reference = child.reference.as_ref().unwrap().parse::<Ref>().unwrap();
        let main = snapshot
            .nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("Main"))
            .unwrap();
        assert_ne!(
            main.reference.as_ref().unwrap(),
            child.reference.as_ref().unwrap()
        );
        let found = snapshot
            .search(&SnapshotSearchOptions {
                pattern: "Child".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(found.matches.len(), 1, "snapshot: {snapshot:?}");
        assert_eq!(found.matches[0].reference, reference.to_string());
        assert_eq!(
            found.matches[0].frame_id.as_deref(),
            snapshot.frames.first().map(|f| f.frame_id.as_str())
        );
        let handle = world.resolve_ref(&tab, &reference).unwrap();
        assert_ne!(
            Some(handle.frame_id.as_str()),
            snapshot.root_frame_id.as_deref()
        );
        let clicked = world
            .call_function_on(
                &tab,
                &handle,
                "function() { this.click(); return this.textContent; }",
                vec![],
            )
            .unwrap();
        assert_eq!(clicked, Value::String("01".into()));
        tab.evaluate(
            "document.querySelector('iframe').srcdoc = '<button>Replacement</button>'",
            false,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if world.resolve_ref(&tab, &reference).is_err() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "old child ref survived navigation"
            );
            std::thread::park_timeout(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn omitted_depth_stays_null_so_the_injected_truncation_guard_is_disabled() {
        let value = serde_json::to_value(SnapshotOptions::default()).unwrap();
        assert!(
            value["depth"].is_null(),
            "a null depth keeps the injected numeric depth guard false"
        );
        assert_eq!(value["boxes"], false);
    }

    #[test]
    fn zero_depth_serializes_as_a_number_so_the_injected_guard_truncates_at_the_root() {
        let value = serde_json::to_value(SnapshotOptions {
            depth: Some(0),
            ..SnapshotOptions::default()
        })
        .unwrap();
        assert_eq!(value["depth"], 0);
        assert!(value["depth"].is_number());
    }

    #[test]
    fn scoping_controls_serialize_together_for_the_injected_bridge() {
        let value = serde_json::to_value(SnapshotOptions {
            mode: SnapshotMode::Ai,
            refs: true,
            boxes: true,
            depth: Some(2),
            do_not_render_active: false,
            include_frames: false,
            max_frame_depth: 2,
        })
        .unwrap();
        assert_eq!(value["mode"], "ai");
        assert_eq!(value["refs"], true);
        assert_eq!(value["boxes"], true);
        assert_eq!(value["depth"], 2);
    }
}
