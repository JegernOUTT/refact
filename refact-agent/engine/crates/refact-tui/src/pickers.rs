use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerKind {
    Model,
    Mode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    pub id: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerState {
    pub kind: PickerKind,
    items: Vec<PickerItem>,
    pub filter: String,
    pub selected: usize,
}

impl PickerState {
    pub fn new(kind: PickerKind, items: Vec<PickerItem>) -> Self {
        Self {
            kind,
            items,
            filter: String::new(),
            selected: 0,
        }
    }

    pub fn items(&self) -> &[PickerItem] {
        &self.items
    }

    pub fn filtered_items(&self) -> Vec<PickerItem> {
        if self.filter.trim().is_empty() {
            return self.items.clone();
        }
        let needle = self.filter.to_ascii_lowercase();
        self.items
            .iter()
            .filter(|item| {
                item.id.to_ascii_lowercase().contains(&needle)
                    || item.title.to_ascii_lowercase().contains(&needle)
                    || item.description.to_ascii_lowercase().contains(&needle)
            })
            .cloned()
            .collect()
    }

    pub fn selected_item(&self) -> Option<PickerItem> {
        self.filtered_items().get(self.selected).cloned()
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
    fn parses_models_from_caps() {
        let caps =
            serde_json::json!({"chat_models": {"m1": {"name": "Model One", "provider": "p"}}});
        let items = model_items_from_caps(&caps);
        assert_eq!(items[0].id, "m1");
        assert_eq!(items[0].title, "Model One");
    }
}
