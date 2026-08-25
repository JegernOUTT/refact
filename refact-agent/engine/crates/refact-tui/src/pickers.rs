use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Model,
    Mode,
    SlashCommand,
    FileMention,
    Session,
    Permissions,
    Reasoning,
    Theme,
    ProviderLogout,
    CompetitorImport,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModeThreadDefaults {
    pub include_project_info: Option<bool>,
    pub checkpoints_enabled: Option<bool>,
    pub auto_approve_editing_tools: Option<bool>,
    pub auto_approve_dangerous_commands: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModePickerItem {
    pub item: PickerItem,
    pub group: String,
    pub tags: Vec<String>,
    pub order: i32,
    pub is_overlay: bool,
    pub is_current: bool,
    pub tools_count: Option<usize>,
    pub thread_defaults: ModeThreadDefaults,
}

impl ModePickerItem {
    pub fn auto_approval_badge(&self) -> Option<&'static str> {
        match (
            self.thread_defaults.auto_approve_editing_tools,
            self.thread_defaults.auto_approve_dangerous_commands,
        ) {
            (_, Some(true)) => Some("! edits + dangerous commands auto-approved"),
            (Some(true), _) => Some("! edits auto-approved"),
            _ => None,
        }
    }

    pub fn picker_description(&self) -> String {
        let mut details = Vec::new();
        if !self.item.description.trim().is_empty() {
            details.push(self.item.description.clone());
        }
        let tools = self
            .tools_count
            .map(|count| format!("{count} tools"))
            .unwrap_or_else(|| "tool count unknown".to_string());
        let defaults = [
            default_label("project info", self.thread_defaults.include_project_info),
            default_label("checkpoints", self.thread_defaults.checkpoints_enabled),
            default_label(
                "file edits auto-approved",
                self.thread_defaults.auto_approve_editing_tools,
            ),
            default_label(
                "dangerous commands auto-approved",
                self.thread_defaults.auto_approve_dangerous_commands,
            ),
        ]
        .join(", ");
        details.push(format!("{tools} · defaults: {defaults}"));
        details.join(" · ")
    }
}

fn default_label(label: &str, value: Option<bool>) -> String {
    match value {
        Some(true) => format!("{label} on"),
        Some(false) => format!("{label} off"),
        None => format!("{label} unknown"),
    }
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
    mode_items: Vec<ModePickerItem>,
    pub filter: String,
    pub selected: usize,
    selection_mode: PickerSelectionMode,
    selected_ids: Vec<String>,
}

const PAGE_STEP: usize = 10;

impl PickerState {
    pub fn new(kind: PickerKind, items: Vec<PickerItem>) -> Self {
        Self::with_selection_mode(kind, items, PickerSelectionMode::Single)
    }

    pub fn multi(kind: PickerKind, items: Vec<PickerItem>) -> Self {
        Self::with_selection_mode(kind, items, PickerSelectionMode::Multi)
    }

    pub fn multi_with_selected(
        kind: PickerKind,
        items: Vec<PickerItem>,
        selected_ids: Vec<String>,
    ) -> Self {
        let mut picker = Self::with_selection_mode(kind, items, PickerSelectionMode::Multi);
        picker.selected_ids = selected_ids;
        picker
    }

    pub fn modes(mut items: Vec<ModePickerItem>, current_id: Option<&str>) -> Self {
        for item in &mut items {
            item.is_current = current_id.is_some_and(|current| current == item.item.id);
        }
        let picker_items = items.iter().map(|item| item.item.clone()).collect();
        let mut picker =
            Self::with_selection_mode(PickerKind::Mode, picker_items, PickerSelectionMode::Single);
        picker.mode_items = items;
        if let Some(current_id) = current_id {
            picker.select_item_id(current_id);
        }
        picker
    }

    fn with_selection_mode(
        kind: PickerKind,
        items: Vec<PickerItem>,
        selection_mode: PickerSelectionMode,
    ) -> Self {
        Self {
            kind,
            items,
            mode_items: Vec::new(),
            filter: String::new(),
            selected: 0,
            selection_mode,
            selected_ids: Vec::new(),
        }
    }

    pub fn items(&self) -> &[PickerItem] {
        &self.items
    }

