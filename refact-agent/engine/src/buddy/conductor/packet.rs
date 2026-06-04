use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::buddy::jobs::autonomous_chats::redact_and_cap_text;
use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta};
use refact_buddy_core::conductor::{
    ConductorGoal, ConductorMemo, ConductorWakeReason, GoalAutonomy, GoalStatus, MemoKind,
    PendingQuestion,
};

use super::learn::{lesson_text, PriorConductorLesson};

pub const MAX_PACKET_JSON_CHARS: usize = 24_000;
const MAX_PACKET_TEXT_CHARS: usize = 12_000;
const MAX_TASKS: usize = 6;
const MAX_CARDS_PER_TASK: usize = 8;
const MAX_AGENT_STATUSES: usize = 16;
const MAX_MEMOS: usize = 8;
const MAX_PRIOR_LESSONS: usize = 5;
const MAX_PENDING_QUESTIONS: usize = 8;
const MAX_DONE_CHECKLIST: usize = 8;
const TEXT_TINY: usize = 64;
const TEXT_SHORT: usize = 120;
const TEXT_MEDIUM: usize = 240;
const TEXT_LONG: usize = 420;

#[derive(Debug, Clone)]
pub struct ConductorPacketInput {
    pub goal: ConductorGoal,
    pub wake_reasons: Vec<ConductorWakeReason>,
    pub task_boards: Vec<ConductorTaskSnapshot>,
    pub agent_statuses: Vec<ConductorAgentSnapshot>,
    pub last_wake_at: Option<String>,
    pub prior_lessons: Vec<PriorConductorLesson>,
}

