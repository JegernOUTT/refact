use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::sessions::{display_title, short_chat_id, TrajectoryMeta, WorktreeMeta};

const UNKNOWN: &str = "unknown";
const DEFAULT_PAGE_SIZE: usize = 50;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HistoryGrouping {
    #[default]
    Day,
    Project,
    Task,
}

impl HistoryGrouping {
    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Project => "project",
            Self::Task => "task",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Day => Self::Project,
            Self::Project => Self::Task,
            Self::Task => Self::Day,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryAction {
    Resume,
    Fork,
    Rename,
    Archive,
}

impl HistoryAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Fork => "fork",
            Self::Rename => "rename",
            Self::Archive => "archive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryActionRequest {
    pub action: HistoryAction,
    pub chat_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryListItem {
    Group(String),
    Chat(usize),
}

#[derive(Debug, Clone)]
pub struct HistorySurface {
    trajectories: Vec<TrajectoryMeta>,
    filter: String,
    grouping: HistoryGrouping,
    selected: usize,
    total_count: Option<usize>,
    has_more: bool,
    filter_active: bool,
    detail_open: bool,
    detail_offset: usize,
}

impl HistorySurface {
    pub fn new(trajectories: Vec<TrajectoryMeta>) -> Self {
        let total_count = Some(trajectories.len());
        Self::with_paging(trajectories, total_count, false)
    }