    pub fn has_mode_items(&self) -> bool {
        !self.mode_items.is_empty()
    }

    pub fn filtered_mode_items(&self) -> Vec<ModePickerItem> {
        self.filtered_items()
            .into_iter()
            .filter_map(|item| {
                self.mode_items
                    .iter()
                    .find(|mode| mode.item.id == item.id)
                    .cloned()
            })
            .collect()
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
            PickerKind::Session => "sessions",
            PickerKind::Permissions => "permissions",
            PickerKind::Reasoning => "reasoning",
            PickerKind::Theme => "themes",
            PickerKind::ProviderLogout => "providers",
            PickerKind::CompetitorImport => "imports",
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

    pub fn select_item_id(&mut self, id: &str) {
        let Some(index) = self.filtered_items().iter().position(|item| item.id == id) else {
            return;
        };
        self.selected = index;
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

    pub fn push_filter_text(&mut self, text: &str) {
        self.filter.push_str(text);
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

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.filtered_items().len().saturating_sub(1);
    }

    pub fn select_page_up(&mut self) {
        self.selected = self.selected.saturating_sub(PAGE_STEP);
    }

    pub fn select_page_down(&mut self) {
        self.selected = self.selected.saturating_add(PAGE_STEP);
        self.clamp_selection();
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
    for models in [
        caps.get("chat_models"),
        caps.get("models").and_then(|models| models.get("chat")),
        caps.get("available_models"),
    ] {
        collect_model_items(models, &mut out);
    }
    out.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));
    out
}

fn collect_model_items(models: Option<&Value>, out: &mut Vec<PickerItem>) {
    match models {
        Some(Value::Object(models)) => {
            for (id, model) in models {
                push_model_item(out, id, model);
            }
        }
        Some(Value::Array(models)) => {
            for model in models {
                if let Some(id) = model.get("id").and_then(Value::as_str) {
                    push_model_item(out, id, model);
                }
            }
        }
        _ => {}
    }
}

fn push_model_item(out: &mut Vec<PickerItem>, id: &str, value: &Value) {
    if id.trim().is_empty() || out.iter().any(|item| item.id == id) {
        return;
    }
    let title = value
        .get("name")
        .or_else(|| value.get("display_name"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(id)
        .to_string();
    out.push(PickerItem {
        id: id.to_string(),
        title,
        description: model_description(value),
    });
}

fn model_description(value: &Value) -> String {
    let mut details = Vec::new();
    if let Some(description) = value
        .get("description")
        .or_else(|| value.get("provider"))
        .or_else(|| value.get("selected_provider"))
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        details.push(description.to_string());
    }
    details.push(
        model_context_window(value)
            .map(|tokens| format!("{} context", format_token_count(tokens)))
            .unwrap_or_else(|| "context unknown".to_string()),
    );
    details.push(model_reasoning_label(value));
    details.push(model_pricing_label(value));
    details.join(" · ")
}

fn model_context_window(value: &Value) -> Option<u64> {
    [
        "n_ctx",
        "context_window",
        "context_window_tokens",
        "context_length",
        "max_context_window_tokens",
        "max_prompt_tokens",
        "max_model_len",
    ]
    .into_iter()
    .find_map(|key| value.get(key).and_then(Value::as_u64))
}

fn model_reasoning_label(value: &Value) -> String {
    let effort = value
        .get("reasoning_effort_options")
        .and_then(Value::as_array)
        .is_some_and(|options| !options.is_empty());
    let budget = value
        .get("supports_thinking_budget")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let adaptive = value
        .get("supports_adaptive_thinking_budget")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if effort || budget || adaptive {
        "reasoning supported".to_string()
    } else {
        "reasoning unavailable".to_string()
    }
}

fn model_pricing_label(value: &Value) -> String {
    let Some(pricing) = value.get("pricing").and_then(Value::as_object) else {
        return "pricing unknown".to_string();
    };
    let input = pricing.get("prompt").and_then(Value::as_f64);
    let output = pricing.get("generated").and_then(Value::as_f64);
    match (input, output) {
        (Some(input), Some(output)) => {
            format!(
                "${} in / ${} out per 1M",
                compact_price(input),
                compact_price(output)
            )
        }
        _ => "pricing unknown".to_string(),
    }
}

fn compact_price(value: f64) -> String {
    format!("{value:.4}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn format_token_count(tokens: u64) -> String {
    if tokens >= 1_000_000 && tokens % 1_000_000 == 0 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 && tokens % 1_000 == 0 {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

pub fn mode_items_from_response(response: &Value) -> Vec<ModePickerItem> {
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
            let tags = mode
                .get("ui")
                .and_then(|ui| ui.get("tags"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            let is_overlay = mode
                .get("is_overlay")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || mode.get("kind").and_then(Value::as_str) == Some("overlay")
                || mode.get("base").and_then(Value::as_str) == Some("agent")
                || tags.iter().any(|tag| {
                    matches!(
                        tag.to_ascii_lowercase().as_str(),
                        "overlay" | "model-compat" | "model-compatibility"
                    )
                });
            let group = if is_overlay {
                "Model compatibility overlays".to_string()
            } else {
                tags.first()
                    .map(|tag| title_case_tag(tag))
                    .unwrap_or_else(|| "Other modes".to_string())
            };
            Some(ModePickerItem {
                item: PickerItem {
                    id,
                    title,
                    description,
                },
                group,
                tags,
                order: mode
                    .get("ui")
                    .and_then(|ui| ui.get("order"))
                    .and_then(Value::as_i64)
                    .and_then(|order| i32::try_from(order).ok())
                    .unwrap_or(i32::MAX),
                is_overlay,
                is_current: false,
                tools_count: mode
                    .get("tools_count")
                    .and_then(Value::as_u64)
                    .and_then(|count| usize::try_from(count).ok()),
                thread_defaults: mode_thread_defaults(mode),
            })
        })
        .collect::<Vec<_>>();
    let group_orders = out
        .iter()
        .fold(HashMap::<String, i32>::new(), |mut orders, item| {
            orders
                .entry(item.group.clone())
                .and_modify(|order| *order = (*order).min(item.order))
                .or_insert(item.order);
            orders
        });
    out.sort_by(|left, right| {
        left.is_overlay
            .cmp(&right.is_overlay)
            .then_with(|| group_orders[&left.group].cmp(&group_orders[&right.group]))
            .then_with(|| left.group.cmp(&right.group))
            .then_with(|| left.order.cmp(&right.order))
            .then_with(|| left.item.title.cmp(&right.item.title))
            .then_with(|| left.item.id.cmp(&right.item.id))
    });
    out
}

fn mode_thread_defaults(mode: &Value) -> ModeThreadDefaults {
    let defaults = mode.get("thread_defaults");
    ModeThreadDefaults {
        include_project_info: defaults
            .and_then(|defaults| defaults.get("include_project_info"))
            .and_then(Value::as_bool),
        checkpoints_enabled: defaults
            .and_then(|defaults| defaults.get("checkpoints_enabled"))
            .and_then(Value::as_bool),
        auto_approve_editing_tools: defaults
            .and_then(|defaults| defaults.get("auto_approve_editing_tools"))
            .and_then(Value::as_bool),
        auto_approve_dangerous_commands: defaults
            .and_then(|defaults| defaults.get("auto_approve_dangerous_commands"))
            .and_then(Value::as_bool),
    }
}

fn title_case_tag(tag: &str) -> String {
    let mut chars = tag.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!("{}{}", first.to_uppercase(), chars.as_str())
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
    fn picker_navigation_supports_home_end_and_paging() {
        let items = (0..25)
            .map(|index| PickerItem {
                id: index.to_string(),
                title: index.to_string(),
                description: String::new(),
            })
            .collect();
        let mut picker = PickerState::new(PickerKind::Model, items);

        picker.select_last();
        assert_eq!(picker.selected, 24);
        picker.select_page_up();
        assert_eq!(picker.selected, 14);
        picker.select_page_down();
        assert_eq!(picker.selected, 24);
        picker.select_first();
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn multi_select_returns_original_item_order() {
        let mut picker = PickerState::multi(
            PickerKind::Permissions,
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
        assert!(items[0].description.contains("context unknown"));
        assert!(items[0].description.contains("pricing unknown"));
    }

    #[test]
    fn model_picker_includes_capabilities_and_tolerates_missing_pricing() {
        let caps = serde_json::json!({
            "chat_models": {
                "priced": {
                    "name": "Priced model",
                    "n_ctx": 128000,
                    "reasoning_effort_options": ["low"],
                    "pricing": {"prompt": 3.0, "generated": 15.0},
                },
                "unknown": {"n_ctx": 8192},
            },
        });
        let items = model_items_from_caps(&caps);
        let priced = items.iter().find(|item| item.id == "priced").unwrap();
        assert!(priced.description.contains("128K context"));
        assert!(priced.description.contains("reasoning supported"));
        assert!(priced.description.contains("$3 in / $15 out per 1M"));
        let unknown = items.iter().find(|item| item.id == "unknown").unwrap();
        assert!(unknown.description.contains("pricing unknown"));
    }

    #[test]
    fn mode_items_group_by_tags_order_and_separate_overlays() {
        let modes = serde_json::json!({"modes": [
            {
                "id": "review", "title": "Review", "description": "Inspect changes",
                "tools_count": 4,
                "thread_defaults": {"auto_approve_editing_tools": false},
                "ui": {"order": 30, "tags": ["analysis"]}
            },
            {
                "id": "ask", "title": "Ask", "description": "Answer questions",
                "tools_count": 1,
                "thread_defaults": {"auto_approve_editing_tools": false},
                "ui": {"order": 5, "tags": ["chat"]}
            },
            {
                "id": "compat", "title": "Compatibility", "description": "Patch Agent",
                "base": "agent", "tools_count": 8,
                "thread_defaults": {"auto_approve_editing_tools": true},
                "ui": {"order": 1, "tags": []}
            }
        ]});

        let items = mode_items_from_response(&modes);

        assert_eq!(
            items
                .iter()
                .map(|item| item.item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ask", "review", "compat"]
        );
        assert_eq!(items[0].group, "Chat");
        assert_eq!(items[1].group, "Analysis");
        assert_eq!(items[2].group, "Model compatibility overlays");
        assert!(items[2].is_overlay);
    }

    #[test]
    fn mode_items_badge_only_resolved_auto_approval_defaults() {
        let modes = serde_json::json!({"modes": [
            {
                "id": "safe", "title": "Safe", "thread_defaults": {
                    "auto_approve_editing_tools": false,
                    "auto_approve_dangerous_commands": false
                }, "ui": {"order": 1, "tags": []}
            },
            {
                "id": "edit", "title": "Edit", "thread_defaults": {
                    "auto_approve_editing_tools": true,
                    "auto_approve_dangerous_commands": false
                }, "ui": {"order": 2, "tags": []}
            },
            {
                "id": "danger", "title": "Danger", "thread_defaults": {
                    "auto_approve_editing_tools": true,
                    "auto_approve_dangerous_commands": true
                }, "ui": {"order": 3, "tags": []}
            }
        ]});

        let items = mode_items_from_response(&modes);
        let badged = items
            .iter()
            .filter(|item| item.auto_approval_badge().is_some())
            .count();
        let resolved_auto_approving = items
            .iter()
            .filter(|item| {
                item.thread_defaults.auto_approve_editing_tools == Some(true)
                    || item.thread_defaults.auto_approve_dangerous_commands == Some(true)
            })
            .count();

        assert_eq!(badged, resolved_auto_approving);
        assert_eq!(badged, 2);
        assert_eq!(
            items
                .iter()
                .find(|item| item.item.id == "danger")
                .unwrap()
                .auto_approval_badge(),
            Some("! edits + dangerous commands auto-approved")
        );
    }

    #[test]
    fn mode_picker_marks_the_current_mode() {
        let modes = serde_json::json!({"modes": [
            {"id": "ask", "title": "Ask", "ui": {"order": 1, "tags": []}},
            {"id": "agent", "title": "Agent", "ui": {"order": 2, "tags": []}}
        ]});
        let picker = PickerState::modes(mode_items_from_response(&modes), Some("agent"));

        let current = picker
            .filtered_mode_items()
            .into_iter()
            .filter(|item| item.is_current)
            .collect::<Vec<_>>();

        assert_eq!(current.len(), 1);
        assert_eq!(current[0].item.id, "agent");
    }
}
