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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScopeGuardMode {
    #[default]
    Off,
    Warn,
    Reject,
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
        BoardColumn {
            id: "regressed".into(),
            title: "Regressed".into(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FinalReport {
    pub summary: String,
    pub success: bool,
    pub files_changed: Vec<String>,
    pub tests_added_or_updated: Vec<String>,
    pub verification: Vec<VerificationResult>,
    pub followup_cards: Vec<SuggestedCard>,
    pub risks: Vec<String>,
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VerificationResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub passed: bool,
    pub output_tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VerifierReport {
    pub passed: bool,
    pub command_results: Vec<VerificationResult>,
    pub concerns: Vec<String>,
    pub recommendation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SuggestedCard {
    pub title: String,
    pub instructions: String,
    pub priority: String,
    pub target_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbVariantInfo {
    pub agent_id: String,
    pub chat_id: String,
    pub worktree: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbVariants {
    pub a: AbVariantInfo,
    pub b: AbVariantInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamMember {
    pub role: String,
    pub agent_chat_id: Option<String>,
    pub agent_branch: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardComment {
    pub id: String,
    pub author_role: String,
    pub author_id: Option<String>,
    pub timestamp: String,
    pub body: String,
    pub reply_to: Option<String>,
}

impl FinalReport {
    pub fn to_markdown(&self) -> String {
        let mut output = String::new();

        output.push_str("# Final Report\n\n");
        output.push_str("## Summary\n");
        push_text_or_empty(&mut output, &self.summary);
        output.push('\n');

        output.push_str("## Result\n");
        output.push_str(if self.success {
            "- Success: true\n\n"
        } else {
            "- Success: false\n\n"
        });

        push_string_list(&mut output, "Files Changed", &self.files_changed);
        push_string_list(
            &mut output,
            "Tests Added or Updated",
            &self.tests_added_or_updated,
        );
        push_verification(&mut output, &self.verification);
        push_followup_cards(&mut output, &self.followup_cards);
        push_string_list(&mut output, "Risks", &self.risks);
        push_string_list(&mut output, "Assumptions", &self.assumptions);

        while output.ends_with('\n') {
            output.pop();
        }
        output.push('\n');
        output
    }
}

fn push_text_or_empty(output: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        output.push_str("_None provided._\n");
    } else {
        output.push_str(text);
        output.push('\n');
    }
}

fn push_string_list(output: &mut String, title: &str, items: &[String]) {
    output.push_str("## ");
    output.push_str(title);
    output.push('\n');
    if items.is_empty() {
        output.push_str("- _None._\n\n");
        return;
    }
    for item in items {
        output.push_str("- ");
        output.push_str(item);
        output.push('\n');
    }
    output.push('\n');
}

fn push_verification(output: &mut String, verification: &[VerificationResult]) {
    output.push_str("## Verification\n");
    if verification.is_empty() {
        output.push_str("- _None._\n\n");
        return;
    }
    for result in verification {
        output.push_str("- ");
        output.push_str(&markdown_inline_code(&result.command));
        output.push_str(" — ");
        output.push_str(if result.passed { "passed" } else { "failed" });
        if let Some(exit_code) = result.exit_code {
            output.push_str(&format!(" (exit code {})", exit_code));
        }
        output.push('\n');
        let tail = result.output_tail.trim();
        if !tail.is_empty() {
            let fence = markdown_code_fence(tail);
            output.push('\n');
            output.push_str(&fence);
            output.push_str("text\n");
            output.push_str(tail);
            output.push('\n');
            output.push_str(&fence);
            output.push('\n');
        }
    }
    output.push('\n');
}

fn markdown_inline_code(text: &str) -> String {
    let max_run = max_backtick_run(text);
    if max_run == 0 {
        return format!("`{}`", text);
    }
    let fence = "`".repeat(max_run + 1);
    format!("{} {} {}", fence, text, fence)
}

fn markdown_code_fence(text: &str) -> String {
    "`".repeat(max_backtick_run(text).max(2) + 1)
}

fn max_backtick_run(text: &str) -> usize {
    let mut max_run = 0;
    let mut current = 0;
    for c in text.chars() {
        if c == '`' {
            current += 1;
            max_run = max_run.max(current);
        } else {
            current = 0;
        }
    }
    max_run
}

fn push_followup_cards(output: &mut String, cards: &[SuggestedCard]) {
    output.push_str("## Follow-up Cards\n");
    if cards.is_empty() {
        output.push_str("- _None._\n\n");
        return;
    }
    for card in cards {
        output.push_str("### ");
        output.push_str(if card.title.trim().is_empty() {
            "Untitled"
        } else {
            card.title.trim()
        });
        output.push('\n');
        output.push_str("- Priority: ");
        output.push_str(if card.priority.trim().is_empty() {
            "unspecified"
        } else {
            card.priority.trim()
        });
        output.push('\n');
        if !card.target_files.is_empty() {
            output.push_str("- Target files: ");
            output.push_str(&card.target_files.join(", "));
            output.push('\n');
        }
        if !card.instructions.trim().is_empty() {
            output.push('\n');
            output.push_str(card.instructions.trim());
            output.push('\n');
        }
        output.push('\n');
    }
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<CardComment>,
    #[serde(default)]
    pub final_report: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_report_structured: Option<FinalReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier_report: Option<VerifierReport>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ab_variants: Option<AbVariants>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub team_members: Vec<TeamMember>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_files: Vec<String>,
    #[serde(default)]
    pub scope_guard_mode: ScopeGuardMode,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting_for_card_ids: Vec<String>,
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
                "failed" | "regressed" => failed.push(card.id.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, title: &str, column: &str, depends_on: Vec<&str>) -> BoardCard {
        BoardCard {
            id: id.into(),
            title: title.into(),
            column: column.into(),
            priority: default_priority(),
            depends_on: depends_on.into_iter().map(String::from).collect(),
            instructions: String::new(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: "2026-05-16T00:00:00Z".into(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: ScopeGuardMode::Off,
        }
    }

    #[test]
    fn default_board_has_schema_and_columns() {
        let board = TaskBoard::default();

        assert_eq!(board.schema_version, 1);
        assert_eq!(board.rev, 0);
        assert!(board.cards.is_empty());
        assert_eq!(
            board
                .columns
                .iter()
                .map(|column| (column.id.as_str(), column.title.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("planned", "Planned"),
                ("doing", "Doing"),
                ("done", "Done"),
                ("failed", "Failed"),
                ("regressed", "Regressed"),
            ]
        );
    }

    #[test]
    fn serde_defaults_preserve_schema_values() {
        let meta: TaskMeta = serde_json::from_str(
            r#"{
                "id": "task-1",
                "name": "Task One",
                "status": "active",
                "created_at": "created",
                "updated_at": "updated"
            }"#,
        )
        .unwrap();
        let board: TaskBoard = serde_json::from_str(r#"{"cards": []}"#).unwrap();

        assert_eq!(meta.schema_version, 1);
        assert_eq!(meta.cards_total, 0);
        assert_eq!(meta.cards_done, 0);
        assert_eq!(meta.cards_failed, 0);
        assert_eq!(meta.agents_active, 0);
        assert!(!meta.is_name_generated);
        assert_eq!(board.schema_version, 1);
        assert_eq!(board.rev, 0);
        assert_eq!(board.columns.len(), 5);
    }

    #[test]
    fn deserialize_old_card_json_without_structured_report() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "done",
                "depends_on": [],
                "instructions": "",
                "assignee": null,
                "agent_chat_id": null,
                "status_updates": [],
                "final_report": "legacy markdown report",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": "2026-05-16T01:00:00Z"
            }"#,
        )
        .unwrap();

        assert_eq!(card.final_report.as_deref(), Some("legacy markdown report"));
        assert!(card.final_report_structured.is_none());
        assert!(card.verifier_report.is_none());
        assert!(card.ab_variants.is_none());
        assert!(card.comments.is_empty());
    }

    #[test]
    fn board_card_without_comments_deserializes_with_empty_vec() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "planned",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": null,
                "assignee": null,
                "agent_chat_id": null
            }"#,
        )
        .unwrap();

        assert!(card.comments.is_empty());
    }

    #[test]
    fn ab_variants_field_defaults_to_none() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "planned",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": null,
                "assignee": null,
                "agent_chat_id": null
            }"#,
        )
        .unwrap();

        assert!(card.ab_variants.is_none());
    }

    #[test]
    fn scope_guard_mode_defaults_to_off() {
        assert_eq!(ScopeGuardMode::default(), ScopeGuardMode::Off);
    }

    #[test]
    fn board_card_without_scope_guard_field_deserializes() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "planned",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": null,
                "assignee": null,
                "agent_chat_id": null
            }"#,
        )
        .unwrap();

        assert_eq!(card.scope_guard_mode, ScopeGuardMode::Off);
    }

    #[test]
    fn board_card_without_team_members_deserializes_with_empty_vec() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id": "T-1",
                "title": "Legacy card",
                "column": "planned",
                "created_at": "2026-05-16T00:00:00Z",
                "started_at": null,
                "completed_at": null,
                "assignee": null,
                "agent_chat_id": null
            }"#,
        )
        .unwrap();

        assert!(card.team_members.is_empty());
    }

    #[test]
    fn empty_team_members_are_skipped_in_serialized_yaml_output() {
        let card = card("T-1", "Schema card", "planned", vec![]);

        let output = serde_json::to_string(&card).unwrap();

        assert!(!output.contains("team_members"));
    }

    #[test]
    fn team_members_round_trip_three_entries() {
        let mut card = card("T-1", "Team card", "doing", vec![]);
        card.team_members = vec![
            TeamMember {
                role: "implementer".into(),
                agent_chat_id: Some("chat-implementer".into()),
                agent_branch: Some("team/implementer".into()),
                status: Some("doing".into()),
            },
            TeamMember {
                role: "tester".into(),
                agent_chat_id: Some("chat-tester".into()),
                agent_branch: Some("team/tester".into()),
                status: Some("idle".into()),
            },
            TeamMember {
                role: "reviewer".into(),
                agent_chat_id: None,
                agent_branch: None,
                status: Some("done".into()),
            },
        ];

        let encoded = serde_json::to_string(&card).unwrap();
        let decoded: BoardCard = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.team_members.len(), 3);
        assert_eq!(decoded.team_members[0].role, "implementer");
        assert_eq!(
            decoded.team_members[0].agent_chat_id.as_deref(),
            Some("chat-implementer")
        );
        assert_eq!(
            decoded.team_members[0].agent_branch.as_deref(),
            Some("team/implementer")
        );
        assert_eq!(decoded.team_members[0].status.as_deref(), Some("doing"));
        assert_eq!(decoded.team_members[1].role, "tester");
        assert_eq!(
            decoded.team_members[1].agent_chat_id.as_deref(),
            Some("chat-tester")
        );
        assert_eq!(
            decoded.team_members[1].agent_branch.as_deref(),
            Some("team/tester")
        );
        assert_eq!(decoded.team_members[1].status.as_deref(), Some("idle"));
        assert_eq!(decoded.team_members[2].role, "reviewer");
        assert!(decoded.team_members[2].agent_chat_id.is_none());
        assert!(decoded.team_members[2].agent_branch.is_none());
        assert_eq!(decoded.team_members[2].status.as_deref(), Some("done"));
    }

    #[test]
    fn final_report_round_trips_json() {
        let report = FinalReport {
            summary: "Implemented structured reports".into(),
            success: true,
            files_changed: vec!["refact-agent/engine/src/tools/tool_task_agent_finish.rs".into()],
            tests_added_or_updated: vec!["final_report_round_trips_json".into()],
            verification: vec![VerificationResult {
                command: "cargo test --lib -p refact-tasks".into(),
                exit_code: Some(0),
                passed: true,
                output_tail: "test result: ok".into(),
            }],
            followup_cards: vec![SuggestedCard {
                title: "Render structured reports in GUI".into(),
                instructions: "Prefer final_report_structured when present.".into(),
                priority: "P2".into(),
                target_files: vec!["refact-agent/gui/src/features/Tasks".into()],
            }],
            risks: vec!["Older consumers still read markdown.".into()],
            assumptions: vec!["Legacy report remains populated.".into()],
        };

        let encoded = serde_json::to_string(&report).unwrap();
        let decoded: FinalReport = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, report);
    }

    #[test]
    fn verifier_report_round_trips_json() {
        let report = VerifierReport {
            passed: false,
            command_results: vec![VerificationResult {
                command: "cargo test --lib -p refact-lsp -- verifier".into(),
                exit_code: Some(1),
                passed: false,
                output_tail: "test failed".into(),
            }],
            concerns: vec!["Diff removes a required guard".into()],
            recommendation: "fix-needed".into(),
        };

        let encoded = serde_json::to_string(&report).unwrap();
        let decoded: VerifierReport = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, report);
    }

    #[test]
    fn final_report_to_markdown_renders_all_sections() {
        let report = FinalReport {
            summary: "Summary text".into(),
            success: false,
            files_changed: vec!["src/lib.rs".into()],
            tests_added_or_updated: vec!["report test".into()],
            verification: vec![VerificationResult {
                command: "cargo test".into(),
                exit_code: Some(1),
                passed: false,
                output_tail: "failure tail".into(),
            }],
            followup_cards: vec![SuggestedCard {
                title: "Fix follow-up".into(),
                instructions: "Do the next thing.".into(),
                priority: "P1".into(),
                target_files: vec!["src/followup.rs".into()],
            }],
            risks: vec!["Risk item".into()],
            assumptions: vec!["Assumption item".into()],
        };

        let markdown = report.to_markdown();

        assert!(markdown.contains("## Summary\nSummary text"));
        assert!(markdown.contains("## Result\n- Success: false"));
        assert!(markdown.contains("## Files Changed\n- src/lib.rs"));
        assert!(markdown.contains("## Tests Added or Updated\n- report test"));
        assert!(markdown.contains("## Verification\n- `cargo test` — failed (exit code 1)"));
        assert!(markdown.contains("failure tail"));
        assert!(markdown.contains("## Follow-up Cards\n### Fix follow-up"));
        assert!(markdown.contains("- Priority: P1"));
        assert!(markdown.contains("- Target files: src/followup.rs"));
        assert!(markdown.contains("Do the next thing."));
        assert!(markdown.contains("## Risks\n- Risk item"));
        assert!(markdown.contains("## Assumptions\n- Assumption item"));
    }

    #[test]
    fn final_report_to_markdown_escapes_command_backticks() {
        let report = FinalReport {
            verification: vec![VerificationResult {
                command: "cargo test -- --exact `final_report`".into(),
                exit_code: Some(0),
                passed: true,
                output_tail: String::new(),
            }],
            ..Default::default()
        };

        let markdown = report.to_markdown();

        assert!(markdown.contains("- `` cargo test -- --exact `final_report` `` — passed"));
        assert!(!markdown.contains("- `cargo test -- --exact `final_report`` — passed"));
    }

    #[test]
    fn final_report_to_markdown_uses_safe_output_fence() {
        let report = FinalReport {
            verification: vec![VerificationResult {
                command: "cargo test".into(),
                exit_code: Some(1),
                passed: false,
                output_tail: "before\n```\n## Fake Section\n```\nafter".into(),
            }],
            ..Default::default()
        };

        let markdown = report.to_markdown();

        assert!(markdown.contains("\n````text\nbefore\n```\n## Fake Section\n```\nafter\n````\n"));
        assert!(!markdown.contains("\n```text\nbefore\n```\n## Fake Section"));
    }

    #[test]
    fn ready_cards_separate_ready_blocked_and_terminal_columns() {
        let board = TaskBoard {
            cards: vec![
                card("dep-done", "Dependency done", "done", vec![]),
                card("dep-failed", "Dependency failed", "failed", vec![]),
                card("ready", "Ready", "planned", vec!["dep-done"]),
                card("blocked", "Blocked", "planned", vec!["dep-failed"]),
                card(
                    "blocked-missing",
                    "Blocked missing",
                    "planned",
                    vec!["missing"],
                ),
                card("in-progress", "In progress", "doing", vec![]),
                card("regressed", "Regressed", "regressed", vec![]),
            ],
            ..TaskBoard::default()
        };

        let result = board.get_ready_cards();

        assert_eq!(result.ready, vec!["ready"]);
        assert_eq!(result.blocked, vec!["blocked", "blocked-missing"]);
        assert_eq!(result.in_progress, vec!["in-progress"]);
        assert_eq!(result.completed, vec!["dep-done"]);
        assert_eq!(result.failed, vec!["dep-failed", "regressed"]);
    }

    #[test]
    fn dependency_reports_include_only_dependencies_with_reports() {
        let mut reported = card("dep-reported", "Reported dependency", "done", vec![]);
        reported.final_report = Some("finished cleanly".into());
        let unreported = card("dep-unreported", "Unreported dependency", "done", vec![]);
        let consumer = card(
            "consumer",
            "Consumer",
            "planned",
            vec!["dep-reported", "dep-unreported", "missing"],
        );
        let board = TaskBoard {
            cards: vec![reported, unreported, consumer],
            ..TaskBoard::default()
        };

        assert_eq!(
            board.get_dependency_reports("consumer"),
            vec![("Reported dependency".into(), "finished cleanly".into())]
        );
        assert!(board.get_dependency_reports("missing").is_empty());
    }
}
