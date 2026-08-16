use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Page;
use serde_json::Value;

use crate::{ElementHandle, HandleError, WorldManager};

pub type FrameId = String;
pub type FrameSessionId = String;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameExecutionContext {
    pub session_id: Option<FrameSessionId>,
    pub context_id: i64,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameHandle {
    pub id: FrameId,
    pub parent_id: Option<FrameId>,
    pub children: Vec<FrameId>,
    pub session_id: Option<FrameSessionId>,
    pub utility_context: Option<FrameExecutionContext>,
    pub generation: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameInvalidation {
    pub frame_ids: Vec<FrameId>,
    pub contexts: Vec<FrameExecutionContext>,
}

#[derive(Clone, Debug, Default)]
pub struct FrameTree {
    frames: HashMap<FrameId, FrameHandle>,
    session_frames: HashMap<FrameSessionId, FrameId>,
    main_frame_id: Option<FrameId>,
}

impl FrameTree {
    pub fn from_cdp(tree: &Page::FrameTree) -> Self {
        let mut frames = Self::default();
        frames.add_cdp_subtree(tree, None);
        frames.main_frame_id = Some(tree.frame.id.clone());
        frames
    }

    pub fn main_frame_id(&self) -> Option<&str> {
        self.main_frame_id.as_deref()
    }

    pub fn frame(&self, frame_id: &str) -> Option<&FrameHandle> {
        self.frames.get(frame_id)
    }

    pub fn frame_for_session(&self, session_id: &str) -> Option<&FrameHandle> {
        self.session_frames
            .get(session_id)
            .and_then(|frame_id| self.frames.get(frame_id))
    }

    pub fn frame_ids(&self) -> Vec<FrameId> {
        let Some(main_frame_id) = &self.main_frame_id else {
            return Vec::new();
        };
        let mut output = Vec::new();
        self.collect_subtree(main_frame_id, &mut output);
        output
    }

    pub fn record_cdp_tree(&mut self, tree: &Page::FrameTree) {
        if self.main_frame_id.is_none() {
            self.main_frame_id = Some(tree.frame.id.clone());
        }
        self.add_cdp_subtree(tree, tree.frame.parent_id.clone());
    }

    pub fn attach(&mut self, frame_id: FrameId, parent_id: Option<FrameId>) {
        if self.main_frame_id.is_none() && parent_id.is_none() {
            self.main_frame_id = Some(frame_id.clone());
        }
        if let Some(previous_parent) = self
            .frames
            .get(&frame_id)
            .and_then(|frame| frame.parent_id.clone())
        {
            if Some(previous_parent.as_str()) != parent_id.as_deref() {
                self.remove_child(&previous_parent, &frame_id);
            }
        }
        let frame = self
            .frames
            .entry(frame_id.clone())
            .or_insert_with(|| FrameHandle {
                id: frame_id.clone(),
                parent_id: parent_id.clone(),
                children: Vec::new(),
                session_id: None,
                utility_context: None,
                generation: 0,
            });
        frame.parent_id = parent_id.clone();
        if let Some(parent_id) = parent_id {
            let parent = self
                .frames
                .entry(parent_id.clone())
                .or_insert_with(|| FrameHandle {
                    id: parent_id,
                    parent_id: None,
                    children: Vec::new(),
                    session_id: None,
                    utility_context: None,
                    generation: 0,
                });
            if !parent.children.contains(&frame_id) {
                parent.children.push(frame_id);
            }
        }
    }

    pub fn detach(&mut self, frame_id: &str) -> FrameInvalidation {
        let mut frame_ids = Vec::new();
        self.collect_subtree(frame_id, &mut frame_ids);
        let mut invalidation = FrameInvalidation::default();
        for id in frame_ids.into_iter().rev() {
            if let Some(frame) = self.frames.remove(&id) {
                if let Some(context) = frame.utility_context {
                    invalidation.contexts.push(context);
                }
                if let Some(session_id) = frame.session_id {
                    self.session_frames.remove(&session_id);
                }
                if let Some(parent_id) = frame.parent_id {
                    self.remove_child(&parent_id, &id);
                }
                invalidation.frame_ids.push(id);
            }
        }
        invalidation.frame_ids.reverse();
        invalidation.contexts.reverse();
        if self.main_frame_id.as_deref() == Some(frame_id) {
            self.main_frame_id = None;
        }
        invalidation
    }

    pub fn navigate(&mut self, frame_id: &str) -> FrameInvalidation {
        let children = self
            .frames
            .get(frame_id)
            .map(|frame| frame.children.clone())
            .unwrap_or_default();
        let mut invalidation = FrameInvalidation::default();
        for child in children {
            invalidation.extend(self.detach(&child));
        }
        if let Some(frame) = self.frames.get_mut(frame_id) {
            if let Some(context) = frame.utility_context.take() {
                invalidation.contexts.push(context);
            }
            frame.generation += 1;
            invalidation.frame_ids.push(frame_id.to_string());
        }
        invalidation
    }

    pub fn process_swap(
        &mut self,
        frame_id: FrameId,
        parent_id: Option<FrameId>,
        session_id: FrameSessionId,
    ) -> FrameInvalidation {
        self.attach(frame_id.clone(), parent_id);
        let invalidation = self.navigate(&frame_id);
        let frame = self.frames.get_mut(&frame_id).unwrap();
        if let Some(previous_session) = frame.session_id.replace(session_id.clone()) {
            self.session_frames.remove(&previous_session);
        }
        self.session_frames.insert(session_id, frame_id);
        invalidation
    }

    pub fn detach_session(&mut self, session_id: &str) -> FrameInvalidation {
        let Some(frame_id) = self.session_frames.remove(session_id) else {
            return FrameInvalidation::default();
        };
        let mut invalidation = FrameInvalidation::default();
        if let Some(frame) = self.frames.get_mut(&frame_id) {
            frame.session_id = None;
            if let Some(context) = frame.utility_context.take() {
                invalidation.contexts.push(context);
            }
            frame.generation += 1;
            invalidation.frame_ids.push(frame_id);
        }
        invalidation
    }

    pub fn set_utility_context(
        &mut self,
        frame_id: &str,
        session_id: Option<FrameSessionId>,
        context_id: i64,
    ) -> Option<FrameExecutionContext> {
        let frame = self.frames.get_mut(frame_id)?;
        let context = FrameExecutionContext {
            session_id,
            context_id,
            generation: frame.generation,
        };
        frame.utility_context.replace(context)
    }

    pub fn destroy_context(
        &mut self,
        session_id: Option<&str>,
        context_id: i64,
    ) -> Option<FrameId> {
        let frame = self.frames.values_mut().find(|frame| {
            frame.utility_context.as_ref().is_some_and(|context| {
                context.context_id == context_id && context.session_id.as_deref() == session_id
            })
        })?;
        frame.utility_context = None;
        Some(frame.id.clone())
    }

    pub fn clear_contexts(&mut self) -> FrameInvalidation {
        let mut invalidation = FrameInvalidation::default();
        for frame in self.frames.values_mut() {
            if let Some(context) = frame.utility_context.take() {
                invalidation.contexts.push(context);
                invalidation.frame_ids.push(frame.id.clone());
            }
        }
        invalidation
    }

    fn add_cdp_subtree(&mut self, tree: &Page::FrameTree, parent_id: Option<FrameId>) {
        let frame_id = tree.frame.id.clone();
        self.attach(frame_id.clone(), parent_id);
        if let Some(children) = &tree.child_frames {
            for child in children {
                self.add_cdp_subtree(child, Some(frame_id.clone()));
            }
        }
    }

    fn collect_subtree(&self, frame_id: &str, output: &mut Vec<FrameId>) {
        let Some(frame) = self.frames.get(frame_id) else {
            return;
        };
        output.push(frame_id.to_string());
        for child in &frame.children {
            self.collect_subtree(child, output);
        }
    }

    fn remove_child(&mut self, parent_id: &str, frame_id: &str) {
        if let Some(parent) = self.frames.get_mut(parent_id) {
            parent.children.retain(|child| child != frame_id);
        }
    }
}

impl FrameInvalidation {
    fn extend(&mut self, other: Self) {
        self.frame_ids.extend(other.frame_ids);
        self.contexts.extend(other.contexts);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameLocatorError {
    StrictViolation { count: usize },
    ExpectedFrameElement,
    NoContentFrame,
    Handle(HandleError),
}

impl Display for FrameLocatorError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StrictViolation { count } => write!(
                formatter,
                "Strict mode violation: frame locator resolved to {count} iframe elements"
            ),
            Self::ExpectedFrameElement => {
                formatter.write_str("Frame locator resolved to an element that is not an iframe")
            }
            Self::NoContentFrame => {
                formatter.write_str("Frame locator iframe has no attached content frame")
            }
            Self::Handle(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for FrameLocatorError {}

impl From<HandleError> for FrameLocatorError {
    fn from(error: HandleError) -> Self {
        Self::Handle(error)
    }
}

impl WorldManager {
    pub fn resolve_frame_locator(
        &self,
        tab: &Tab,
        parent_frame_id: &str,
        owner_locator: Value,
        target_locator: Value,
    ) -> Result<Vec<ElementHandle>, FrameLocatorError> {
        let owners = self.call_injected_handles_in_frame(
            tab,
            parent_frame_id,
            "resolveAll",
            Value::Array(vec![owner_locator]),
        )?;
        let owner = strict_owner(owners)?;
        let is_frame = self.call_function_on(
            tab,
            &owner,
            "function() { return this instanceof HTMLIFrameElement || this instanceof HTMLFrameElement; }",
            Vec::new(),
        )?;
        if is_frame != Value::Bool(true) {
            let _ = self.release_handle(tab, &owner);
            return Err(FrameLocatorError::ExpectedFrameElement);
        }
        let frame_id = self.content_frame_for_handle(tab, &owner)?;
        let _ = self.release_handle(tab, &owner);
        let frame_id = frame_id.ok_or(FrameLocatorError::NoContentFrame)?;
        self.call_injected_handles_in_frame(
            tab,
            &frame_id,
            "resolveAll",
            Value::Array(vec![target_locator]),
        )
        .map_err(FrameLocatorError::from)
    }
}

fn strict_owner(mut owners: Vec<ElementHandle>) -> Result<ElementHandle, FrameLocatorError> {
    if owners.len() != 1 {
        return Err(FrameLocatorError::StrictViolation {
            count: owners.len(),
        });
    }
    Ok(owners.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn handle(id: &str) -> ElementHandle {
        ElementHandle {
            object_id: id.to_string(),
            backend_node_id: None,
            context_id: 1,
            frame_id: "main".to_string(),
        }
    }

    #[test]
    fn attach_detach_removes_descendants_and_contexts() {
        let mut tree = FrameTree::default();
        tree.attach("main".to_string(), None);
        tree.attach("outer".to_string(), Some("main".to_string()));
        tree.attach("inner".to_string(), Some("outer".to_string()));
        tree.set_utility_context("outer", None, 7);
        tree.set_utility_context("inner", None, 8);

        let invalidation = tree.detach("outer");

        assert_eq!(invalidation.frame_ids, vec!["outer", "inner"]);
        assert_eq!(
            invalidation
                .contexts
                .iter()
                .map(|context| context.context_id)
                .collect::<HashSet<_>>(),
            HashSet::from([7, 8])
        );
        assert_eq!(tree.frame_ids(), vec!["main"]);
    }

    #[test]
    fn navigation_invalidates_context_and_child_frames() {
        let mut tree = FrameTree::default();
        tree.attach("main".to_string(), None);
        tree.attach("child".to_string(), Some("main".to_string()));
        tree.set_utility_context("main", None, 3);
        tree.set_utility_context("child", None, 4);

        let invalidation = tree.navigate("main");

        assert_eq!(invalidation.frame_ids, vec!["child", "main"]);
        assert_eq!(tree.frame_ids(), vec!["main"]);
        assert_eq!(tree.frame("main").unwrap().generation, 1);
        assert_eq!(tree.frame("main").unwrap().utility_context, None);
    }

    #[test]
    fn process_swap_replaces_session_and_ignores_late_detach() {
        let mut tree = FrameTree::default();
        tree.attach("main".to_string(), None);
        tree.attach("child".to_string(), Some("main".to_string()));
        tree.process_swap(
            "child".to_string(),
            Some("main".to_string()),
            "old".to_string(),
        );
        tree.set_utility_context("child", Some("old".to_string()), 11);

        let invalidation = tree.process_swap(
            "child".to_string(),
            Some("main".to_string()),
            "new".to_string(),
        );
        let late = tree.detach_session("old");

        assert_eq!(invalidation.contexts[0].context_id, 11);
        assert_eq!(late, FrameInvalidation::default());
        assert_eq!(tree.frame_for_session("new").unwrap().id, "child");
    }

    #[test]
    fn context_destruction_is_scoped_by_session() {
        let mut tree = FrameTree::default();
        tree.attach("first".to_string(), None);
        tree.attach("second".to_string(), None);
        tree.set_utility_context("first", Some("one".to_string()), 7);
        tree.set_utility_context("second", Some("two".to_string()), 7);

        assert_eq!(
            tree.destroy_context(Some("two"), 7),
            Some("second".to_string())
        );
        assert!(tree.frame("first").unwrap().utility_context.is_some());
        assert!(tree.frame("second").unwrap().utility_context.is_none());
    }

    #[test]
    fn clearing_contexts_keeps_frame_topology() {
        let mut tree = FrameTree::default();
        tree.attach("main".to_string(), None);
        tree.attach("child".to_string(), Some("main".to_string()));
        tree.set_utility_context("main", None, 3);
        tree.set_utility_context("child", None, 4);

        let invalidation = tree.clear_contexts();

        assert_eq!(invalidation.contexts.len(), 2);
        assert_eq!(tree.frame_ids(), vec!["main", "child"]);
        assert!(tree.frame("main").unwrap().utility_context.is_none());
        assert!(tree.frame("child").unwrap().utility_context.is_none());
    }

    #[test]
    fn frame_locator_owner_is_strict() {
        let error = strict_owner(vec![handle("first"), handle("second")]).unwrap_err();
        assert_eq!(error, FrameLocatorError::StrictViolation { count: 2 });
        assert!(error.to_string().contains("resolved to 2 iframe elements"));
    }
}
