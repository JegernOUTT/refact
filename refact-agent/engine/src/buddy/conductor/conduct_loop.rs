use std::sync::Arc;

use chrono::Utc;
use refact_buddy_core::conductor::{
    ConductorGoal, ConductorMemo, ConductorWakeReason, GoalBudgetSpent, GoalLedger, GoalStatus,
    LearningOutcome, MemoKind,
};
use refact_buddy_core::conductor_store::{load_goal_ledger, mutate_goal_ledger, MissingGoalBehavior};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::buddy::actor::make_runtime_event;
use crate::buddy::events::BuddyEvent;
use crate::buddy::types::BuddyThreadMeta;
use crate::call_validation::ChatMessage;
use crate::global_context::GlobalContext;
use crate::tasks::types::TaskBoard;

use super::budget::{aggregate_goal_spent, hydrate_goal_spent};
use super::learn::{load_prior_lessons, record_goal_learning};
use super::packet::{
    build_conductor_packet, ConductorAgentSnapshot, ConductorPacketInput, ConductorTaskSnapshot,
};
use super::wake::complete_conductor_wake;

const CONDUCTOR_WORKFLOW_ID: &str = "buddy_conductor";
const MAX_CONDUCTOR_TURN_STEPS: usize = 24;
const MAX_TRANSIENT_TURN_FAILURES: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeAction {
    RunTurn,
    SkipHumanWins,
    Escalate,
    MarkDone,
    Wait,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConductWakeOutcome {
    RanTurn { chat_id: Option<String> },
    SkippedHumanWins,
    Escalated { reason: String },
    MarkedDone,
    Waiting,
    Backoff { failures: u32 },
}

#[derive(Debug, Clone)]
struct ConductorSubchatRequest {
    goal_id: String,
    prompt: String,
    evidence: String,
}

#[derive(Debug, Clone)]
struct ConductorSubchatOutcome {
    chat_id: Option<String>,
}

pub fn decide_wake_action(
    goal: &ConductorGoal,
    spent: &GoalBudgetSpent,
    reasons: &[ConductorWakeReason],
    human_steered: bool,
    board: Option<&TaskBoard>,
) -> WakeAction {
    if goal.status == GoalStatus::Done {
        return WakeAction::MarkDone;
    }
    if budget_overrun(goal, spent) || no_progress_overrun(goal, spent) {
        return WakeAction::Escalate;
    }
    if human_steered {
        return WakeAction::SkipHumanWins;
    }
    if matches!(goal.status, GoalStatus::Escalated | GoalStatus::Abandoned) {
        return WakeAction::Wait;
    }
    if reasons.is_empty() && !board_has_work(board) {
        return WakeAction::Wait;
    }
    WakeAction::RunTurn
}

pub async fn conductor_wake(
    gcx: Arc<GlobalContext>,
    goal_id: &str,
    reasons: Vec<ConductorWakeReason>,
) -> Result<ConductWakeOutcome, String> {
    let project_root = conductor_project_root(gcx.clone()).await?;
    let ledger = load_goal_ledger(&project_root, goal_id)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let goal = ConductorGoal::from_ledger(goal_id.to_string(), ledger);
    conductor_wake_for_goal(gcx, goal, reasons).await
}

pub async fn conductor_wake_for_goal(
    gcx: Arc<GlobalContext>,
    mut goal: ConductorGoal,
    reasons: Vec<ConductorWakeReason>,
) -> Result<ConductWakeOutcome, String> {
    let project_root = conductor_project_root(gcx.clone()).await?;
    let now = Utc::now().to_rfc3339();
    sync_goal_spent_from_ledger(&mut goal);
    let spent = aggregate_goal_spent(gcx.clone(), &goal)
        .await
        .unwrap_or_else(|_| goal.spent.clone());
    let spent = GoalBudgetSpent {
        no_progress_wakes: goal.ledger.no_progress_wakes,
        ..spent
    };
    goal.spent = spent.clone();
    let task_snapshots = collect_task_snapshots(gcx.clone(), &goal).await;
    let board_for_decision = task_snapshots.first().map(|snapshot| &snapshot.board);
    let human_steered = reasons
        .iter()
        .any(|reason| *reason == ConductorWakeReason::HumanSteering);
    let action = decide_wake_action(&goal, &spent, &reasons, human_steered, board_for_decision);

    match action {
        WakeAction::Escalate => {
            let reason = escalation_reason(&goal, &spent);
            goal.ledger.last_wake_at = Some(now.clone());
            goal.ledger.last_wake_reason = reasons.last().copied();
            goal.ledger.turn_failures = 0;
            push_memo(&mut goal.ledger, MemoKind::Escalation, &reason, None);
            apply_goal_status(&mut goal, GoalStatus::Escalated);
            record_goal_learning(
                gcx.clone(),
                &project_root,
                &mut goal,
                LearningOutcome::Escalated,
                Some(&reason),
                None,
            )
            .await?;
            persist_goal_ledger_serialized(&project_root, &goal.id, &goal.ledger)
                .await
                .map_err(|error| error.to_string())?;
            emit_goal_updated(gcx, &goal).await;
            return Ok(ConductWakeOutcome::Escalated { reason });
        }
        WakeAction::SkipHumanWins => {
            goal.ledger.last_wake_at = Some(now.clone());
            goal.ledger.last_wake_reason = reasons.last().copied();
            push_memo(
                &mut goal.ledger,
                MemoKind::HumanSteering,
                "Skipped conductor turn because a human steered the goal.",
                None,
            );
            persist_goal_ledger_serialized(&project_root, &goal.id, &goal.ledger)
                .await
                .map_err(|error| error.to_string())?;
            emit_goal_updated(gcx, &goal).await;
            return Ok(ConductWakeOutcome::SkippedHumanWins);
        }
        WakeAction::MarkDone => {
            goal.ledger.last_wake_at = Some(now.clone());
            goal.ledger.last_wake_reason = reasons.last().copied();
            let source_chat_id = goal.ledger.chat_ids.last().cloned();
            apply_goal_status(&mut goal, GoalStatus::Done);
            record_goal_learning(
                gcx.clone(),
                &project_root,
                &mut goal,
                LearningOutcome::Done,
                None,
                source_chat_id,
            )
            .await?;
            persist_goal_ledger_serialized(&project_root, &goal.id, &goal.ledger)
                .await
                .map_err(|error| error.to_string())?;
            emit_goal_updated(gcx, &goal).await;
            return Ok(ConductWakeOutcome::MarkedDone);
        }
        WakeAction::Wait => return Ok(ConductWakeOutcome::Waiting),
        WakeAction::RunTurn => {}
    }

    let agent_statuses = collect_agent_snapshots(gcx.clone(), &task_snapshots).await;
    let prior_lessons = load_prior_lessons(&project_root, &goal)
        .await
        .unwrap_or_default();
    let built = build_conductor_packet(ConductorPacketInput {
        goal: goal.clone(),
        wake_reasons: reasons.clone(),
        task_boards: task_snapshots,
        agent_statuses,
        last_wake_at: goal.ledger.last_wake_at.clone(),
        prior_lessons,
    });
    let request = ConductorSubchatRequest {
        goal_id: goal.id.clone(),
        prompt: conductor_prompt(&goal),
        evidence: format!("{}\n\n{}", built.text, built.json),
    };

    match run_conductor_turn(gcx.clone(), request).await {
        Ok(outcome) => {
            goal.ledger.last_wake_at = Some(now.clone());
            goal.ledger.last_wake_reason = reasons.last().copied();
            goal.ledger.turn_failures = 0;
            update_no_progress_counter(&mut goal.ledger, &now);
            if let Some(chat_id) = outcome.chat_id.as_ref() {
                push_unique(&mut goal.ledger.chat_ids, chat_id.clone());
            }
            push_memo(
                &mut goal.ledger,
                MemoKind::Progress,
                "Conductor ran one bounded decision turn.",
                outcome.chat_id.clone(),
            );
            persist_goal_ledger_serialized(&project_root, &goal.id, &goal.ledger)
                .await
                .map_err(|error| error.to_string())?;
            goal.spent.no_progress_wakes = goal.ledger.no_progress_wakes;
            emit_goal_updated(gcx, &goal).await;
            Ok(ConductWakeOutcome::RanTurn {
                chat_id: outcome.chat_id,
            })
        }
        Err(error) => {
            goal.ledger.turn_failures = goal.ledger.turn_failures.saturating_add(1);
            goal.ledger.last_wake_at = Some(now.clone());
            goal.ledger.last_wake_reason = reasons.last().copied();
            if goal.ledger.turn_failures >= MAX_TRANSIENT_TURN_FAILURES {
                let reason = format!(
                    "Conductor turn failed {} times: {}",
                    goal.ledger.turn_failures, error
                );
                push_memo(&mut goal.ledger, MemoKind::Escalation, &reason, None);
                apply_goal_status(&mut goal, GoalStatus::Escalated);
                record_goal_learning(
                    gcx.clone(),
                    &project_root,
                    &mut goal,
                    LearningOutcome::Escalated,
                    Some(&reason),
                    None,
                )
                .await?;
                persist_goal_ledger_serialized(&project_root, &goal.id, &goal.ledger)
                    .await
                    .map_err(|error| error.to_string())?;
                emit_goal_updated(gcx, &goal).await;
                Ok(ConductWakeOutcome::Escalated { reason })
            } else {
                push_memo(
                    &mut goal.ledger,
                    MemoKind::Risk,
                    &format!("Transient conductor turn failure: {error}"),
                    None,
                );
                persist_goal_ledger_serialized(&project_root, &goal.id, &goal.ledger)
                    .await
                    .map_err(|error| error.to_string())?;
                requeue_backoff(gcx, &goal.id).await;
                Ok(ConductWakeOutcome::Backoff {
                    failures: goal.ledger.turn_failures,
                })
            }
        }
    }
}

pub async fn run_due_conductor_wakes(gcx: Arc<GlobalContext>) {
    let due = super::wake::try_drain_due_conductor_wakes(&gcx, Utc::now());
    for wake in due {
        let gcx_for_turn = gcx.clone();
        tokio::spawn(async move {
            let goal_id = wake.goal_id.clone();
            let result = conductor_wake(gcx_for_turn.clone(), &goal_id, wake.reasons).await;
            if let Err(error) = result {
                tracing::warn!("conductor wake {} failed: {}", goal_id, error);
            }
            complete_conductor_wake(&gcx_for_turn, &goal_id).await;
        });
    }
}

fn budget_overrun(goal: &ConductorGoal, spent: &GoalBudgetSpent) -> bool {
    goal.budget
        .wall_clock_secs
        .map(|limit| spent.elapsed_secs >= limit)
        .unwrap_or(false)
        || goal
            .budget
            .total_tokens
            .map(|limit| spent.total_tokens >= limit)
            .unwrap_or(false)
        || match (goal.budget.usd, spent.usd) {
            (Some(limit), Some(spent)) => spent >= limit,
            _ => false,
        }
}

fn no_progress_overrun(goal: &ConductorGoal, spent: &GoalBudgetSpent) -> bool {
    goal.budget
        .no_progress_wakes
        .map(|limit| spent.no_progress_wakes >= limit)
        .unwrap_or(false)
}

fn board_has_work(board: Option<&TaskBoard>) -> bool {
    let Some(board) = board else {
        return false;
    };
    let ready = board.get_ready_cards();
    !ready.ready.is_empty()
        || !ready.in_progress.is_empty()
        || !ready.blocked.is_empty()
        || !ready.failed.is_empty()
}

fn escalation_reason(goal: &ConductorGoal, spent: &GoalBudgetSpent) -> String {
    if budget_overrun(goal, spent) {
        return "Conductor budget exhausted before turn.".to_string();
    }
    if no_progress_overrun(goal, spent) {
        return "Conductor no-progress wake budget exhausted before turn.".to_string();
    }
    "Conductor escalated before turn.".to_string()
}

fn sync_goal_spent_from_ledger(goal: &mut ConductorGoal) {
    goal.spent.no_progress_wakes = goal.ledger.no_progress_wakes;
}

fn update_no_progress_counter(ledger: &mut GoalLedger, now: &str) {
    let progress_after_last_wake = match (&ledger.last_progress_at, &ledger.last_wake_at) {
        (Some(progress), Some(last_wake)) => progress > last_wake,
        (Some(_), None) => true,
        _ => false,
    };
    if progress_after_last_wake {
        ledger.no_progress_wakes = 0;
    } else {
        ledger.no_progress_wakes = ledger.no_progress_wakes.saturating_add(1);
    }
    ledger.last_wake_at = Some(now.to_string());
}

fn apply_goal_status(goal: &mut ConductorGoal, status: GoalStatus) {
    goal.status = status;
    goal.ledger.status = Some(status);
    if status.is_terminal() {
        let completed_at = goal
            .completed_at
            .clone()
            .or_else(|| goal.ledger.completed_at.clone())
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        goal.completed_at = Some(completed_at.clone());
        goal.ledger.completed_at = Some(completed_at);
    } else {
        goal.completed_at = None;
        goal.ledger.completed_at = None;
    }
}

async fn conductor_project_root(gcx: Arc<GlobalContext>) -> Result<std::path::PathBuf, String> {
    crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "No workspace folder found".to_string())
}

