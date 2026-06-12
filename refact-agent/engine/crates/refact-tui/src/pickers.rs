use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Model,
    Mode,
    SlashCommand,
    FileMention,
    MultiSelect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerSelectionMode {
    Single,
    Multi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    pub id: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAccept {
    Single(Option<PickerItem>),
    Multi(Vec<PickerItem>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerState {
    pub kind: PickerKind,
    items: Vec<PickerItem>,
    pub filter: String,
    pub selected: usize,
    selection_mode: PickerSelectionMode,
    selected_ids: Vec<String>,
}

impl PickerState {
    pub fn new(kind: PickerKind, items: Vec<PickerItem>) -> Self {
        Self::with_selection_mode(kind, items, PickerSelectionMode::Single)
    }

    pub fn multi(kind: PickerKind, items: Vec<PickerItem>) -> Self {
        Self::with_selection_mode(kind, items, PickerSelectionMode::Multi)
    }

    fn with_selection_mode(
        kind: PickerKind,
        items: Vec<PickerItem>,
        selection_mode: PickerSelectionMode,
    ) -> Self {
        Self {
            kind,
            items,
            filter: String::new(),
            selected: 0,
            selection_mode,
            selected_ids: Vec::new(),
        }
    }

    pub fn items(&self) -> &[PickerItem] {
        &self.items
    }

    pub fn selection_mode(&self) -> PickerSelectionMode {
        self.selection_mode
    }

    pub fn is_multi(&self) -> bool {
        self.selection_mode == PickerSelectionMode::Multi
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            PickerKind::Model => "models",
            PickerKind::Mode => "modes",
            PickerKind::SlashCommand => "commands",
            PickerKind::FileMention => "files",
            PickerKind::MultiSelect => "select",
        }
    }

    pub fn filtered_items(&self) -> Vec<PickerItem> {
        let mut matched = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                match_rank(item, &self.filter).map(|rank| (rank, index, item))
            })
            .collect::<Vec<_>>();
        matched.sort_by(|(left_rank, left_index, _), (right_rank, right_index, _)| {
            left_rank
                .cmp(right_rank)
                .then_with(|| left_index.cmp(right_index))
        });
        matched
            .into_iter()
            .map(|(_, _, item)| item.clone())
            .collect()
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
        self.selected = 0;
        self.clamp_selection();
    }

    pub fn selected_item(&self) -> Option<PickerItem> {
        self.filtered_items().get(self.selected).cloned()
    }

    pub fn selected_items(&self) -> Vec<PickerItem> {
        self.items
            .iter()
            .filter(|item| self.selected_ids.iter().any(|id| id == &item.id))
            .cloned()
            .collect()
    }

    pub fn selected_count(&self) -> usize {
        self.selected_ids.len()
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selected_ids.iter().any(|selected| selected == id)
    }

    pub fn clamp_selection(&mut self) {
        let len = self.filtered_items().len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }

    pub fn push_filter(&mut self, ch: char) {
        self.filter.push(ch);
        self.selected = 0;
        self.clamp_selection();
    }

    pub fn pop_filter(&mut self) {
        self.filter.pop();
        self.clamp_selection();
    }

    pub fn select_next(&mut self) {
        self.selected = self.selected.saturating_add(1);
        self.clamp_selection();
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn toggle_selected(&mut self) {
        if self.selection_mode != PickerSelectionMode::Multi {
            return;
        }
        let Some(item) = self.selected_item() else {
            return;
        };
        if let Some(index) = self.selected_ids.iter().position(|id| id == &item.id) {
            self.selected_ids.remove(index);
        } else {
            self.selected_ids.push(item.id);
        }
    }

    pub fn accept(&self) -> PickerAccept {
        match self.selection_mode {
            PickerSelectionMode::Single => PickerAccept::Single(self.selected_item()),
            PickerSelectionMode::Multi => PickerAccept::Multi(self.selected_items()),
        }
    }
}

fn match_rank(item: &PickerItem, filter: &str) -> Option<usize> {
    let needle = filter.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Some(0);
    }
    let names = [
        item.id.to_ascii_lowercase(),
        item.title.to_ascii_lowercase(),
        item.title.trim_start_matches('/').to_ascii_lowercase(),
    ];
    let description = item.description.to_ascii_lowercase();
    if names.iter().any(|field| field.starts_with(&needle)) {
        return Some(0);
    }
    if names.iter().any(|field| field.contains(&needle)) {
        return Some(1);
    }
    if description.starts_with(&needle) {
        return Some(2);
    }
    if description.contains(&needle) {
        return Some(3);
    }
    if let Some(length) = names
        .iter()
        .filter(|field| fuzzy_subsequence_match(field, &needle))
        .map(String::len)
        .min()
    {
        return Some(400 + length);
    }
    fuzzy_subsequence_match(&description, &needle).then_some(500 + description.len())
}