    pub fn with_paging(
        mut trajectories: Vec<TrajectoryMeta>,
        total_count: Option<usize>,
        has_more: bool,
    ) -> Self {
        trajectories.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        Self {
            trajectories,
            filter: String::new(),
            grouping: HistoryGrouping::default(),
            selected: 0,
            total_count,
            has_more,
            filter_active: false,
            detail_open: false,
            detail_offset: 0,
        }
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn filter_active(&self) -> bool {
        self.filter_active
    }

    pub fn begin_filter(&mut self) {
        self.filter_active = true;
    }

    pub fn cancel_filter(&mut self) {
        self.filter_active = false;
    }

    pub fn grouping(&self) -> HistoryGrouping {
        self.grouping
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn detail_open(&self) -> bool {
        self.detail_open
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
        self.selected = 0;
        self.detail_offset = 0;
        self.clamp_selection();
    }

    pub fn push_filter(&mut self, ch: char) {
        self.filter.push(ch);
        self.selected = 0;
        self.detail_offset = 0;
        self.clamp_selection();
    }

    pub fn pop_filter(&mut self) {
        self.filter.pop();
        self.selected = 0;
        self.detail_offset = 0;
        self.clamp_selection();
    }

    pub fn cycle_grouping(&mut self) {
        self.grouping = self.grouping.next();
        self.selected = 0;
        self.detail_offset = 0;
        self.clamp_selection();
    }

    pub fn toggle_detail(&mut self) {
        self.detail_open = !self.detail_open;
        self.detail_offset = 0;
    }

    pub fn set_detail_open(&mut self, detail_open: bool) {
        self.detail_open = detail_open;
        self.detail_offset = 0;
    }

    pub fn scroll_detail_up(&mut self) {
        self.detail_offset = self.detail_offset.saturating_sub(1);
    }

    pub fn scroll_detail_down(&mut self) {
        self.detail_offset = self.detail_offset.saturating_add(1);
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.detail_offset = 0;
    }

    pub fn select_next(&mut self) {
        self.selected = self.selected.saturating_add(1);
        self.detail_offset = 0;
        self.clamp_selection();
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
        self.detail_offset = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.filtered_indices().len().saturating_sub(1);
        self.detail_offset = 0;
    }

    pub fn select_page_up(&mut self) {
        self.selected = self.selected.saturating_sub(DEFAULT_PAGE_SIZE / 5);
        self.detail_offset = 0;
    }

    pub fn select_page_down(&mut self) {
        self.selected = self.selected.saturating_add(DEFAULT_PAGE_SIZE / 5);
        self.detail_offset = 0;
        self.clamp_selection();
    }

    pub fn selected_trajectory(&self) -> Option<&TrajectoryMeta> {
        self.filtered_indices()
            .get(self.selected)
            .and_then(|index| self.trajectories.get(*index))
    }

    pub fn selected_action(&self, action: HistoryAction) -> Option<HistoryActionRequest> {
        let chat_id = self.selected_trajectory()?.id.clone();
        (!chat_id.trim().is_empty()).then_some(HistoryActionRequest { action, chat_id })
    }

    pub fn navigate_to_parent(&mut self) -> bool {
        let Some(parent_id) = self
            .selected_trajectory()
            .and_then(|trajectory| non_empty(trajectory.parent_id.as_deref()))
            .map(str::to_string)
        else {
            return false;
        };
        self.select_chat_id(&parent_id)
    }

    pub fn navigate_to_root(&mut self) -> bool {
        let Some(root_id) = self
            .selected_trajectory()
            .and_then(|trajectory| non_empty(trajectory.root_chat_id.as_deref()))
            .map(str::to_string)
        else {
            return false;
        };
        self.select_chat_id(&root_id)
    }

    pub fn list_items(&self) -> Vec<HistoryListItem> {
        let mut groups = BTreeMap::<String, Vec<usize>>::new();
        for index in self.filtered_indices() {
            groups
                .entry(group_key(&self.trajectories[index], self.grouping))
                .or_default()
                .push(index);
        }
        groups
            .into_iter()
            .flat_map(|(group, indexes)| {
                std::iter::once(HistoryListItem::Group(group))
                    .chain(indexes.into_iter().map(HistoryListItem::Chat))
            })
            .collect()
    }

    pub fn summary_line(&self) -> String {
        let loaded = self.trajectories.len();
        let filtered = self.filtered_indices().len();
        let count = match self.total_count {
            Some(total) if total > loaded => format!("{loaded}/{total} loaded"),
            Some(total) => format!("{total} chats"),
            None => format!("{loaded} chats"),
        };
        let beyond_first_page = loaded.saturating_sub(DEFAULT_PAGE_SIZE);
        let paging = if self.has_more {
            "more exist".to_string()
        } else if beyond_first_page > 0 {
            format!("{beyond_first_page} beyond first {DEFAULT_PAGE_SIZE}")
        } else {
            "all loaded".to_string()
        };
        let filter = if self.filter.is_empty() {
            "filter: all".to_string()
        } else {
            format!("filter: {} ({filtered})", self.filter)
        };
        format!(
            "History · {count} · {paging} · group: {} · {filter}",
            self.grouping.label()
        )
    }

    pub fn render_lines(&self, width: usize) -> Vec<String> {
        if self.detail_open {
            return self.detail_lines(width);
        }
        let mut lines = vec![self.summary_line()];
        let selected_id = self
            .selected_trajectory()
            .map(|trajectory| trajectory.id.as_str());
        for item in self.list_items() {
            match item {
                HistoryListItem::Group(group) => lines.push(format!("[{group}]")),
                HistoryListItem::Chat(index) => {
                    let trajectory = &self.trajectories[index];
                    let marker = (Some(trajectory.id.as_str()) == selected_id).then_some('›');
                    lines.push(compact_row(trajectory, marker, width));
                }
            }
        }
        if self.filtered_indices().is_empty() {
            lines.push("No chats match this filter".to_string());
        }
        lines.push(action_hint_line(width));
        lines
    }

    pub fn detail_lines(&self, width: usize) -> Vec<String> {
        let Some(trajectory) = self.selected_trajectory() else {
            return vec![self.summary_line(), "No chat selected".to_string()];
        };
        let metadata = metadata_lines(trajectory);
        let mut lines = vec![
            self.summary_line(),
            format!("Details · {}", display_title(&trajectory.title)),
        ];
        lines.extend(metadata.into_iter().skip(self.detail_offset));
        lines.push(action_hint_line(width));
        lines
    }

    fn filtered_indices(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_ascii_lowercase();
        self.trajectories
            .iter()
            .enumerate()
            .filter_map(|(index, trajectory)| {
                (needle.is_empty() || searchable_text(trajectory).contains(&needle))
                    .then_some(index)
            })
            .collect()
    }

    fn clamp_selection(&mut self) {
        self.selected = self
            .selected
            .min(self.filtered_indices().len().saturating_sub(1));
    }

    fn select_chat_id(&mut self, id: &str) -> bool {
        let Some(index) = self
            .trajectories
            .iter()
            .position(|trajectory| trajectory.id == id)
        else {
            return false;
        };
        self.filter.clear();
        self.selected = self
            .filtered_indices()
            .iter()
            .position(|filtered_index| *filtered_index == index)
            .unwrap_or_default();
        true
    }
}

pub fn surfaces_enabled_from_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub fn surfaces_enabled_from_env() -> bool {
    surfaces_enabled_from_value(std::env::var("REFACT_TUI_SURFACES").ok().as_deref())
}

pub fn metadata_lines(trajectory: &TrajectoryMeta) -> Vec<String> {
    let worktree = trajectory.worktree.as_ref();
    vec![
        field("id", non_empty(Some(&trajectory.id))),
        field("title", non_empty(Some(&trajectory.title))),
        field("created_at", non_empty(Some(&trajectory.created_at))),
        field("updated_at", non_empty(Some(&trajectory.updated_at))),
        field("model", non_empty(Some(&trajectory.model))),
        field("mode", non_empty(Some(&trajectory.mode))),
        number_field("message_count", trajectory.message_count),
        field("parent_id", non_empty(trajectory.parent_id.as_deref())),
        field("link_type", non_empty(trajectory.link_type.as_deref())),
        field("task_id", non_empty(trajectory.task_id.as_deref())),
        field("role", non_empty(trajectory.task_role.as_deref())),
        field("agent_id", non_empty(trajectory.agent_id.as_deref())),
        field("card_id", non_empty(trajectory.card_id.as_deref())),
        field(
            "session_state",
            non_empty(trajectory.session_state.as_deref()),
        ),
        field(
            "root_chat_id",
            non_empty(trajectory.root_chat_id.as_deref()),
        ),
        worktree_field("worktree.id", worktree, |worktree| worktree.id.as_deref()),
        worktree_field("worktree.kind", worktree, |worktree| {
            worktree.kind.as_deref()
        }),
        worktree_path_field("worktree.root", worktree, |worktree| worktree.root.as_ref()),
        worktree_path_field("worktree.source_workspace_root", worktree, |worktree| {
            worktree.source_workspace_root.as_ref()
        }),
        worktree_path_field("worktree.repo_root", worktree, |worktree| {
            worktree.repo_root.as_ref()
        }),
        worktree_field("worktree.branch", worktree, |worktree| {
            worktree.branch.as_deref()
        }),
        worktree_field("worktree.base_branch", worktree, |worktree| {
            worktree.base_branch.as_deref()
        }),
        worktree_field("worktree.base_commit", worktree, |worktree| {
            worktree.base_commit.as_deref()
        }),
        worktree_field("worktree.task_id", worktree, |worktree| {
            worktree.task_id.as_deref()
        }),
        worktree_field("worktree.card_id", worktree, |worktree| {
            worktree.card_id.as_deref()
        }),
        worktree_field("worktree.agent_id", worktree, |worktree| {
            worktree.agent_id.as_deref()
        }),
        bool_field(
            "worktree.enforce",
            worktree.and_then(|worktree| worktree.enforce),
        ),
        number_field("total_lines_added", trajectory.total_lines_added),
        number_field("total_lines_removed", trajectory.total_lines_removed),
        number_field("tasks_total", trajectory.tasks_total),
        number_field("tasks_done", trajectory.tasks_done),
        number_field("tasks_failed", trajectory.tasks_failed),
        number_field("total_prompt_tokens", trajectory.total_prompt_tokens),
        number_field(
            "total_completion_tokens",
            trajectory.total_completion_tokens,
        ),
        number_field("total_tokens", trajectory.total_tokens),
        number_field(
            "total_cache_read_tokens",
            trajectory.total_cache_read_tokens,
        ),
        number_field(
            "total_cache_creation_tokens",
            trajectory.total_cache_creation_tokens,
        ),
        cost_field(trajectory.total_cost_usd),
    ]
}

fn compact_row(trajectory: &TrajectoryMeta, marker: Option<char>, width: usize) -> String {
    let marker = marker
        .map(|marker| format!("{marker} "))
        .unwrap_or_default();
    let title = display_title(&trajectory.title);
    let time = unknown_or(non_empty(Some(&trajectory.updated_at)));
    let id = unknown_or(non_empty(Some(&trajectory.id)));
    if width < 60 {
        return format!("{marker}{title} · {time} · {}", short_chat_id(&id));
    }
    let cost = trajectory
        .total_cost_usd
        .map(|cost| format!("${cost:.4}"))
        .unwrap_or_else(|| UNKNOWN.to_string());
    let tokens = number_or_unknown(trajectory.total_tokens);
    let worktree = trajectory
        .worktree
        .as_ref()
        .and_then(|worktree| worktree.branch.as_deref())
        .or_else(|| {
            trajectory
                .worktree
                .as_ref()
                .and_then(|worktree| worktree.id.as_deref())
        })
        .and_then(|value| non_empty(Some(value)))
        .unwrap_or(UNKNOWN);
    format!(
        "{marker}{title} · {time} · ${cost} · tokens {tokens} · worktree {worktree} · {}",
        short_chat_id(&id)
    )
}

fn action_hint_line(width: usize) -> String {
    if width < 60 {
        "Enter resume · / filter · arrows".to_string()
    } else {
        "Enter resume · f fork · r rename · a archive · t details · / filter · g group · l parent · R root · Esc close".to_string()
    }
}

fn group_key(trajectory: &TrajectoryMeta, grouping: HistoryGrouping) -> String {
    match grouping {
        HistoryGrouping::Day => DateTime::parse_from_rfc3339(&trajectory.updated_at)
            .map(|date| date.with_timezone(&Utc).format("%Y-%m-%d").to_string())
            .unwrap_or_else(|_| "unknown day".to_string()),
        HistoryGrouping::Project => trajectory
            .worktree
            .as_ref()
            .and_then(|worktree| {
                worktree
                    .source_workspace_root
                    .as_ref()
                    .or(worktree.repo_root.as_ref())
            })
            .map(|path| path.display().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown project".to_string()),
        HistoryGrouping::Task => trajectory
            .task_id
            .as_deref()
            .or_else(|| {
                trajectory
                    .worktree
                    .as_ref()
                    .and_then(|worktree| worktree.task_id.as_deref())
            })
            .and_then(|value| non_empty(Some(value)))
            .unwrap_or("unknown task")
            .to_string(),
    }
}

fn searchable_text(trajectory: &TrajectoryMeta) -> String {
    metadata_lines(trajectory).join("\n").to_ascii_lowercase()
}

fn field(name: &str, value: Option<&str>) -> String {
    format!("{name}: {}", unknown_or(value))
}

fn worktree_field(
    name: &str,
    worktree: Option<&WorktreeMeta>,
    value: impl FnOnce(&WorktreeMeta) -> Option<&str>,
) -> String {
    field(
        name,
        worktree
            .and_then(value)
            .and_then(|value| non_empty(Some(value))),
    )
}

fn worktree_path_field(
    name: &str,
    worktree: Option<&WorktreeMeta>,
    value: impl FnOnce(&WorktreeMeta) -> Option<&std::path::PathBuf>,
) -> String {
    let value = worktree
        .and_then(value)
        .map(|value| value.display().to_string());
    field(name, value.as_deref())
}

fn number_field<T: std::fmt::Display>(name: &str, value: Option<T>) -> String {
    format!("{name}: {}", number_or_unknown(value))
}

fn bool_field(name: &str, value: Option<bool>) -> String {
    format!(
        "{name}: {}",
        value
            .map(|value| value.to_string())
            .unwrap_or_else(|| UNKNOWN.to_string())
    )
}

fn cost_field(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("total_cost_usd: ${value:.4}"),
        None => format!("total_cost_usd: {UNKNOWN}"),
    }
}

fn number_or_unknown<T: std::fmt::Display>(value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| UNKNOWN.to_string())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn unknown_or(value: Option<&str>) -> &str {
    value.unwrap_or(UNKNOWN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn trajectory(id: &str) -> TrajectoryMeta {
        TrajectoryMeta {
            id: id.to_string(),
            title: "History navigator".to_string(),
            created_at: "2026-08-25T09:00:00Z".to_string(),
            updated_at: "2026-08-25T10:00:00Z".to_string(),
            model: "gpt-5.6".to_string(),
            mode: "task_agent".to_string(),
            message_count: Some(8),
            parent_id: Some("parent-chat".to_string()),
            link_type: Some("fork".to_string()),
            task_id: Some("task-56".to_string()),
            task_role: Some("agent".to_string()),
            agent_id: Some("agent-56".to_string()),
            card_id: Some("T-56".to_string()),
            session_state: Some("idle".to_string()),
            root_chat_id: Some("root-chat".to_string()),
            worktree: Some(WorktreeMeta {
                id: Some("worktree-56".to_string()),
                kind: Some("task_agent".to_string()),
                root: Some(PathBuf::from("/tmp/worktree")),
                source_workspace_root: Some(PathBuf::from("/tmp/source")),
                repo_root: Some(PathBuf::from("/tmp/repo")),
                branch: Some("refact/task/T-56".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("abc123".to_string()),
                task_id: Some("task-56".to_string()),
                card_id: Some("T-56".to_string()),
                agent_id: Some("agent-56".to_string()),
                enforce: Some(true),
            }),
            total_lines_added: Some(100),
            total_lines_removed: Some(25),
            tasks_total: Some(4),
            tasks_done: Some(3),
            tasks_failed: Some(1),
            total_prompt_tokens: Some(1000),
            total_completion_tokens: Some(500),
            total_tokens: Some(1500),
            total_cache_read_tokens: Some(250),
            total_cache_creation_tokens: Some(75),
            total_cost_usd: Some(0.042),
        }
    }

    #[test]
    fn metadata_renders_every_trajectory_and_worktree_field() {
        let lines = metadata_lines(&trajectory("chat-56"));
        let text = lines.join("\n");
        for expected in [
            "id: chat-56",
            "title: History navigator",
            "created_at: 2026-08-25T09:00:00Z",
            "updated_at: 2026-08-25T10:00:00Z",
            "model: gpt-5.6",
            "mode: task_agent",
            "message_count: 8",
            "parent_id: parent-chat",
            "link_type: fork",
            "task_id: task-56",
            "role: agent",
            "agent_id: agent-56",
            "card_id: T-56",
            "session_state: idle",
            "root_chat_id: root-chat",
            "worktree.id: worktree-56",
            "worktree.kind: task_agent",
            "worktree.root: /tmp/worktree",
            "worktree.source_workspace_root: /tmp/source",
            "worktree.repo_root: /tmp/repo",
            "worktree.branch: refact/task/T-56",
            "worktree.base_branch: main",
            "worktree.base_commit: abc123",
            "worktree.task_id: task-56",
            "worktree.card_id: T-56",
            "worktree.agent_id: agent-56",
            "worktree.enforce: true",
            "total_lines_added: 100",
            "total_lines_removed: 25",
            "tasks_total: 4",
            "tasks_done: 3",
            "tasks_failed: 1",
            "total_prompt_tokens: 1000",
            "total_completion_tokens: 500",
            "total_tokens: 1500",
            "total_cache_read_tokens: 250",
            "total_cache_creation_tokens: 75",
            "total_cost_usd: $0.0420",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
    }

    #[test]
    fn missing_metadata_is_unknown_without_fabricated_cost() {
        let lines = metadata_lines(&TrajectoryMeta {
            id: "chat-unknown".to_string(),
            ..Default::default()
        });
        let text = lines.join("\n");
        assert!(text.contains("total_cost_usd: unknown"));
        assert!(!text.contains("$0.0000"));
        assert!(text.contains("total_tokens: unknown"));
        assert!(text.contains("worktree.branch: unknown"));
    }

    #[test]
    fn search_filter_narrows_by_all_rendered_metadata() {
        let mut chat_56 = trajectory("chat-56");
        chat_56.agent_id = Some("agent-56".to_string());
        chat_56.worktree.as_mut().unwrap().agent_id = Some("agent-56".to_string());
        let mut other = trajectory("chat-other");
        other.title = "Other".to_string();
        other.agent_id = Some("agent-other".to_string());
        other.worktree.as_mut().unwrap().agent_id = Some("agent-other".to_string());
        other.worktree.as_mut().unwrap().branch = Some("refact/task/T-99".to_string());
        let mut surface = HistorySurface::new(vec![chat_56, other]);

        assert!(searchable_text(&surface.trajectories[0]).contains("agent-other"));
        assert!(searchable_text(&surface.trajectories[1]).contains("agent-56"));

        surface.set_filter("agent-56");
        assert_eq!(surface.selected_trajectory().unwrap().id, "chat-56");
        surface.set_filter("T-99");
        assert_eq!(surface.selected_trajectory().unwrap().id, "chat-other");
        surface.set_filter("not-present");
        assert!(surface.selected_trajectory().is_none());
    }
    #[test]
    fn grouping_and_lineage_are_navigable() {
        let mut child = trajectory("child-chat");
        child.parent_id = Some("parent-chat".to_string());
        child.root_chat_id = Some("root-chat".to_string());
        let mut parent = trajectory("parent-chat");
        parent.parent_id = None;
        parent.root_chat_id = Some("root-chat".to_string());
        let root = trajectory("root-chat");
        let mut surface = HistorySurface::new(vec![child, parent, root]);
        surface.set_filter("child-chat");

        assert!(surface.navigate_to_parent());
        assert_eq!(surface.selected_trajectory().unwrap().id, "parent-chat");
        assert!(surface.navigate_to_root());
        assert_eq!(surface.selected_trajectory().unwrap().id, "root-chat");
        surface.set_filter("");
        surface.cycle_grouping();
        surface.cycle_grouping();
        assert!(surface
            .list_items()
            .iter()
            .any(|item| matches!(item, HistoryListItem::Group(group) if group == "task-56")));
    }

    #[test]
    fn detail_scroll_exposes_metadata_beyond_the_initial_rows() {
        let mut surface = HistorySurface::new(vec![trajectory("chat-56")]);
        surface.toggle_detail();
        assert!(surface
            .detail_lines(40)
            .iter()
            .any(|line| line.contains("parent_id")));
        for _ in 0..20 {
            surface.scroll_detail_down();
        }
        assert!(surface
            .detail_lines(40)
            .iter()
            .any(|line| line.contains("total_cost_usd")));
    }

    #[test]
    fn paging_status_and_selection_reach_chats_past_fifty() {
        let trajectories = (0..51)
            .map(|index| {
                let mut trajectory = trajectory(&format!("chat-{index}"));
                trajectory.updated_at = format!("2026-08-25T10:{index:02}:00Z");
                trajectory
            })
            .collect();
        let mut surface = HistorySurface::with_paging(trajectories, Some(52), true);

        surface.select_last();
        assert_eq!(surface.selected_trajectory().unwrap().id, "chat-0");
        assert!(surface.summary_line().contains("51/52 loaded"));
        assert!(surface.summary_line().contains("more exist"));
    }

    #[test]
    fn action_requests_target_the_selected_chat() {
        let mut surface = HistorySurface::new(vec![trajectory("chat-56")]);
        for action in [
            HistoryAction::Resume,
            HistoryAction::Fork,
            HistoryAction::Rename,
            HistoryAction::Archive,
        ] {
            assert_eq!(
                surface.selected_action(action),
                Some(HistoryActionRequest {
                    action,
                    chat_id: "chat-56".to_string(),
                })
            );
        }
        surface.toggle_detail();
        assert!(surface
            .detail_lines(40)
            .iter()
            .any(|line| line.contains("parent_id")));
        assert!(surface
            .render_lines(40)
            .iter()
            .any(|line| line.contains("Enter resume")));
    }

    #[test]
    fn surface_gate_accepts_only_truthy_values() {
        for value in [Some("1"), Some("true"), Some("YES"), Some("on")] {
            assert!(surfaces_enabled_from_value(value));
        }
        for value in [None, Some("0"), Some("false"), Some("anything")] {
            assert!(!surfaces_enabled_from_value(value));
        }
    }
}