async fn persist_goal_ledger_serialized(
    project_root: &std::path::Path,
    goal_id: &str,
    ledger: &GoalLedger,
) -> Result<(), String> {
    let replacement = ledger.clone();
    mutate_goal_ledger(
        project_root,
        goal_id,
        MissingGoalBehavior::CreateDefault,
        |ledger| {
            *ledger = replacement;
            Ok(())
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn collect_task_snapshots(
    gcx: Arc<GlobalContext>,
    goal: &ConductorGoal,
) -> Vec<ConductorTaskSnapshot> {
    let mut snapshots = Vec::new();
    for task_id in owned_task_ids(&goal.ledger) {
        let Ok(meta) = crate::tasks::storage::load_task_meta(gcx.clone(), &task_id).await else {
            continue;
        };
        let Ok(board) = crate::tasks::storage::load_board(gcx.clone(), &task_id).await else {
            continue;
        };
        snapshots.push(ConductorTaskSnapshot { meta, board });
    }
    snapshots.sort_by(|left, right| left.meta.id.cmp(&right.meta.id));
    snapshots
}

async fn collect_agent_snapshots(
    gcx: Arc<GlobalContext>,
    tasks: &[ConductorTaskSnapshot],
) -> Vec<ConductorAgentSnapshot> {
    let mut snapshots = Vec::new();
    for task in tasks {
        for card in &task.board.cards {
            let Some(agent_chat_id) = card.agent_chat_id.clone() else {
                continue;
            };
            let session_state = live_session_state(gcx.clone(), &agent_chat_id).await;
            snapshots.push(ConductorAgentSnapshot {
                task_id: task.meta.id.clone(),
                card_id: card.id.clone(),
                card_title: card.title.clone(),
                agent_chat_id,
                column: card.column.clone(),
                priority: card.priority.clone(),
                session_state,
                last_activity_at: card.last_heartbeat_at.clone(),
                last_status_update: card
                    .status_updates
                    .last()
                    .map(|update| format!("{}: {}", update.timestamp, update.message)),
                final_report: card
                    .final_report_structured
                    .as_ref()
                    .map(|report| report.summary.clone())
                    .or_else(|| card.final_report.clone()),
                last_tool_name: None,
            });
        }
    }
    snapshots
}

async fn live_session_state(gcx: Arc<GlobalContext>, chat_id: &str) -> Option<String> {
    let session = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    }?;
    let state = {
        let session = session.lock().await;
        session.runtime.state.to_string()
    };
    Some(state)
}

fn owned_task_ids(ledger: &GoalLedger) -> Vec<String> {
    let mut task_ids = Vec::new();
    if let Some(task_id) = ledger.planner_task_id.as_ref() {
        push_unique(&mut task_ids, task_id.clone());
    }
    for task_id in &ledger.task_ids {
        push_unique(&mut task_ids, task_id.clone());
    }
    task_ids
}

fn conductor_prompt(goal: &ConductorGoal) -> String {
    format!(
        "Coordinate conductor goal {}: {}\nDone when: {}",
        goal.id, goal.title, goal.done_when.summary
    )
}

async fn run_conductor_turn(
    gcx: Arc<GlobalContext>,
    request: ConductorSubchatRequest,
) -> Result<ConductorSubchatOutcome, String> {
    if let Some(outcome) = run_test_conductor_turn(&request) {
        return outcome;
    }
    let subagent_config = crate::yaml_configs::customization_registry::get_subagent_config(
        gcx.clone(),
        CONDUCTOR_WORKFLOW_ID,
        None,
    )
    .await
    .ok_or_else(|| format!("subagent config '{CONDUCTOR_WORKFLOW_ID}' not found"))?;
    let system_prompt = subagent_config
        .messages
        .system_prompt
        .ok_or_else(|| "buddy_conductor system prompt missing".to_string())?;
    let user_template = subagent_config
        .messages
        .user_template
        .ok_or_else(|| "buddy_conductor user template missing".to_string())?;
    let messages = vec![
        ChatMessage::new("system".to_string(), system_prompt),
        ChatMessage::new(
            "user".to_string(),
            render_conductor_template(&user_template, &request),
        ),
    ];
    let max_steps = subagent_config
        .subchat
        .max_steps
        .unwrap_or(MAX_CONDUCTOR_TURN_STEPS)
        .clamp(1, MAX_CONDUCTOR_TURN_STEPS);
    let mut config = crate::subchat::resolve_subchat_config(
        gcx.clone(),
        CONDUCTOR_WORKFLOW_ID,
        true,
        Some(format!(
            "buddy-conductor-{}-{}",
            request.goal_id,
            Uuid::new_v4()
        )),
        Some(format!("Conductor: {}", request.goal_id)),
        None,
        None,
        None,
        Some(subagent_config.tools),
        max_steps,
        false,
        None,
        "buddy".to_string(),
    )
    .await?;
    config.autonomous_no_confirm = true;
    config.mode = "buddy".to_string();
    config.buddy_meta = Some(BuddyThreadMeta {
        is_buddy_chat: true,
        buddy_chat_kind: "conductor".to_string(),
        workflow_id: Some(CONDUCTOR_WORKFLOW_ID.to_string()),
        goal_id: Some(request.goal_id.clone()),
    });
    let result = crate::subchat::run_subchat(gcx, messages, config).await?;
    Ok(ConductorSubchatOutcome {
        chat_id: result.chat_id,
    })
}

fn render_conductor_template(template: &str, request: &ConductorSubchatRequest) -> String {
    template
        .replace("{{workflow_id}}", CONDUCTOR_WORKFLOW_ID)
        .replace("{{title}}", "Buddy Conductor")
        .replace("{{signal_hash}}", &request.goal_id)
        .replace("{{priority}}", "high")
        .replace("{{project_root}}", "")
        .replace("{{prompt}}", &request.prompt)
        .replace("{{evidence}}", &request.evidence)
}

async fn requeue_backoff(gcx: Arc<GlobalContext>, goal_id: &str) {
    let mut bus = gcx.conductor_wake_bus.lock().await;
    bus.enqueue_goal(goal_id, ConductorWakeReason::Heartbeat, Utc::now());
}

async fn emit_goal_updated(gcx: Arc<GlobalContext>, goal: &ConductorGoal) {
    if goal.status.is_terminal() {
        super::wake::refresh_conductor_wake_targets(gcx.clone()).await;
    }
    let goal = hydrate_goal_spent(gcx.clone(), goal.clone()).await;
    if let Some(tx) = gcx.buddy_events_tx.as_ref() {
        let _ = tx.send(BuddyEvent::ConductorGoalUpdated { goal: goal.clone() });
    }
    let event = make_runtime_event(
        "conductor_goal_updated",
        &format!("Conductor goal updated: {}", goal.title),
        "conductor",
        &format!("conductor_goal:{}", goal.id),
        match goal.status {
            GoalStatus::Paused => "paused",
            GoalStatus::Done => "completed",
            GoalStatus::Escalated => "escalated",
            GoalStatus::Abandoned => "failed",
            _ => "running",
        },
        Some("normal"),
    );
    crate::buddy::actor::buddy_enqueue_event(AppState::from_gcx(gcx).await, event).await;
}

fn push_memo(
    ledger: &mut GoalLedger,
    kind: MemoKind,
    content: &str,
    source_chat_id: Option<String>,
) {
    ledger.memos.push(ConductorMemo {
        id: Uuid::new_v4().to_string(),
        kind,
        content: content.to_string(),
        created_at: Utc::now().to_rfc3339(),
        source_chat_id,
        related_task_id: ledger.planner_task_id.clone(),
    });
}

fn push_unique(values: &mut Vec<String>, value: String) {
    let value = value.trim().to_string();
    if !value.is_empty() && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

#[cfg(test)]
type TestRunner = fn(ConductorSubchatRequest) -> Result<ConductorSubchatOutcome, String>;

#[cfg(test)]
fn test_runner_cell() -> &'static std::sync::Mutex<Option<TestRunner>> {
    static CELL: std::sync::OnceLock<std::sync::Mutex<Option<TestRunner>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(Default::default)
}

#[cfg(test)]
struct TestRunnerGuard;

#[cfg(test)]
impl Drop for TestRunnerGuard {
    fn drop(&mut self) {
        *test_runner_cell().lock().unwrap() = None;
    }
}

#[cfg(test)]
fn install_test_runner(runner: TestRunner) -> TestRunnerGuard {
    *test_runner_cell().lock().unwrap() = Some(runner);
    TestRunnerGuard
}

#[cfg(test)]
fn run_test_conductor_turn(
    request: &ConductorSubchatRequest,
) -> Option<Result<ConductorSubchatOutcome, String>> {
    test_runner_cell()
        .lock()
        .unwrap()
        .map(|runner| runner(request.clone()))
}

#[cfg(not(test))]
fn run_test_conductor_turn(
    _request: &ConductorSubchatRequest,
) -> Option<Result<ConductorSubchatOutcome, String>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use refact_buddy_core::conductor::{DoneWhen, GoalBudget};
    use refact_buddy_core::conductor_store::{load_goal_ledger, save_goal_ledger};
    use tokio::time::timeout;

    async fn test_serial() -> tokio::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
        SERIAL.get_or_init(Default::default).lock().await
    }

    async fn test_gcx(root: &std::path::Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    fn goal(goal_id: &str) -> ConductorGoal {
        ConductorGoal {
            id: goal_id.to_string(),
            title: "Ship conductor".to_string(),
            done_when: DoneWhen {
                summary: "Conductor decides the next safe action".to_string(),
                checklist: vec!["turn ran".to_string()],
            },
            budget: GoalBudget {
                wall_clock_secs: Some(3600),
                no_progress_wakes: Some(3),
                total_tokens: Some(100_000),
                usd: Some(10.0),
            },
            created_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        }
    }

    fn spent() -> GoalBudgetSpent {
        GoalBudgetSpent {
            elapsed_secs: 10,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cache_read_tokens: 0,
            usd: Some(0.0),
            no_progress_wakes: 0,
        }
    }

    fn success_runner(request: ConductorSubchatRequest) -> Result<ConductorSubchatOutcome, String> {
        assert_eq!(request.goal_id, "goal-run");
        assert!(request.prompt.contains("Ship conductor"));
        assert!(request.evidence.contains("conductor_decision"));
        Ok(ConductorSubchatOutcome {
            chat_id: Some("conductor-chat-1".to_string()),
        })
    }

    fn any_success_runner(
        request: ConductorSubchatRequest,
    ) -> Result<ConductorSubchatOutcome, String> {
        Ok(ConductorSubchatOutcome {
            chat_id: Some(format!("chat-{}", request.goal_id)),
        })
    }

    fn failing_runner(_: ConductorSubchatRequest) -> Result<ConductorSubchatOutcome, String> {
        Err("temporary conductor failure".to_string())
    }

    #[test]
    fn budget_overrun_escalates_before_turn() {
        let mut goal = goal("goal-budget");
        goal.budget.wall_clock_secs = Some(10);
        let mut spent = spent();
        spent.elapsed_secs = 10;

        let action = decide_wake_action(&goal, &spent, &[ConductorWakeReason::Manual], false, None);

        assert_eq!(action, WakeAction::Escalate);
    }

    #[test]
    fn no_progress_overrun_escalates_before_turn() {
        let goal = goal("goal-no-progress");
        let mut spent = spent();
        spent.no_progress_wakes = 3;

        let action = decide_wake_action(
            &goal,
            &spent,
            &[ConductorWakeReason::Heartbeat],
            false,
            None,
        );

        assert_eq!(action, WakeAction::Escalate);
    }

    #[test]
    fn human_yield_skips_after_guardrails() {
        let goal = goal("goal-human");

        let action = decide_wake_action(
            &goal,
            &spent(),
            &[ConductorWakeReason::HumanSteering],
            true,
            None,
        );

        assert_eq!(action, WakeAction::SkipHumanWins);
    }

    #[tokio::test]
    async fn normal_wake_runs_bounded_turn_and_persists_ledger() {
        let _serial = test_serial().await;
        let _runner = install_test_runner(success_runner);
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        let goal = goal("goal-run");

        let outcome = conductor_wake_for_goal(
            gcx,
            goal,
            vec![ConductorWakeReason::Manual, ConductorWakeReason::TaskBoard],
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            ConductWakeOutcome::RanTurn {
                chat_id: Some("conductor-chat-1".to_string())
            }
        );
        let ledger = load_goal_ledger(dir.path(), "goal-run")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.turn_failures, 0);
        assert_eq!(ledger.no_progress_wakes, 1);
        assert_eq!(ledger.chat_ids, vec!["conductor-chat-1".to_string()]);
        assert!(ledger.last_wake_at.is_some());
        assert_eq!(
            ledger.last_wake_reason,
            Some(ConductorWakeReason::TaskBoard)
        );
        assert!(ledger
            .memos
            .iter()
            .any(|memo| memo.kind == MemoKind::Progress
                && memo.source_chat_id.as_deref() == Some("conductor-chat-1")));
    }

    #[tokio::test]
    async fn turn_failure_backs_off_and_persists_failure() {
        let _serial = test_serial().await;
        let _runner = install_test_runner(failing_runner);
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        let goal = goal("goal-backoff");

        let outcome = conductor_wake_for_goal(gcx.clone(), goal, vec![ConductorWakeReason::Manual])
            .await
            .unwrap();

        assert_eq!(outcome, ConductWakeOutcome::Backoff { failures: 1 });
        let ledger = load_goal_ledger(dir.path(), "goal-backoff")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.turn_failures, 1);
        assert!(ledger.memos.iter().any(|memo| memo.kind == MemoKind::Risk));
        let bus = gcx.conductor_wake_bus.lock().await;
        assert!(bus.mailbox("goal-backoff").is_some());
    }

    #[tokio::test]
    async fn repeated_turn_failure_escalates() {
        let _serial = test_serial().await;
        let _runner = install_test_runner(failing_runner);
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        let mut goal = goal("goal-repeated-failure");
        goal.ledger.turn_failures = 1;

        let outcome = conductor_wake_for_goal(gcx, goal, vec![ConductorWakeReason::Manual])
            .await
            .unwrap();

        match outcome {
            ConductWakeOutcome::Escalated { reason } => {
                assert!(reason.contains("failed 2 times"));
            }
            other => panic!("expected escalation, got {other:?}"),
        }
        let ledger = load_goal_ledger(dir.path(), "goal-repeated-failure")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.turn_failures, 2);
        assert_eq!(ledger.status, Some(GoalStatus::Escalated));
        assert!(ledger.completed_at.is_some());
        assert!(ledger
            .memos
            .iter()
            .any(|memo| memo.kind == MemoKind::Escalation));
    }

    #[tokio::test]
    async fn goal_update_event_is_emitted_after_turn() {
        let _serial = test_serial().await;
        let _runner = install_test_runner(any_success_runner);
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        let mut rx = gcx.buddy_events_tx.as_ref().unwrap().subscribe();

        conductor_wake_for_goal(gcx, goal("goal-event"), vec![ConductorWakeReason::Manual])
            .await
            .unwrap();

        let event = timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            BuddyEvent::ConductorGoalUpdated { goal } => {
                assert_eq!(goal.id, "goal-event");
                assert_eq!(goal.spent.no_progress_wakes, 1);
            }
            other => panic!("expected ConductorGoalUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn persisted_last_wake_memos_and_spent_survive_goal_id_wrapper() {
        let _serial = test_serial().await;
        let _runner = install_test_runner(any_success_runner);
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-wrapper",
            &GoalLedger {
                no_progress_wakes: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let outcome = conductor_wake(gcx, "goal-wrapper", vec![ConductorWakeReason::Heartbeat])
            .await
            .unwrap();

        assert!(matches!(outcome, ConductWakeOutcome::RanTurn { .. }));
        let ledger = load_goal_ledger(dir.path(), "goal-wrapper")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.no_progress_wakes, 2);
        assert!(ledger.last_wake_at.is_some());
        assert_eq!(
            ledger.last_wake_reason,
            Some(ConductorWakeReason::Heartbeat)
        );
        assert_eq!(ledger.memos.len(), 1);
    }
}
