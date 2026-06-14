use std::collections::VecDeque;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedInput {
    pub id: u64,
    pub text: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueEditState {
    index: usize,
    draft: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputQueue {
    items: VecDeque<QueuedInput>,
    selected: Option<usize>,
    editing: Option<QueueEditState>,
    next_id: u64,
}

impl InputQueue {
    pub fn new() -> Self {
        Self {
            items: VecDeque::new(),
            selected: None,
            editing: None,
            next_id: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &VecDeque<QueuedInput> {
        &self.items
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected.filter(|index| *index < self.items.len())
    }

    pub fn editing_index(&self) -> Option<usize> {
        self.editing
            .as_ref()
            .map(|edit| edit.index)
            .filter(|index| *index < self.items.len())
    }

    pub fn is_editing(&self) -> bool {
        self.editing_index().is_some()
    }

    pub fn enqueue(&mut self, text: String, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.items.push_back(QueuedInput { id, text, params });
        id
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.selected = None;
        self.editing = None;
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    pub fn select_prev(&mut self) -> bool {
        if self.items.is_empty() || self.editing.is_some() {
            return false;
        }
        let next = match self.selected_index() {
            Some(0) => 0,
            Some(index) => index.saturating_sub(1),
            None => self.items.len() - 1,
        };
        self.selected = Some(next);
        true
    }

    pub fn select_next_or_clear(&mut self) -> bool {
        if self.items.is_empty() || self.editing.is_some() {
            return false;
        }
        let Some(index) = self.selected_index() else {
            return false;
        };
        if index + 1 < self.items.len() {
            self.selected = Some(index + 1);
        } else {
            self.selected = None;
        }
        true
    }

    pub fn begin_edit_selected(&mut self, draft: String) -> Option<String> {
        if self.editing.is_some() {
            return None;
        }
        let index = self.selected_index()?;
        let text = self.items.get(index)?.text.clone();
        self.editing = Some(QueueEditState { index, draft });
        Some(text)
    }

    pub fn finish_edit(&mut self, text: String) -> Option<String> {
        let edit = self.editing.take()?;
        if let Some(item) = self.items.get_mut(edit.index) {
            item.text = text;
            self.selected = Some(edit.index);
        }
        Some(edit.draft)
    }

    pub fn cancel_edit(&mut self) -> Option<String> {
        self.editing.take().map(|edit| edit.draft)
    }

    pub fn remove_selected(&mut self) -> Option<QueuedInput> {
        if self.editing.is_some() {
            return None;
        }
        let index = self.selected_index()?;
        let removed = self.items.remove(index)?;
        self.clamp_selection_after_remove(index);
        Some(removed)
    }

    pub fn pop_next_ready(&mut self) -> Option<QueuedInput> {
        if self.editing.is_some() {
            return None;
        }
        let item = self.items.pop_front()?;
        self.selected = self.selected_index().and_then(|index| index.checked_sub(1));
        Some(item)
    }

    fn clamp_selection_after_remove(&mut self, removed_index: usize) {
        if self.items.is_empty() {
            self.selected = None;
        } else if removed_index >= self.items.len() {
            self.selected = Some(self.items.len() - 1);
        } else {
            self.selected = Some(removed_index);
        }
    }
}

impl Default for InputQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queue_pops_in_insert_order() {
        let mut queue = InputQueue::new();
        queue.enqueue("one".to_string(), json!({"mode":"agent"}));
        queue.enqueue("two".to_string(), json!({}));

        assert_eq!(queue.pop_next_ready().unwrap().text, "one");
        assert_eq!(queue.pop_next_ready().unwrap().text, "two");
        assert!(queue.pop_next_ready().is_none());
    }

    #[test]
    fn selected_item_can_be_edited_with_draft_restore() {
        let mut queue = InputQueue::new();
        queue.enqueue("one".to_string(), json!({}));
        queue.enqueue("two".to_string(), json!({}));

        assert!(queue.select_prev());
        assert_eq!(
            queue.begin_edit_selected("draft".to_string()).as_deref(),
            Some("two")
        );
        assert_eq!(
            queue.finish_edit("two edited".to_string()).as_deref(),
            Some("draft")
        );
        assert_eq!(queue.items()[1].text, "two edited");
        assert_eq!(queue.selected_index(), Some(1));
    }

    #[test]
    fn selected_item_can_be_removed() {
        let mut queue = InputQueue::new();
        queue.enqueue("one".to_string(), json!({}));
        queue.enqueue("two".to_string(), json!({}));

        queue.select_prev();
        assert_eq!(queue.remove_selected().unwrap().text, "two");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.selected_index(), Some(0));
    }

    #[test]
    fn editing_item_blocks_auto_pop_until_finished() {
        let mut queue = InputQueue::new();
        queue.enqueue("one".to_string(), json!({}));
        queue.select_prev();
        queue.begin_edit_selected("draft".to_string());

        assert!(queue.pop_next_ready().is_none());
        queue.finish_edit("one edited".to_string());
        assert_eq!(queue.pop_next_ready().unwrap().text, "one edited");
    }
}