#[derive(Debug, Clone)]
pub struct ConductorTaskSnapshot {
    pub meta: TaskMeta,
    pub board: TaskBoard,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConductorAgentSnapshot {
    pub task_id: String,
    pub card_id: String,
    pub card_title: String,
    pub agent_chat_id: String,
    pub column: String,
    pub priority: String,
    pub session_state: Option<String>,
    pub last_activity_at: Option<String>,
    pub last_status_update: Option<String>,
    pub final_report: Option<String>,
    pub last_tool_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuiltConductorPacket {
    pub packet: ConductorDecisionPacket,
    pub json: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConductorDecisionPacket {
    pub schema_version: u32,
    pub packet_kind: String,
    pub size_cap_chars: usize,
    pub goal: PacketGoal,
    pub budget: PacketBudget,
    pub wake: PacketWake,
    pub task_boards: Vec<PacketTaskBoard>,
    pub planner: PacketPlannerSummary,
    pub agents: PacketAgentSummary,
    pub memos: PacketMemoDigest,
    pub prior_lessons: PacketPriorLessons,
    pub pending_questions: PacketPendingQuestions,
    pub truncation_markers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketGoal {
    pub id: String,
    pub title: String,
    pub status: GoalStatus,
    pub autonomy: GoalAutonomy,
    pub plan_doc_slug: Option<String>,
    pub done_when: PacketDoneWhen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketDoneWhen {
    pub summary: String,
    pub checklist: Vec<String>,
    pub total_checklist_items: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketBudget {
    pub source: String,
    pub wall_clock_secs_limit: Option<u64>,
    pub elapsed_secs_spent: u64,
    pub wall_clock_secs_remaining: Option<u64>,
    pub no_progress_wakes_limit: Option<u32>,
    pub no_progress_wakes_spent: u32,
    pub no_progress_wakes_remaining: Option<u32>,
    pub total_tokens_limit: Option<u64>,
    pub total_tokens_spent: u64,
    pub total_tokens_remaining: Option<u64>,
    pub cache_read_tokens_spent: u64,
    pub usd_limit: Option<f64>,
    pub usd_spent: Option<f64>,
    pub usd_remaining: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketWake {
    pub reasons: Vec<String>,
    pub last_wake_reason: Option<String>,
    pub last_wake_at: Option<String>,
    pub last_progress_at: Option<String>,
    pub no_progress_counter: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketTaskBoard {
    pub task_id: String,
    pub name: String,
    pub status: String,
    pub planner_session_state: Option<String>,
    pub board_rev: u64,
    pub counts_by_column: BTreeMap<String, usize>,
    pub ready: Vec<String>,
    pub blocked: Vec<String>,
    pub in_progress: Vec<String>,
    pub completed: Vec<String>,
    pub failed: Vec<String>,
    pub cards: Vec<PacketCardSummary>,
    pub total_cards: usize,
    pub cards_shown: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketCardSummary {
    pub id: String,
    pub title: String,
    pub column: String,
    pub priority: String,
    pub assignee: Option<String>,
    pub agent_chat_id: Option<String>,
    pub last_update: Option<String>,
    pub final_report_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketPlannerSummary {
    pub tasks_total: usize,
    pub planner_task_id: Option<String>,
    pub planner_states: Vec<PacketPlannerState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketPlannerState {
    pub task_id: String,
    pub session_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketAgentSummary {
    pub total: usize,
    pub shown: usize,
    pub counts_by_state: BTreeMap<String, usize>,
    pub statuses: Vec<PacketAgentStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketAgentStatus {
    pub task_id: String,
    pub card_id: String,
    pub card_title: String,
    pub agent_chat_id: String,
    pub column: String,
    pub priority: String,
    pub state: String,
    pub last_activity_at: Option<String>,
    pub last_tool_name: Option<String>,
    pub last_update: Option<String>,
    pub final_report_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketMemoDigest {
    pub total: usize,
    pub shown: usize,
    pub counts_by_kind: BTreeMap<String, usize>,
    pub recent: Vec<PacketMemoSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketMemoSummary {
    pub id: String,
    pub kind: String,
    pub created_at: String,
    pub source_chat_id: Option<String>,
    pub related_task_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketPriorLessons {
    pub total: usize,
    pub shown: usize,
    pub lessons: Vec<PacketPriorLesson>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketPriorLesson {
    pub goal_id: String,
    pub outcome: String,
    pub created_at: String,
    pub lesson: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketPendingQuestions {
    pub total_open: usize,
    pub blocking_open: usize,
    pub shown: usize,
    pub questions: Vec<PacketQuestionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketQuestionSummary {
    pub id: String,
    pub asked_at: String,
    pub source_chat_id: Option<String>,
    pub blocking: bool,
    pub question: String,
}

pub fn build_conductor_packet(input: ConductorPacketInput) -> BuiltConductorPacket {
    let mut markers = BTreeSet::new();
    let mut packet = ConductorDecisionPacket {
        schema_version: 1,
        packet_kind: "conductor_decision".to_string(),
        size_cap_chars: MAX_PACKET_JSON_CHARS,
        goal: packet_goal(&input.goal, &mut markers),
        budget: packet_budget(&input.goal),
        wake: packet_wake(&input, &mut markers),
        task_boards: packet_task_boards(&input, &mut markers),
        planner: packet_planner_summary(&input, &mut markers),
        agents: packet_agent_summary(&input, &mut markers),
        memos: packet_memo_digest(&input.goal.ledger.memos, &mut markers),
        prior_lessons: packet_prior_lessons(&input.prior_lessons, &mut markers),
        pending_questions: packet_pending_questions(
            &input.goal.ledger.pending_questions,
            &mut markers,
        ),
        truncation_markers: Vec::new(),
    };
    packet.truncation_markers = markers.into_iter().collect();
    let json = render_json_capped(&mut packet);
    let text = render_text_capped(&packet);
    BuiltConductorPacket { packet, json, text }
}

fn packet_prior_lessons(
    lessons: &[PriorConductorLesson],
    markers: &mut BTreeSet<String>,
) -> PacketPriorLessons {
    let mut lessons = lessons.iter().collect::<Vec<_>>();
    let total = lessons.len();
    lessons.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.goal_id.cmp(&right.goal_id))
    });
    if lessons.len() > MAX_PRIOR_LESSONS {
        markers.insert("prior_lessons".to_string());
        lessons.truncate(MAX_PRIOR_LESSONS);
    }
    let items = lessons
        .into_iter()
        .map(|lesson| PacketPriorLesson {
            goal_id: clean("prior_lessons.goal_id", &lesson.goal_id, TEXT_TINY, markers),
            outcome: enum_name(&lesson.outcome),
            created_at: clean(
                "prior_lessons.created_at",
                &lesson.created_at,
                TEXT_TINY,
                markers,
            ),
            lesson: clean(
                "prior_lessons.lesson",
                &lesson_text(lesson),
                TEXT_LONG,
                markers,
            ),
        })
        .collect::<Vec<_>>();
    PacketPriorLessons {
        total,
        shown: items.len(),
        lessons: items,
    }
}

fn packet_goal(goal: &ConductorGoal, markers: &mut BTreeSet<String>) -> PacketGoal {
    PacketGoal {
        id: clean("goal.id", &goal.id, TEXT_TINY, markers),
        title: clean("goal.title", &goal.title, TEXT_SHORT, markers),
        status: goal.status,
        autonomy: goal.autonomy,
        plan_doc_slug: goal
            .plan_doc_slug
            .as_deref()
            .map(|slug| clean("goal.plan_doc_slug", slug, TEXT_TINY, markers)),
        done_when: PacketDoneWhen {
            summary: clean(
                "goal.done_when.summary",
                &goal.done_when.summary,
                TEXT_MEDIUM,
                markers,
            ),
            checklist: capped_clean_list(
                "goal.done_when.checklist",
                &goal.done_when.checklist,
                MAX_DONE_CHECKLIST,
                TEXT_SHORT,
                markers,
            ),
            total_checklist_items: goal.done_when.checklist.len(),
        },
    }
}

fn packet_budget(goal: &ConductorGoal) -> PacketBudget {
    PacketBudget {
        source: "goal_ledger_placeholder".to_string(),
        wall_clock_secs_limit: goal.budget.wall_clock_secs,
        elapsed_secs_spent: goal.spent.elapsed_secs,
        wall_clock_secs_remaining: remaining_u64(
            goal.budget.wall_clock_secs,
            goal.spent.elapsed_secs,
        ),
        no_progress_wakes_limit: goal.budget.no_progress_wakes,
        no_progress_wakes_spent: goal.spent.no_progress_wakes,
        no_progress_wakes_remaining: remaining_u32(
            goal.budget.no_progress_wakes,
            goal.spent.no_progress_wakes,
        ),
        total_tokens_limit: goal.budget.total_tokens,
        total_tokens_spent: goal.spent.total_tokens,
        total_tokens_remaining: remaining_u64(goal.budget.total_tokens, goal.spent.total_tokens),
        cache_read_tokens_spent: goal.spent.cache_read_tokens,
        usd_limit: goal.budget.usd,
        usd_spent: goal.spent.usd,
        usd_remaining: remaining_f64(goal.budget.usd, goal.spent.usd),
    }
}

fn packet_wake(input: &ConductorPacketInput, markers: &mut BTreeSet<String>) -> PacketWake {
    let mut reasons = input
        .wake_reasons
        .iter()
        .map(wake_reason_name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if reasons.len() > 16 {
        reasons.truncate(16);
        markers.insert("wake.reasons".to_string());
    }
    PacketWake {
        reasons,
        last_wake_reason: input
            .goal
            .ledger
            .last_wake_reason
            .as_ref()
            .map(wake_reason_name),
        last_wake_at: input
            .last_wake_at
            .as_deref()
            .map(|value| clean("wake.last_wake_at", value, TEXT_TINY, markers)),
        last_progress_at: input
            .goal
            .ledger
            .last_progress_at
            .as_deref()
            .map(|value| clean("wake.last_progress_at", value, TEXT_TINY, markers)),
        no_progress_counter: input.goal.spent.no_progress_wakes,
    }
}

fn packet_task_boards(
    input: &ConductorPacketInput,
    markers: &mut BTreeSet<String>,
) -> Vec<PacketTaskBoard> {
    let owned_ids = input
        .goal
        .ledger
        .task_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut tasks = input
        .task_boards
        .iter()
        .filter(|task| owned_ids.is_empty() || owned_ids.contains(task.meta.id.as_str()))
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| left.meta.id.cmp(&right.meta.id));
    if tasks.len() > MAX_TASKS {
        markers.insert("task_boards".to_string());
        tasks.truncate(MAX_TASKS);
    }
    tasks
        .into_iter()
        .map(|task| packet_task_board(task, markers))
        .collect()
}

fn packet_task_board(
    task: &ConductorTaskSnapshot,
    markers: &mut BTreeSet<String>,
) -> PacketTaskBoard {
    let ready = task.board.get_ready_cards();
    let mut cards = task.board.cards.iter().collect::<Vec<_>>();
    cards.sort_by(|left, right| {
        priority_rank(&left.priority)
            .cmp(&priority_rank(&right.priority))
            .then_with(|| column_rank(&left.column).cmp(&column_rank(&right.column)))
            .then_with(|| left.id.cmp(&right.id))
    });
    if cards.len() > MAX_CARDS_PER_TASK {
        markers.insert(format!("task_boards.{}.cards", task.meta.id));
        cards.truncate(MAX_CARDS_PER_TASK);
    }
    let cards = cards
        .into_iter()
        .map(|card| packet_card_summary(card, markers))
        .collect::<Vec<_>>();
    PacketTaskBoard {
        task_id: clean("task.task_id", &task.meta.id, TEXT_TINY, markers),
        name: clean("task.name", &task.meta.name, TEXT_SHORT, markers),
        status: enum_name(&task.meta.status),
        planner_session_state: task
            .meta
            .planner_session_state
            .as_deref()
            .map(|value| clean("task.planner_session_state", value, TEXT_TINY, markers)),
        board_rev: task.board.rev,
        counts_by_column: board_counts_by_column(&task.board),
        ready: capped_clean_list(
            "task.ready",
            &ready.ready,
            MAX_CARDS_PER_TASK,
            TEXT_TINY,
            markers,
        ),
        blocked: capped_clean_list(
            "task.blocked",
            &ready.blocked,
            MAX_CARDS_PER_TASK,
            TEXT_TINY,
            markers,
        ),
        in_progress: capped_clean_list(
            "task.in_progress",
            &ready.in_progress,
            MAX_CARDS_PER_TASK,
            TEXT_TINY,
            markers,
        ),
        completed: capped_clean_list(
            "task.completed",
            &ready.completed,
            MAX_CARDS_PER_TASK,
            TEXT_TINY,
            markers,
        ),
        failed: capped_clean_list(
            "task.failed",
            &ready.failed,
            MAX_CARDS_PER_TASK,
            TEXT_TINY,
            markers,
        ),
        total_cards: task.board.cards.len(),
        cards_shown: cards.len(),
        cards,
    }
}

fn packet_card_summary(card: &BoardCard, markers: &mut BTreeSet<String>) -> PacketCardSummary {
    PacketCardSummary {
        id: clean("card.id", &card.id, TEXT_TINY, markers),
        title: clean("card.title", &card.title, TEXT_SHORT, markers),
        column: clean("card.column", &card.column, TEXT_TINY, markers),
        priority: clean("card.priority", &card.priority, TEXT_TINY, markers),
        assignee: card
            .assignee
            .as_deref()
            .map(|value| clean("card.assignee", value, TEXT_TINY, markers)),
        agent_chat_id: card
            .agent_chat_id
            .as_deref()
            .map(|value| clean("card.agent_chat_id", value, TEXT_TINY, markers)),
        last_update: card
            .status_updates
            .last()
            .map(|update| format!("{}: {}", update.timestamp, update.message))
            .map(|value| clean("card.last_update", &value, TEXT_MEDIUM, markers)),
        final_report_summary: card
            .final_report_structured
            .as_ref()
            .map(|report| report.summary.as_str())
            .or(card.final_report.as_deref())
            .map(|value| clean("card.final_report_summary", value, TEXT_MEDIUM, markers)),
    }
}

fn packet_planner_summary(
    input: &ConductorPacketInput,
    markers: &mut BTreeSet<String>,
) -> PacketPlannerSummary {
    let mut states = input
        .task_boards
        .iter()
        .map(|task| PacketPlannerState {
            task_id: clean("planner.task_id", &task.meta.id, TEXT_TINY, markers),
            session_state: task
                .meta
                .planner_session_state
                .as_deref()
                .map(|value| clean("planner.session_state", value, TEXT_TINY, markers)),
        })
        .collect::<Vec<_>>();
    states.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    if states.len() > MAX_TASKS {
        markers.insert("planner.states".to_string());
        states.truncate(MAX_TASKS);
    }
    PacketPlannerSummary {
        tasks_total: input.task_boards.len(),
        planner_task_id: input
            .goal
            .ledger
            .planner_task_id
            .as_deref()
            .map(|value| clean("planner.planner_task_id", value, TEXT_TINY, markers)),
        planner_states: states,
    }
}

fn packet_agent_summary(
    input: &ConductorPacketInput,
    markers: &mut BTreeSet<String>,
) -> PacketAgentSummary {
    let mut statuses = input.agent_statuses.iter().collect::<Vec<_>>();
    statuses.sort_by(|left, right| {
        left.task_id
            .cmp(&right.task_id)
            .then_with(|| priority_rank(&left.priority).cmp(&priority_rank(&right.priority)))
            .then_with(|| left.card_id.cmp(&right.card_id))
            .then_with(|| left.agent_chat_id.cmp(&right.agent_chat_id))
    });
    let mut counts = BTreeMap::new();
    for status in &statuses {
        *counts.entry(agent_state(status)).or_insert(0) += 1;
    }
    if statuses.len() > MAX_AGENT_STATUSES {
        markers.insert("agents.statuses".to_string());
        statuses.truncate(MAX_AGENT_STATUSES);
    }
    let items = statuses
        .into_iter()
        .map(|status| PacketAgentStatus {
            task_id: clean("agent.task_id", &status.task_id, TEXT_TINY, markers),
            card_id: clean("agent.card_id", &status.card_id, TEXT_TINY, markers),
            card_title: clean("agent.card_title", &status.card_title, TEXT_SHORT, markers),
            agent_chat_id: clean(
                "agent.agent_chat_id",
                &status.agent_chat_id,
                TEXT_TINY,
                markers,
            ),
            column: clean("agent.column", &status.column, TEXT_TINY, markers),
            priority: clean("agent.priority", &status.priority, TEXT_TINY, markers),
            state: agent_state(status),
            last_activity_at: status
                .last_activity_at
                .as_deref()
                .map(|value| clean("agent.last_activity_at", value, TEXT_TINY, markers)),
            last_tool_name: status
                .last_tool_name
                .as_deref()
                .map(|value| clean("agent.last_tool_name", value, TEXT_TINY, markers)),
            last_update: status
                .last_status_update
                .as_deref()
                .map(|value| clean("agent.last_update", value, TEXT_MEDIUM, markers)),
            final_report_summary: status
                .final_report
                .as_deref()
                .map(|value| clean("agent.final_report_summary", value, TEXT_MEDIUM, markers)),
        })
        .collect::<Vec<_>>();
    PacketAgentSummary {
        total: input.agent_statuses.len(),
        shown: items.len(),
        counts_by_state: counts,
        statuses: items,
    }
}

fn packet_memo_digest(memos: &[ConductorMemo], markers: &mut BTreeSet<String>) -> PacketMemoDigest {
    let mut counts = BTreeMap::new();
    for memo in memos {
        *counts.entry(memo_kind_name(memo.kind)).or_insert(0) += 1;
    }
    let mut recent = memos.iter().collect::<Vec<_>>();
    recent.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    if recent.len() > MAX_MEMOS {
        markers.insert("memos.recent".to_string());
        recent.truncate(MAX_MEMOS);
    }
    let recent = recent
        .into_iter()
        .map(|memo| PacketMemoSummary {
            id: clean("memo.id", &memo.id, TEXT_TINY, markers),
            kind: memo_kind_name(memo.kind),
            created_at: clean("memo.created_at", &memo.created_at, TEXT_TINY, markers),
            source_chat_id: memo
                .source_chat_id
                .as_deref()
                .map(|value| clean("memo.source_chat_id", value, TEXT_TINY, markers)),
            related_task_id: memo
                .related_task_id
                .as_deref()
                .map(|value| clean("memo.related_task_id", value, TEXT_TINY, markers)),
            content: clean("memo.content", &memo.content, TEXT_LONG, markers),
        })
        .collect::<Vec<_>>();
    PacketMemoDigest {
        total: memos.len(),
        shown: recent.len(),
        counts_by_kind: counts,
        recent,
    }
}

fn packet_pending_questions(
    questions: &[PendingQuestion],
    markers: &mut BTreeSet<String>,
) -> PacketPendingQuestions {
    let mut open = questions
        .iter()
        .filter(|question| question.answer.is_none())
        .collect::<Vec<_>>();
    open.sort_by(|left, right| {
        right
            .blocking
            .cmp(&left.blocking)
            .then_with(|| left.asked_at.cmp(&right.asked_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    let total_open = open.len();
    let blocking_open = open.iter().filter(|question| question.blocking).count();
    if open.len() > MAX_PENDING_QUESTIONS {
        markers.insert("pending_questions.questions".to_string());
        open.truncate(MAX_PENDING_QUESTIONS);
    }
    let questions = open
        .into_iter()
        .map(|question| PacketQuestionSummary {
            id: clean("question.id", &question.id, TEXT_TINY, markers),
            asked_at: clean("question.asked_at", &question.asked_at, TEXT_TINY, markers),
            source_chat_id: question
                .source_chat_id
                .as_deref()
                .map(|value| clean("question.source_chat_id", value, TEXT_TINY, markers)),
            blocking: question.blocking,
            question: clean(
                "question.question",
                &question.question,
                TEXT_MEDIUM,
                markers,
            ),
        })
        .collect::<Vec<_>>();
    PacketPendingQuestions {
        total_open,
        blocking_open,
        shown: questions.len(),
        questions,
    }
}

fn render_json_capped(packet: &mut ConductorDecisionPacket) -> String {
    let mut json = serde_json::to_string(packet).unwrap_or_else(|_| "{}".to_string());
    if json.len() <= MAX_PACKET_JSON_CHARS {
        return json;
    }
    push_marker(packet, "packet.json");
    while json.len() > MAX_PACKET_JSON_CHARS {
        if trim_largest_task_cards(packet) {
            json = serde_json::to_string(packet).unwrap_or_else(|_| "{}".to_string());
            continue;
        }
        if packet.agents.statuses.pop().is_some() {
            packet.agents.shown = packet.agents.statuses.len();
            json = serde_json::to_string(packet).unwrap_or_else(|_| "{}".to_string());
            continue;
        }
        if packet.memos.recent.pop().is_some() {
            packet.memos.shown = packet.memos.recent.len();
            json = serde_json::to_string(packet).unwrap_or_else(|_| "{}".to_string());
            continue;
        }
        if packet.prior_lessons.lessons.pop().is_some() {
            packet.prior_lessons.shown = packet.prior_lessons.lessons.len();
            json = serde_json::to_string(packet).unwrap_or_else(|_| "{}".to_string());
            continue;
        }
        if packet.pending_questions.questions.pop().is_some() {
            packet.pending_questions.shown = packet.pending_questions.questions.len();
            json = serde_json::to_string(packet).unwrap_or_else(|_| "{}".to_string());
            continue;
        }
        if compact_card_text(packet) {
            json = serde_json::to_string(packet).unwrap_or_else(|_| "{}".to_string());
            continue;
        }
        packet.task_boards.clear();
        packet.agents.statuses.clear();
        packet.agents.shown = 0;
        packet.memos.recent.clear();
        packet.memos.shown = 0;
        packet.prior_lessons.lessons.clear();
        packet.prior_lessons.shown = 0;
        packet.pending_questions.questions.clear();
        packet.pending_questions.shown = 0;
        json = serde_json::to_string(packet).unwrap_or_else(|_| "{}".to_string());
        break;
    }
    json
}

fn render_text_capped(packet: &ConductorDecisionPacket) -> String {
    let mut lines = vec![
        format!("# Conductor decision packet: {}", packet.goal.id),
        format!(
            "Goal: {} status={:?} autonomy={:?} plan_doc={}",
            packet.goal.title,
            packet.goal.status,
            packet.goal.autonomy,
            packet.goal.plan_doc_slug.as_deref().unwrap_or("none")
        ),
        format!("Wake reasons: {}", packet.wake.reasons.join(", ")),
        format!(
            "Budget: elapsed={}s wall_remaining={:?} no_progress={}/{:?}",
            packet.budget.elapsed_secs_spent,
            packet.budget.wall_clock_secs_remaining,
            packet.budget.no_progress_wakes_spent,
            packet.budget.no_progress_wakes_limit
        ),
        format!(
            "Boards: {} task(s), agents shown {}/{}",
            packet.task_boards.len(),
            packet.agents.shown,
            packet.agents.total
        ),
    ];
    for task in &packet.task_boards {
        lines.push(format!(
            "- task {} {} counts={:?}",
            task.task_id, task.status, task.counts_by_column
        ));
        for card in &task.cards {
            lines.push(format!(
                "  - {} {} {} {}",
                card.priority, card.id, card.column, card.title
            ));
        }
    }
    if !packet.memos.recent.is_empty() {
        lines.push("Memos:".to_string());
        for memo in &packet.memos.recent {
            lines.push(format!(
                "- {} {}: {}",
                memo.created_at, memo.kind, memo.content
            ));
        }
    }
    if !packet.prior_lessons.lessons.is_empty() {
        lines.push("Prior lessons:".to_string());
        for lesson in &packet.prior_lessons.lessons {
            lines.push(format!("- {}", lesson.lesson));
        }
    }
    if !packet.pending_questions.questions.is_empty() {
        lines.push("Pending questions:".to_string());
        for question in &packet.pending_questions.questions {
            lines.push(format!(
                "- blocking={} {}",
                question.blocking, question.question
            ));
        }
    }
    let text = lines.join("\n");
    redact_and_cap_text(&text, MAX_PACKET_TEXT_CHARS)
}

fn trim_largest_task_cards(packet: &mut ConductorDecisionPacket) -> bool {
    let Some((index, _)) = packet
        .task_boards
        .iter()
        .enumerate()
        .filter(|(_, task)| !task.cards.is_empty())
        .max_by(|(_, left), (_, right)| left.cards.len().cmp(&right.cards.len()))
    else {
        return false;
    };
    packet.task_boards[index].cards.pop();
    packet.task_boards[index].cards_shown = packet.task_boards[index].cards.len();
    true
}

fn compact_card_text(packet: &mut ConductorDecisionPacket) -> bool {
    let mut changed = false;
    for task in &mut packet.task_boards {
        for card in &mut task.cards {
            if card.last_update.take().is_some() {
                changed = true;
            }
            if card.final_report_summary.take().is_some() {
                changed = true;
            }
        }
    }
    changed
}

fn push_marker(packet: &mut ConductorDecisionPacket, marker: &str) {
    if !packet
        .truncation_markers
        .iter()
        .any(|existing| existing == marker)
    {
        packet.truncation_markers.push(marker.to_string());
        packet.truncation_markers.sort();
    }
}

fn board_counts_by_column(board: &TaskBoard) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for card in &board.cards {
        *counts.entry(card.column.clone()).or_insert(0) += 1;
    }
    counts
}

fn capped_clean_list(
    label: &str,
    values: &[String],
    max_items: usize,
    max_chars: usize,
    markers: &mut BTreeSet<String>,
) -> Vec<String> {
    let mut items = values
        .iter()
        .take(max_items)
        .map(|value| clean(label, value, max_chars, markers))
        .collect::<Vec<_>>();
    items.sort();
    items.dedup();
    if values.len() > max_items {
        markers.insert(label.to_string());
    }
    items
}

fn clean(label: &str, value: &str, max_chars: usize, markers: &mut BTreeSet<String>) -> String {
    let without_code = omit_fenced_code_blocks(value);
    let normalized = without_code
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = redact_and_cap_text(&normalized, max_chars);
    if normalized.len() > max_chars || cleaned.contains("...[truncated]") || without_code != value {
        markers.insert(label.to_string());
    }
    cleaned
}

fn omit_fenced_code_blocks(value: &str) -> String {
    let mut output = Vec::new();
    let mut in_fence = false;
    for line in value.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if !in_fence {
                output.push("[code omitted]".to_string());
            }
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            output.push(line.to_string());
        }
    }
    output.join("\n")
}

fn remaining_u64(limit: Option<u64>, spent: u64) -> Option<u64> {
    limit.map(|limit| limit.saturating_sub(spent))
}

fn remaining_u32(limit: Option<u32>, spent: u32) -> Option<u32> {
    limit.map(|limit| limit.saturating_sub(spent))
}

fn remaining_f64(limit: Option<f64>, spent: Option<f64>) -> Option<f64> {
    Some((limit? - spent.unwrap_or_default()).max(0.0))
}

fn enum_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn wake_reason_name(reason: &ConductorWakeReason) -> String {
    enum_name(reason)
}

fn memo_kind_name(kind: MemoKind) -> String {
    enum_name(&kind)
}

fn priority_rank(priority: &str) -> u8 {
    match priority.to_ascii_uppercase().as_str() {
        "P0" => 0,
        "P1" => 1,
        "P2" => 2,
        _ => 3,
    }
}

fn column_rank(column: &str) -> u8 {
    match column {
        "doing" => 0,
        "failed" | "regressed" => 1,
        "planned" => 2,
        "done" => 3,
        _ => 4,
    }
}

fn agent_state(status: &ConductorAgentSnapshot) -> String {
    match status.column.as_str() {
        "done" => "done".to_string(),
        "failed" | "regressed" => "failed".to_string(),
        "doing" => match status.session_state.as_deref() {
            Some("error") | Some("Error") => "stuck".to_string(),
            Some("paused")
            | Some("Paused")
            | Some("waiting_user_input")
            | Some("WaitingUserInput")
            | Some("waiting_ide")
            | Some("WaitingIde") => "paused".to_string(),
            Some("generating")
            | Some("Generating")
            | Some("executing_tools")
            | Some("ExecutingTools") => "running".to_string(),
            Some("idle") | Some("Idle") | None => "stuck".to_string(),
            _ => "running".to_string(),
        },
        _ => "running".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{BoardCard, ScopeGuardMode, StatusUpdate, TaskStatus};
    use refact_buddy_core::conductor::{
        ConductorMemo, DoneWhen, GoalAutonomy, GoalBudget, GoalBudgetSpent, GoalLedger,
        PendingQuestion,
    };

    fn goal() -> ConductorGoal {
        ConductorGoal {
            id: "goal-1".to_string(),
            title: "Ship Buddy Conductor".to_string(),
            plan_doc_slug: Some("master-plan".to_string()),
            plan_markdown: "# Ship Buddy Conductor\n```rust\nfn raw_secret_source() {}\n```"
                .to_string(),
            done_when: DoneWhen {
                summary: "All conductor cards are done".to_string(),
                checklist: vec!["packet exists".to_string(), "tests pass".to_string()],
            },
            status: GoalStatus::Running,
            autonomy: GoalAutonomy::FullAuto,
            budget: GoalBudget {
                wall_clock_secs: Some(7200),
                no_progress_wakes: Some(4),
                total_tokens: Some(10_000),
                usd: Some(3.0),
            },
            spent: GoalBudgetSpent {
                elapsed_secs: 1200,
                prompt_tokens: 1600,
                completion_tokens: 900,
                total_tokens: 2500,
                cache_read_tokens: 400,
                usd: Some(0.7),
                no_progress_wakes: 1,
            },
            ledger: GoalLedger {
                title: Some("Ship Buddy Conductor".to_string()),
                plan_doc_slug: Some("master-plan".to_string()),
                plan_markdown: Some("# Ship Buddy Conductor".to_string()),
                done_when: Some(DoneWhen {
                    summary: "All conductor cards are complete".to_string(),
                    checklist: vec!["routes".to_string(), "scheduler".to_string()],
                }),
                budget: Some(GoalBudget {
                    wall_clock_secs: Some(7200),
                    no_progress_wakes: Some(3),
                    total_tokens: Some(10_000),
                    usd: Some(3.0),
                }),
                created_at: Some("2026-06-03T00:00:00Z".to_string()),
                updated_at: Some("2026-06-03T00:04:00Z".to_string()),
                completed_at: None,
                status: Some(GoalStatus::Running),
                autonomy: Some(GoalAutonomy::FullAuto),
                planner_task_id: Some("task-1".to_string()),
                task_ids: vec!["task-1".to_string()],
                chat_ids: vec!["planner-chat".to_string()],
                memos: vec![
                    ConductorMemo {
                        id: "memo-2".to_string(),
                        kind: MemoKind::Risk,
                        content: format!(
                            "Long memo with token=rawsecret and {}",
                            "memo-body ".repeat(100)
                        ),
                        created_at: "2026-06-03T00:02:00Z".to_string(),
                        source_chat_id: Some("chat-2".to_string()),
                        related_task_id: Some("task-1".to_string()),
                    },
                    ConductorMemo {
                        id: "memo-1".to_string(),
                        kind: MemoKind::Decision,
                        content: "Use bounded packet inputs".to_string(),
                        created_at: "2026-06-03T00:01:00Z".to_string(),
                        source_chat_id: Some("chat-1".to_string()),
                        related_task_id: Some("task-1".to_string()),
                    },
                ],
                learning_records: Vec::new(),
                ghost_messages: Vec::new(),
                recurring: None,
                pending_questions: vec![PendingQuestion {
                    id: "question-1".to_string(),
                    question: "Should conductor continue automatically?".to_string(),
                    asked_at: "2026-06-03T00:03:00Z".to_string(),
                    source_chat_id: Some("planner-chat".to_string()),
                    blocking: true,
                    answer: None,
                    answered_at: None,
                }],
                no_progress_wakes: 1,
                turn_failures: 0,
                last_wake_at: Some("2026-06-03T00:05:00Z".to_string()),
                last_progress_at: Some("2026-06-03T00:04:00Z".to_string()),
                last_wake_reason: Some(ConductorWakeReason::TaskBoard),
            },
            created_at: Some("2026-06-03T00:00:00Z".to_string()),
            updated_at: Some("2026-06-03T00:04:00Z".to_string()),
            completed_at: None,
        }
    }

    fn task_meta() -> TaskMeta {
        TaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Conductor Task".to_string(),
            status: TaskStatus::Active,
            created_at: "2026-06-03T00:00:00Z".to_string(),
            updated_at: "2026-06-03T00:10:00Z".to_string(),
            cards_total: 3,
            cards_done: 1,
            cards_failed: 0,
            agents_active: 1,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: Some("generating".to_string()),
            conductor: None,
        }
    }

    fn card(id: &str, title: &str, column: &str, priority: &str) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: title.to_string(),
            column: column.to_string(),
            priority: priority.to_string(),
            depends_on: Vec::new(),
            instructions: "RAW_CONTEXT_FILE_BODY fn should_not_leak() { println!(\"secret\"); }"
                .to_string(),
            assignee: None,
            agent_chat_id: Some(format!("agent-{id}")),
            status_updates: vec![StatusUpdate {
                timestamp: "2026-06-03T00:05:00Z".to_string(),
                message: format!("{} update", title),
            }],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: "2026-06-03T00:00:00Z".to_string(),
            started_at: Some("2026-06-03T00:01:00Z".to_string()),
            last_heartbeat_at: Some("2026-06-03T00:05:00Z".to_string()),
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

    fn input() -> ConductorPacketInput {
        ConductorPacketInput {
            goal: goal(),
            wake_reasons: vec![ConductorWakeReason::Manual, ConductorWakeReason::TaskBoard],
            task_boards: vec![ConductorTaskSnapshot {
                meta: task_meta(),
                board: TaskBoard {
                    cards: vec![
                        card("T-2", "Build packet", "doing", "P1"),
                        card("T-1", "Design packet", "done", "P0"),
                        card("T-3", "Verify packet", "planned", "P2"),
                    ],
                    ..TaskBoard::default()
                },
            }],
            agent_statuses: vec![ConductorAgentSnapshot {
                task_id: "task-1".to_string(),
                card_id: "T-2".to_string(),
                card_title: "Build packet".to_string(),
                agent_chat_id: "agent-T-2".to_string(),
                column: "doing".to_string(),
                priority: "P1".to_string(),
                session_state: Some("generating".to_string()),
                last_activity_at: Some("2026-06-03T00:05:00Z".to_string()),
                last_status_update: Some("2026-06-03T00:05:00Z: working".to_string()),
                final_report: None,
                last_tool_name: Some("cat".to_string()),
            }],
            last_wake_at: Some("2026-06-03T00:06:00Z".to_string()),
            prior_lessons: Vec::new(),
        }
    }

    #[test]
    fn conductor_packet_contains_required_goal_wake_task_and_memo_fields() {
        let built = build_conductor_packet(input());
        let packet = &built.packet;

        assert_eq!(packet.goal.id, "goal-1");
        assert_eq!(packet.goal.plan_doc_slug.as_deref(), Some("master-plan"));
        assert_eq!(packet.goal.autonomy, GoalAutonomy::FullAuto);
        assert_eq!(
            packet.goal.done_when.summary,
            "All conductor cards are done"
        );
        assert_eq!(packet.budget.wall_clock_secs_remaining, Some(6000));
        assert_eq!(packet.wake.reasons, vec!["manual", "task_board"]);
        assert_eq!(packet.wake.no_progress_counter, 1);
        assert_eq!(packet.task_boards[0].task_id, "task-1");
        assert_eq!(packet.planner.planner_task_id.as_deref(), Some("task-1"));
        assert_eq!(packet.agents.counts_by_state.get("running"), Some(&1));
        assert_eq!(packet.memos.counts_by_kind.get("decision"), Some(&1));
        assert_eq!(packet.pending_questions.blocking_open, 1);
        assert!(built.text.contains("Wake reasons: manual, task_board"));
    }

    #[test]
    fn conductor_packet_output_is_deterministic() {
        let left = build_conductor_packet(input());
        let right = build_conductor_packet(input());

        assert_eq!(left.json, right.json);
        assert_eq!(left.text, right.text);
        assert_eq!(left.packet, right.packet);
    }

    #[test]
    fn conductor_packet_truncates_long_memo_and_board_fields() {
        let mut input = input();
        input.task_boards[0].board.cards[0].title = "long-card-title ".repeat(40);
        let built = build_conductor_packet(input);
        let json = built.json;

        assert!(json.len() <= MAX_PACKET_JSON_CHARS);
        assert!(json.contains("...[truncated]"));
        assert!(built
            .packet
            .truncation_markers
            .contains(&"memo.content".to_string()));
        assert!(built
            .packet
            .truncation_markers
            .contains(&"card.title".to_string()));
        assert!(!json.contains("rawsecret"));
    }

    #[test]
    fn conductor_packet_includes_wake_reasons() {
        let built = build_conductor_packet(input());

        assert!(built.json.contains("task_board"));
        assert!(built.json.contains("manual"));
        assert_eq!(
            built.packet.wake.last_wake_reason.as_deref(),
            Some("task_board")
        );
        assert_eq!(
            built.packet.wake.last_wake_at.as_deref(),
            Some("2026-06-03T00:06:00Z")
        );
    }

    #[test]
    fn conductor_packet_excludes_raw_plan_code_and_context_file_bodies() {
        let built = build_conductor_packet(input());
        let output = format!("{}\n{}", built.json, built.text);

        assert!(!output.contains("raw_secret_source"));
        assert!(!output.contains("RAW_CONTEXT_FILE_BODY"));
        assert!(!output.contains("should_not_leak"));
        assert!(output.contains("master-plan"));
    }
}
