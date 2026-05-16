use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMeta {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub status: TaskStatus,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub cards_total: usize,
    #[serde(default)]
    pub cards_done: usize,
    #[serde(default)]
    pub cards_failed: usize,
    #[serde(default)]
    pub agents_active: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_model: Option<String>,
    #[serde(default)]
    pub is_name_generated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_agents_summary_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_session_state: Option<String>,
}

fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Planning,
    Active,
    Paused,
    Completed,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBoard {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub rev: u64,
    #[serde(default = "default_columns")]
    pub columns: Vec<BoardColumn>,
    #[serde(default)]
    pub cards: Vec<BoardCard>,
}

fn default_columns() -> Vec<BoardColumn> {
    vec![
        BoardColumn {
            id: "planned".into(),
            title: "Planned".into(),
        },
        BoardColumn {
            id: "doing".into(),
            title: "Doing".into(),
        },
        BoardColumn {
            id: "done".into(),
            title: "Done".into(),
        },
        BoardColumn {
            id: "failed".into(),
            title: "Failed".into(),
        },
    ]
}

impl Default for TaskBoard {
    fn default() -> Self {
        Self {
            schema_version: 1,
            rev: 0,
            columns: default_columns(),
            cards: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardColumn {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardCard {
    pub id: String,
    pub title: String,
    pub column: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub instructions: String,
    pub assignee: Option<String>,
    pub agent_chat_id: Option<String>,
    #[serde(default)]
    pub status_updates: Vec<StatusUpdate>,
    pub final_report: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_at: Option<String>,
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_worktree_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_files: Vec<String>,
}

fn default_priority() -> String {
    "P1".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusUpdate {
    pub timestamp: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyCardsResult {
    pub ready: Vec<String>,
    pub blocked: Vec<String>,
    pub in_progress: Vec<String>,
    pub completed: Vec<String>,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryInfo {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
}

impl TaskBoard {
    pub fn get_ready_cards(&self) -> ReadyCardsResult {
        let mut ready = vec![];
        let mut blocked = vec![];
        let mut in_progress = vec![];
        let mut completed = vec![];
        let mut failed = vec![];

        let done_cards: std::collections::HashSet<_> = self
            .cards
            .iter()
            .filter(|c| c.column == "done")
            .map(|c| c.id.as_str())
            .collect();

        for card in &self.cards {
            match card.column.as_str() {
                "done" => completed.push(card.id.clone()),
                "failed" => failed.push(card.id.clone()),
                "doing" => in_progress.push(card.id.clone()),
                "planned" => {
                    let deps_satisfied = card
                        .depends_on
                        .iter()
                        .all(|dep| done_cards.contains(dep.as_str()));
                    if deps_satisfied {
                        ready.push(card.id.clone());
                    } else {
                        blocked.push(card.id.clone());
                    }
                }
                _ => {}
            }
        }

        ReadyCardsResult {
            ready,
            blocked,
            in_progress,
            completed,
            failed,
        }
    }

    pub fn get_card(&self, card_id: &str) -> Option<&BoardCard> {
        self.cards.iter().find(|c| c.id == card_id)
    }

    pub fn get_card_mut(&mut self, card_id: &str) -> Option<&mut BoardCard> {
        self.cards.iter_mut().find(|c| c.id == card_id)
    }

    pub fn get_dependency_reports(&self, card_id: &str) -> Vec<(String, String)> {
        let card = match self.get_card(card_id) {
            Some(c) => c,
            None => return vec![],
        };

        card.depends_on
            .iter()
            .filter_map(|dep_id| {
                self.get_card(dep_id).and_then(|dep_card| {
                    dep_card
                        .final_report
                        .as_ref()
                        .map(|report| (dep_card.title.clone(), report.clone()))
                })
            })
            .collect()
    }
}