fn fuzzy_subsequence_match(field: &str, needle: &str) -> bool {
    let mut chars = needle.chars();
    let Some(mut wanted) = chars.next() else {
        return true;
    };
    for ch in field.chars() {
        if ch == wanted {
            match chars.next() {
                Some(next) => wanted = next,
                None => return true,
            }
        }
    }
    false
}

pub fn model_items_from_caps(caps: &Value) -> Vec<PickerItem> {
    let mut out = Vec::new();
    if let Some(models) = caps.get("chat_models").and_then(Value::as_object) {
        for (id, value) in models {
            let title = value
                .get("name")
                .or_else(|| value.get("id"))
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_string();
            let description = value
                .get("description")
                .or_else(|| value.get("provider"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            out.push(PickerItem {
                id: id.clone(),
                title,
                description,
            });
        }
    }
    out.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));
    out
}

pub fn mode_items_from_response(response: &Value) -> Vec<PickerItem> {
    let mut out = response
        .get("modes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|mode| {
            let id = mode.get("id").and_then(Value::as_str)?.to_string();
            let title = mode
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string();
            let description = mode
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(PickerItem {
                id,
                title,
                description,
            })
        })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));
    out
}

pub fn file_mention_items_from_completions(completions: Vec<String>) -> Vec<PickerItem> {
    let mut out = Vec::new();
    for completion in completions {
        let path = completion.trim().trim_start_matches('@').trim();
        if path.is_empty() || completion.trim_start().starts_with('/') {
            continue;
        }
        if out.iter().any(|item: &PickerItem| item.id == path) {
            continue;
        }
        out.push(PickerItem {
            id: path.to_string(),
            title: path.to_string(),
            description: "file mention".to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_filter_matches_id_title_and_description() {
        let mut picker = PickerState::new(
            PickerKind::Model,
            vec![
                PickerItem {
                    id: "a".to_string(),
                    title: "Alpha".to_string(),
                    description: "fast".to_string(),
                },
                PickerItem {
                    id: "b".to_string(),
                    title: "Beta".to_string(),
                    description: "careful reasoning".to_string(),
                },
            ],
        );
        picker.filter = "reason".to_string();
        assert_eq!(picker.filtered_items()[0].id, "b");
    }

    #[test]
    fn picker_filter_prefers_prefix_before_fuzzy() {
        let mut picker = PickerState::new(
            PickerKind::SlashCommand,
            vec![
                PickerItem {
                    id: "review".to_string(),
                    title: "/review".to_string(),
                    description: "workflow".to_string(),
                },
                PickerItem {
                    id: "raw".to_string(),
                    title: "/raw".to_string(),
                    description: "inspect response wires".to_string(),
                },
            ],
        );
        picker.filter = "rw".to_string();
        assert_eq!(picker.filtered_items()[0].id, "raw");
        picker.filter = "rv".to_string();
        assert_eq!(picker.filtered_items()[0].id, "review");
    }

    #[test]
    fn picker_navigation_clamps_to_filtered_items() {
        let mut picker = PickerState::new(
            PickerKind::SlashCommand,
            vec![
                PickerItem {
                    id: "new".to_string(),
                    title: "/new".to_string(),
                    description: String::new(),
                },
                PickerItem {
                    id: "model".to_string(),
                    title: "/model".to_string(),
                    description: String::new(),
                },
            ],
        );
        picker.select_next();
        picker.select_next();
        assert_eq!(picker.selected, 1);
        picker.push_filter('n');
        assert_eq!(picker.selected, 0);
        assert_eq!(picker.selected_item().unwrap().id, "new");
    }

    #[test]
    fn multi_select_returns_original_item_order() {
        let mut picker = PickerState::multi(
            PickerKind::MultiSelect,
            vec![
                PickerItem {
                    id: "a".to_string(),
                    title: "Alpha".to_string(),
                    description: String::new(),
                },
                PickerItem {
                    id: "b".to_string(),
                    title: "Beta".to_string(),
                    description: String::new(),
                },
                PickerItem {
                    id: "c".to_string(),
                    title: "Gamma".to_string(),
                    description: String::new(),
                },
            ],
        );
        picker.selected = 2;
        picker.toggle_selected();
        picker.selected = 0;
        picker.toggle_selected();
        let accepted = match picker.accept() {
            PickerAccept::Multi(items) => items,
            other => panic!("unexpected accept: {other:?}"),
        };
        assert_eq!(
            accepted.into_iter().map(|item| item.id).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }

    #[test]
    fn parses_file_mentions_from_at_completions() {
        let items = file_mention_items_from_completions(vec![
            "@src/main.rs ".to_string(),
            "/model".to_string(),
            "@src/main.rs".to_string(),
        ]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "src/main.rs");
    }

    #[test]
    fn parses_models_from_caps() {
        let caps =
            serde_json::json!({"chat_models": {"m1": {"name": "Model One", "provider": "p"}}});
        let items = model_items_from_caps(&caps);
        assert_eq!(items[0].id, "m1");
        assert_eq!(items[0].title, "Model One");
    }
}
