use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::overlap_order::{order_wave, OverlapInputs, WaveOrder};
use crate::types::TaskBoard;

pub fn is_terminal_column(column: &str) -> bool {
    matches!(column, "done" | "failed" | "regressed")
}

pub fn is_failed_column(column: &str) -> bool {
    matches!(column, "failed" | "regressed")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wave {
    pub index: usize,
    pub cards: Vec<String>,
}

/// Topological levels of the board's dependency DAG.
///
/// A card sits one level below its deepest dependency, so a wave can never
/// drift from `depends_on`. Cycles, self-dependencies, duplicates and dangling
/// references are rejected by the board's own `validate_dependencies`.
pub fn derive_waves(board: &TaskBoard) -> Result<Vec<Wave>, String> {
    board.validate_dependencies()?;
    let count = board.cards.len();
    let index_by_id: HashMap<&str, usize> = board
        .cards
        .iter()
        .enumerate()
        .map(|(index, card)| (card.id.as_str(), index))
        .collect();
    let mut indegree: Vec<usize> = board
        .cards
        .iter()
        .map(|card| card.depends_on.len())
        .collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (index, card) in board.cards.iter().enumerate() {
        for dependency in &card.depends_on {
            dependents[index_by_id[dependency.as_str()]].push(index);
        }
    }
    let mut levels = vec![0usize; count];
    let mut queue: VecDeque<usize> = (0..count).filter(|index| indegree[*index] == 0).collect();
    while let Some(index) = queue.pop_front() {
        for dependent in dependents[index].clone() {
            levels[dependent] = levels[dependent].max(levels[index] + 1);
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }
    let depth = levels.iter().copied().max().map(|max| max + 1).unwrap_or(0);
    let mut waves: Vec<Wave> = (0..depth)
        .map(|index| Wave {
            index,
            cards: Vec::new(),
        })
        .collect();
    for (index, card) in board.cards.iter().enumerate() {
        waves[levels[index]].cards.push(card.id.clone());
    }
    Ok(waves)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneKind {
    Parallel,
    Sequential,
    Pipeline,
}

impl LaneKind {
    pub const NAMES: &'static [&'static str] = &["parallel", "sequential", "pipeline"];

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "parallel" => Ok(LaneKind::Parallel),
            "sequential" => Ok(LaneKind::Sequential),
            "pipeline" => Ok(LaneKind::Pipeline),
            other => Err(format!(
                "unknown lane kind '{other}', expected one of {}",
                LaneKind::NAMES.join(", ")
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LaneKind::Parallel => "parallel",
            LaneKind::Sequential => "sequential",
            LaneKind::Pipeline => "pipeline",
        }
    }

    fn advances_one_position_at_a_time(&self) -> bool {
        matches!(self, LaneKind::Sequential | LaneKind::Pipeline)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneStage {
    pub card_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane {
    pub name: String,
    pub kind: LaneKind,
    pub stages: Vec<LaneStage>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveLimits {
    pub max_wave_width: usize,
    pub max_pipeline_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "advance", rename_all = "snake_case")]
pub enum PipelineAdvance {
    Next {
        position: usize,
        card_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },
    Complete,
    OutOfRange {
        position: usize,
    },
}

/// Move to the stage after `completed_position`.
///
/// The cursor is the position, never the agent it happens to name: a pipeline
/// may list the same agent at several positions and each occurrence must
/// advance to its own successor.
pub fn advance_pipeline(stages: &[LaneStage], completed_position: usize) -> PipelineAdvance {
    if completed_position >= stages.len() {
        return PipelineAdvance::OutOfRange {
            position: completed_position,
        };
    }
    match stages.get(completed_position + 1) {
        Some(stage) => PipelineAdvance::Next {
            position: completed_position + 1,
            card_id: stage.card_id.clone(),
            agent_id: stage.agent_id.clone(),
        },
        None => PipelineAdvance::Complete,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cursor", rename_all = "snake_case")]
pub enum PipelineCursor {
    At { position: usize },
    Blocked { position: usize, card_id: String },
    Complete,
}

pub fn pipeline_cursor(board: &TaskBoard, stages: &[LaneStage]) -> PipelineCursor {
    let mut position = 0usize;
    loop {
        let Some(stage) = stages.get(position) else {
            return PipelineCursor::Complete;
        };
        let column = board
            .get_card(&stage.card_id)
            .map(|card| card.column.as_str())
            .unwrap_or("");
        if is_failed_column(column) {
            return PipelineCursor::Blocked {
                position,
                card_id: stage.card_id.clone(),
            };
        }
        if !is_terminal_column(column) {
            return PipelineCursor::At {
                position,
            };
        }
        match advance_pipeline(stages, position) {
            PipelineAdvance::Next {
                position: next, ..
            } => position = next,
            PipelineAdvance::Complete
            | PipelineAdvance::OutOfRange {
                ..
            } => return PipelineCursor::Complete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StageDecision {
    Dispatch,
    NotRun { reason: String },
    Blocked { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageOutcome {
    pub card_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub position: usize,
    pub wave: usize,
    pub decision: StageDecision,
    /// Submission rank inside this stage's wave. Absent when overlap ordering
    /// contributed nothing, in which case the wave keeps its lane order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_rank: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneStatus {
    Dispatched,
    NotRun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneOutcome {
    pub lane: String,
    pub kind: LaneKind,
    pub status: LaneStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub optional: bool,
    pub stages: Vec<StageOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaveSchedule {
    pub waves: Vec<Wave>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admitted_wave: Option<usize>,
    pub lanes: Vec<LaneOutcome>,
    pub dispatch: Vec<String>,
    /// One entry per wave that has something to submit, present only when
    /// overlap ordering ran. Empty means the wave kept its lane order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlap_order: Vec<WaveOrder>,
}

impl WaveSchedule {
    /// `dispatch`, cut into groups that may be submitted together.
    ///
    /// Cards in one group have no overlap known to this schedule. Groups are
    /// submitted one after another, which is the whole effect of ordering:
    /// nothing is withheld, only sequenced. Without ordering there is exactly
    /// one group, so a caller that honours groups behaves as it did before.
    pub fn submission_groups(&self) -> Vec<Vec<String>> {
        let ranks: HashMap<&str, (usize, usize)> = self
            .lanes
            .iter()
            .flat_map(|lane| lane.stages.iter())
            .filter_map(|stage| {
                stage
                    .serial_rank
                    .map(|rank| (stage.card_id.as_str(), (stage.wave, rank)))
            })
            .collect();
        let mut groups: Vec<Vec<String>> = Vec::new();
        let mut current: Option<(usize, usize)> = None;
        for card_id in &self.dispatch {
            let key = ranks.get(card_id.as_str()).copied();
            match (key, groups.last_mut()) {
                (Some(key), Some(group)) if current == Some(key) => group.push(card_id.clone()),
                (None, Some(group)) if current.is_none() => group.push(card_id.clone()),
                _ => {
                    current = key;
                    groups.push(vec![card_id.clone()]);
                }
            }
        }
        groups
    }
}

struct WaveGate {
    frontier: usize,
    holding: Vec<Vec<String>>,
    failed: Vec<Vec<String>>,
    blocked_at: Option<usize>,
}

fn wave_gate(board: &TaskBoard, waves: &[Wave], optional: &HashSet<&str>) -> WaveGate {
    let mut holding = Vec::with_capacity(waves.len());
    let mut failed = Vec::with_capacity(waves.len());
    for wave in waves {
        let mut holding_wave = Vec::new();
        let mut failed_wave = Vec::new();
        for card_id in &wave.cards {
            if optional.contains(card_id.as_str()) {
                continue;
            }
            let column = board
                .get_card(card_id)
                .map(|card| card.column.as_str())
                .unwrap_or("");
            if is_failed_column(column) {
                failed_wave.push(card_id.clone());
            } else if !is_terminal_column(column) {
                holding_wave.push(card_id.clone());
            }
        }
        holding.push(holding_wave);
        failed.push(failed_wave);
    }
    let frontier = holding
        .iter()
        .position(|wave| !wave.is_empty())
        .unwrap_or(waves.len());
    let blocked_at = failed.iter().position(|wave| !wave.is_empty());
    WaveGate {
        frontier,
        holding,
        failed,
        blocked_at,
    }
}

fn validate_lanes(board: &TaskBoard, lanes: &[Lane], limits: &WaveLimits) -> Result<(), String> {
    let mut lane_names = HashSet::new();
    let mut placed = HashSet::new();
    for lane in lanes {
        if lane.name.trim().is_empty() {
            return Err("lane name cannot be empty".to_string());
        }
        if !lane_names.insert(lane.name.as_str()) {
            return Err(format!("lane '{}' is declared more than once", lane.name));
        }
        if lane.kind == LaneKind::Pipeline && lane.stages.len() > limits.max_pipeline_depth {
            return Err(format!(
                "pipeline lane '{}' declares {} stages, exceeding the configured maximum pipeline depth of {}",
                lane.name,
                lane.stages.len(),
                limits.max_pipeline_depth
            ));
        }
        for stage in &lane.stages {
            if board.get_card(&stage.card_id).is_none() {
                return Err(format!(
                    "lane '{}' stage references unknown card {}",
                    lane.name, stage.card_id
                ));
            }
            if !placed.insert(stage.card_id.as_str()) {
                return Err(format!(
                    "card {} is placed in more than one lane stage",
                    stage.card_id
                ));
            }
        }
    }
    Ok(())
}

/// Decide, against the board as it stands now, which of the requested lane
/// stages may be submitted.
///
/// Waves only order submission: everything this returns still has to pass the
/// batch's own readiness validation and the board's claim CAS.
pub fn plan_schedule(
    board: &TaskBoard,
    lanes: &[Lane],
    limits: &WaveLimits,
) -> Result<WaveSchedule, String> {
    plan_schedule_with_overlap(board, lanes, limits, None)
}

/// The same schedule, with cards that are known to touch the same files
/// sequenced inside their wave.
///
/// Overlap can only reorder. It never changes which stages were decided
/// `Dispatch`, never reaches across waves, and `None` reproduces
/// `plan_schedule` exactly.
pub fn plan_schedule_with_overlap(
    board: &TaskBoard,
    lanes: &[Lane],
    limits: &WaveLimits,
    overlap: Option<&OverlapInputs>,
) -> Result<WaveSchedule, String> {
    let waves = derive_waves(board)?;
    validate_lanes(board, lanes, limits)?;

    let optional: HashSet<&str> = lanes
        .iter()
        .flat_map(|lane| {
            lane.stages
                .iter()
                .filter(move |stage| lane.optional || stage.optional)
                .map(|stage| stage.card_id.as_str())
        })
        .collect();
    let wave_of: HashMap<&str, usize> = waves
        .iter()
        .flat_map(|wave| {
            wave.cards
                .iter()
                .map(move |card_id| (card_id.as_str(), wave.index))
        })
        .collect();
    let gate = wave_gate(board, &waves, &optional);

    let mut outcomes: Vec<LaneOutcome> = Vec::with_capacity(lanes.len());
    for lane in lanes {
        let cursor = lane
            .kind
            .advances_one_position_at_a_time()
            .then(|| pipeline_cursor(board, &lane.stages));
        let mut stages = Vec::with_capacity(lane.stages.len());
        for (position, stage) in lane.stages.iter().enumerate() {
            let wave = wave_of
                .get(stage.card_id.as_str())
                .copied()
                .unwrap_or_default();
            let decision = decide_stage(board, &gate, lane, cursor.as_ref(), position, wave);
            stages.push(StageOutcome {
                card_id: stage.card_id.clone(),
                agent_id: stage.agent_id.clone(),
                position,
                wave,
                decision,
                serial_rank: None,
            });
        }
        outcomes.push(LaneOutcome {
            lane: lane.name.clone(),
            kind: lane.kind,
            status: LaneStatus::NotRun,
            reason: None,
            optional: lane.optional,
            stages,
        });
    }

    let overlap_order = overlap
        .filter(|inputs| !inputs.is_empty())
        .map(|inputs| order_dispatch_by_wave(&outcomes, inputs))
        .unwrap_or_default();
    let ranks = stamp_ranks(&mut outcomes, &overlap_order);
    let order = dispatch_order(&outcomes, ranks.as_ref());
    throttle_wave_width(&mut outcomes, &order, limits.max_wave_width);

    let dispatch: Vec<String> = order
        .iter()
        .filter(|(lane_index, stage_index)| {
            outcomes[*lane_index].stages[*stage_index].decision == StageDecision::Dispatch
        })
        .map(|(lane_index, stage_index)| outcomes[*lane_index].stages[*stage_index].card_id.clone())
        .collect();
    for outcome in &mut outcomes {
        if outcome
            .stages
            .iter()
            .any(|stage| stage.decision == StageDecision::Dispatch)
        {
            outcome.status = LaneStatus::Dispatched;
            outcome.reason = None;
        } else {
            outcome.status = LaneStatus::NotRun;
            outcome.reason = Some(lane_not_run_reason(outcome));
        }
    }

    let admitted_wave = gate
        .blocked_at
        .map_or(gate.frontier, |blocked| gate.frontier.min(blocked));
    Ok(WaveSchedule {
        admitted_wave: (admitted_wave < waves.len()).then_some(admitted_wave),
        waves,
        lanes: outcomes,
        dispatch,
        overlap_order,
    })
}

/// Rank each wave's submittable cards on its own. Ordering inside a wave can
/// never move a card past a wave boundary.
fn order_dispatch_by_wave(outcomes: &[LaneOutcome], inputs: &OverlapInputs) -> Vec<WaveOrder> {
    let mut by_wave: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for outcome in outcomes {
        for stage in &outcome.stages {
            if stage.decision == StageDecision::Dispatch {
                by_wave
                    .entry(stage.wave)
                    .or_default()
                    .push(stage.card_id.clone());
            }
        }
    }
    by_wave
        .into_iter()
        .map(|(wave, card_ids)| order_wave(wave, &card_ids, inputs))
        .collect()
}

/// Record the ranks on the stages they belong to. Returns `None` when nothing
/// was ranked, which is what keeps an unordered schedule in lane order.
fn stamp_ranks(
    outcomes: &mut [LaneOutcome],
    overlap_order: &[WaveOrder],
) -> Option<BTreeMap<String, usize>> {
    if overlap_order.is_empty() {
        return None;
    }
    let ranks: BTreeMap<String, usize> = overlap_order
        .iter()
        .flat_map(|order| order.ranks())
        .collect();
    for outcome in outcomes.iter_mut() {
        for stage in outcome.stages.iter_mut() {
            stage.serial_rank = ranks.get(&stage.card_id).copied();
        }
    }
    Some(ranks)
}

/// The submission order of everything decided `Dispatch`.
///
/// Without ranks this is lane order, exactly as the schedule declared it. With
/// ranks it is wave, then rank, then lane order — a permutation of the same
/// set, never a smaller one.
fn dispatch_order(
    outcomes: &[LaneOutcome],
    ranks: Option<&BTreeMap<String, usize>>,
) -> Vec<(usize, usize)> {
    let mut order: Vec<(usize, usize)> = Vec::new();
    for (lane_index, outcome) in outcomes.iter().enumerate() {
        for (stage_index, stage) in outcome.stages.iter().enumerate() {
            if stage.decision == StageDecision::Dispatch {
                order.push((lane_index, stage_index));
            }
        }
    }
    let Some(ranks) = ranks else {
        return order;
    };
    order.sort_by_key(|(lane_index, stage_index)| {
        let stage = &outcomes[*lane_index].stages[*stage_index];
        (
            stage.wave,
            ranks.get(&stage.card_id).copied().unwrap_or_default(),
        )
    });
    order
}

fn decide_stage(
    board: &TaskBoard,
    gate: &WaveGate,
    lane: &Lane,
    cursor: Option<&PipelineCursor>,
    position: usize,
    wave: usize,
) -> StageDecision {
    if let Some(blocked) = gate.blocked_at {
        if wave > blocked {
            return StageDecision::Blocked {
                reason: format!(
                    "wave {blocked} required card(s) {} failed",
                    gate.failed[blocked].join(", ")
                ),
            };
        }
    }
    if wave > gate.frontier {
        return StageDecision::NotRun {
            reason: format!(
                "wave {} is still running required card(s) {}",
                gate.frontier,
                gate.holding
                    .get(gate.frontier)
                    .map(|cards| cards.join(", "))
                    .unwrap_or_default()
            ),
        };
    }
    match cursor {
        Some(PipelineCursor::Complete) => StageDecision::NotRun {
            reason: format!(
                "{} lane '{}' has no remaining stage",
                lane.kind.as_str(),
                lane.name
            ),
        },
        Some(PipelineCursor::Blocked {
            position: failed_position,
            card_id,
        }) => StageDecision::Blocked {
            reason: format!(
                "{} lane '{}' stopped at position {failed_position}: card {card_id} failed",
                lane.kind.as_str(),
                lane.name
            ),
        },
        Some(PipelineCursor::At {
            position: current_position,
        }) => {
            if position == *current_position {
                StageDecision::Dispatch
            } else if position < *current_position {
                StageDecision::NotRun {
                    reason: format!(
                        "{} lane '{}' already passed position {position}",
                        lane.kind.as_str(),
                        lane.name
                    ),
                }
            } else {
                StageDecision::NotRun {
                    reason: format!(
                        "{} lane '{}' is at position {current_position}",
                        lane.kind.as_str(),
                        lane.name
                    ),
                }
            }
        }
        None => terminal_or_dispatch(board, lane, position),
    }
}

fn terminal_or_dispatch(board: &TaskBoard, lane: &Lane, position: usize) -> StageDecision {
    let stage = &lane.stages[position];
    let column = board
        .get_card(&stage.card_id)
        .map(|card| card.column.as_str())
        .unwrap_or("");
    if is_terminal_column(column) {
        return StageDecision::NotRun {
            reason: format!("card {} is already in '{column}'", stage.card_id),
        };
    }
    StageDecision::Dispatch
}

fn throttle_wave_width(
    outcomes: &mut [LaneOutcome],
    order: &[(usize, usize)],
    max_wave_width: usize,
) {
    for (lane_index, stage_index) in order.iter().skip(max_wave_width) {
        outcomes[*lane_index].stages[*stage_index].decision = StageDecision::NotRun {
            reason: format!(
                "wave width limit of {max_wave_width} card(s) per submission was reached"
            ),
        };
    }
}

fn lane_not_run_reason(outcome: &LaneOutcome) -> String {
    match outcome.stages.first() {
        None => format!("lane '{}' declares no stages", outcome.lane),
        Some(stage) => match &stage.decision {
            StageDecision::NotRun {
                reason,
            }
            | StageDecision::Blocked {
                reason,
            } => reason.clone(),
            StageDecision::Dispatch => unreachable!("a dispatched stage marks its lane dispatched"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlap_order::OverlapSource;
    use crate::types::BoardCard;

    fn card(id: &str, column: &str, depends_on: &[&str]) -> BoardCard {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": id,
            "column": column,
            "depends_on": depends_on,
            "assignee": null,
            "agent_chat_id": null,
            "created_at": "2026-05-16T00:00:00Z",
            "started_at": null,
            "completed_at": null,
        }))
        .unwrap()
    }

    fn board(cards: Vec<BoardCard>) -> TaskBoard {
        TaskBoard {
            cards,
            ..TaskBoard::default()
        }
    }

    fn stage(card_id: &str, agent_id: Option<&str>) -> LaneStage {
        LaneStage {
            card_id: card_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            optional: false,
        }
    }

    fn limits() -> WaveLimits {
        WaveLimits {
            max_wave_width: 8,
            max_pipeline_depth: 16,
        }
    }

    fn fixture_dag(columns: &[(&str, &str)]) -> TaskBoard {
        let deps: HashMap<&str, Vec<&str>> = HashMap::from([
            ("T-1", vec![]),
            ("T-2", vec![]),
            ("T-3", vec!["T-1"]),
            ("T-4", vec!["T-1", "T-2"]),
            ("T-5", vec!["T-4"]),
        ]);
        board(
            columns
                .iter()
                .map(|(id, column)| card(id, column, &deps[id]))
                .collect(),
        )
    }

    fn planned_fixture() -> TaskBoard {
        fixture_dag(&[
            ("T-1", "planned"),
            ("T-2", "planned"),
            ("T-3", "planned"),
            ("T-4", "planned"),
            ("T-5", "planned"),
        ])
    }

    fn parallel_lane(name: &str, card_ids: &[&str]) -> Lane {
        Lane {
            name: name.to_string(),
            kind: LaneKind::Parallel,
            stages: card_ids.iter().map(|id| stage(id, None)).collect(),
            optional: false,
        }
    }

    #[test]
    fn waves_derive_deterministically_from_the_dependency_dag() {
        let board = planned_fixture();

        let first = derive_waves(&board).unwrap();
        let second = derive_waves(&board).unwrap();

        assert_eq!(first, second);
        assert_eq!(
            first,
            vec![
                Wave {
                    index: 0,
                    cards: vec!["T-1".to_string(), "T-2".to_string()],
                },
                Wave {
                    index: 1,
                    cards: vec!["T-3".to_string(), "T-4".to_string()],
                },
                Wave {
                    index: 2,
                    cards: vec!["T-5".to_string()],
                },
            ]
        );
    }

    #[test]
    fn wave_derivation_reuses_the_boards_own_cycle_detector() {
        let board = board(vec![
            card("T-1", "planned", &["T-2"]),
            card("T-2", "planned", &["T-1"]),
        ]);

        let error = derive_waves(&board).unwrap_err();

        assert!(error.contains("cycle"), "unexpected error: {error}");
    }

    #[test]
    fn wave_two_is_not_admitted_while_a_required_wave_one_card_is_not_terminal() {
        let board = fixture_dag(&[
            ("T-1", "done"),
            ("T-2", "done"),
            ("T-3", "doing"),
            ("T-4", "planned"),
            ("T-5", "planned"),
        ]);
        let lanes = vec![parallel_lane("build", &["T-4", "T-5"])];

        let schedule = plan_schedule(&board, &lanes, &limits()).unwrap();

        assert_eq!(schedule.dispatch, vec!["T-4".to_string()]);
        let held = &schedule.lanes[0].stages[1];
        assert_eq!(held.card_id, "T-5");
        assert_eq!(
            held.decision,
            StageDecision::NotRun {
                reason: "wave 1 is still running required card(s) T-3, T-4".to_string(),
            }
        );
    }

    #[test]
    fn an_optional_wave_card_does_not_hold_the_wave_open() {
        let board = fixture_dag(&[
            ("T-1", "done"),
            ("T-2", "done"),
            ("T-3", "doing"),
            ("T-4", "done"),
            ("T-5", "planned"),
        ]);
        let optional_lane = Lane {
            name: "docs".to_string(),
            kind: LaneKind::Parallel,
            stages: vec![stage("T-3", None)],
            optional: true,
        };
        let lanes = vec![optional_lane, parallel_lane("ship", &["T-5"])];

        let schedule = plan_schedule(&board, &lanes, &limits()).unwrap();

        assert!(schedule.dispatch.contains(&"T-5".to_string()));
        assert_eq!(
            schedule.lanes[1].stages[0].decision,
            StageDecision::Dispatch
        );
    }

    #[test]
    fn a_pipeline_cursor_walks_repeated_agents_by_position() {
        let board = board(vec![
            card("T-1", "done", &[]),
            card("T-2", "done", &["T-1"]),
            card("T-3", "done", &["T-2"]),
            card("T-4", "planned", &["T-3"]),
        ]);
        let stages = vec![
            stage("T-1", Some("reviewer")),
            stage("T-2", Some("builder")),
            stage("T-3", Some("reviewer")),
            stage("T-4", Some("shipper")),
        ];

        assert_eq!(
            pipeline_cursor(&board, &stages),
            PipelineCursor::At {
                position: 3
            }
        );
    }

    #[test]
    fn a_sequential_lane_dispatches_only_its_current_position() {
        let board = planned_fixture();
        let lanes = vec![Lane {
            name: "serial".to_string(),
            kind: LaneKind::Sequential,
            stages: vec![stage("T-1", None), stage("T-2", None)],
            optional: false,
        }];

        let schedule = plan_schedule(&board, &lanes, &limits()).unwrap();

        assert_eq!(schedule.dispatch, vec!["T-1".to_string()]);
        assert_eq!(
            schedule.lanes[0].stages[1].decision,
            StageDecision::NotRun {
                reason: "sequential lane 'serial' is at position 0".to_string(),
            }
        );
    }

    #[test]
    fn a_failed_required_card_blocks_its_waves_successors_with_a_structured_reason() {
        let board = fixture_dag(&[
            ("T-1", "failed"),
            ("T-2", "done"),
            ("T-3", "planned"),
            ("T-4", "planned"),
            ("T-5", "planned"),
        ]);
        let lanes = vec![parallel_lane("next", &["T-3", "T-4", "T-5"])];

        let schedule = plan_schedule(&board, &lanes, &limits()).unwrap();

        assert!(schedule.dispatch.is_empty());
        for stage in &schedule.lanes[0].stages {
            assert_eq!(
                stage.decision,
                StageDecision::Blocked {
                    reason: "wave 0 required card(s) T-1 failed".to_string(),
                },
                "stage {} should be blocked",
                stage.card_id
            );
        }
    }

    #[test]
    fn a_failed_card_does_not_block_its_own_wave_siblings() {
        let board = fixture_dag(&[
            ("T-1", "failed"),
            ("T-2", "planned"),
            ("T-3", "planned"),
            ("T-4", "planned"),
            ("T-5", "planned"),
        ]);
        let lanes = vec![parallel_lane("roots", &["T-2"])];

        let schedule = plan_schedule(&board, &lanes, &limits()).unwrap();

        assert_eq!(schedule.dispatch, vec!["T-2".to_string()]);
    }

    #[test]
    fn a_failed_pipeline_position_blocks_the_rest_of_its_lane() {
        let board = board(vec![
            card("T-1", "failed", &[]),
            card("T-2", "planned", &[]),
        ]);
        let lanes = vec![Lane {
            name: "chain".to_string(),
            kind: LaneKind::Pipeline,
            stages: vec![stage("T-1", Some("a")), stage("T-2", Some("b"))],
            optional: true,
        }];

        let schedule = plan_schedule(&board, &lanes, &limits()).unwrap();

        assert!(schedule.dispatch.is_empty());
        assert_eq!(
            schedule.lanes[0].reason.as_deref(),
            Some("pipeline lane 'chain' stopped at position 0: card T-1 failed")
        );
    }

    #[test]
    fn an_over_deep_pipeline_lane_is_refused() {
        let board = board(vec![
            card("T-1", "planned", &[]),
            card("T-2", "planned", &[]),
        ]);
        let lanes = vec![Lane {
            name: "chain".to_string(),
            kind: LaneKind::Pipeline,
            stages: vec![stage("T-1", Some("a")), stage("T-2", Some("b"))],
            optional: false,
        }];
        let shallow = WaveLimits {
            max_wave_width: 8,
            max_pipeline_depth: 1,
        };

        let error = plan_schedule(&board, &lanes, &shallow).unwrap_err();

        assert!(error.contains("maximum pipeline depth"), "{error}");
    }

    #[test]
    fn a_card_placed_in_two_lanes_is_refused() {
        let board = board(vec![card("T-1", "planned", &[])]);
        let lanes = vec![parallel_lane("a", &["T-1"]), parallel_lane("b", &["T-1"])];

        let error = plan_schedule(&board, &lanes, &limits()).unwrap_err();

        assert!(error.contains("more than one lane"), "{error}");
    }

    #[test]
    fn a_stage_naming_an_unknown_card_is_refused() {
        let board = board(vec![card("T-1", "planned", &[])]);
        let lanes = vec![parallel_lane("a", &["T-9"])];

        let error = plan_schedule(&board, &lanes, &limits()).unwrap_err();

        assert!(error.contains("unknown card T-9"), "{error}");
    }

    fn overlapping(pairs: &[(&str, &[&str])]) -> OverlapInputs {
        let mut inputs = OverlapInputs::new();
        for (card_id, files) in pairs {
            inputs.declare(card_id, files.iter().map(|file| file.to_string()));
        }
        inputs
    }

    #[test]
    fn overlap_order_serialises_two_cards_declaring_the_same_path() {
        let board = board(vec![
            card("T-1", "planned", &[]),
            card("T-2", "planned", &[]),
            card("T-3", "planned", &[]),
        ]);
        let lanes = vec![parallel_lane("roots", &["T-1", "T-2", "T-3"])];
        let inputs = overlapping(&[
            ("T-1", &["/repo/src/lib.rs"]),
            ("T-2", &["/repo/src/other.rs"]),
            ("T-3", &["/repo/src/lib.rs"]),
        ]);

        let schedule =
            plan_schedule_with_overlap(&board, &lanes, &limits(), Some(&inputs)).unwrap();

        assert_eq!(
            schedule.dispatch,
            vec!["T-1".to_string(), "T-2".to_string(), "T-3".to_string()],
            "the colliding card is submitted after, not instead of, its peer"
        );
        let ranks: Vec<Option<usize>> = schedule.lanes[0]
            .stages
            .iter()
            .map(|stage| stage.serial_rank)
            .collect();
        assert_eq!(ranks, vec![Some(0), Some(0), Some(1)]);
        assert_eq!(
            schedule.submission_groups(),
            vec![
                vec!["T-1".to_string(), "T-2".to_string()],
                vec!["T-3".to_string()]
            ],
            "cards that collide must not enter one submission group"
        );
    }

    #[test]
    fn overlap_order_never_gates_a_pathological_overlap_graph() {
        let board = board(vec![
            card("T-1", "planned", &[]),
            card("T-2", "planned", &[]),
            card("T-3", "planned", &[]),
            card("T-4", "planned", &[]),
            card("T-5", "planned", &[]),
        ]);
        let lanes = vec![parallel_lane("roots", &["T-1", "T-2", "T-3", "T-4", "T-5"])];
        let everything: &[&str] = &["/repo/src/shared.rs"];
        let inputs = overlapping(&[
            ("T-1", everything),
            ("T-2", everything),
            ("T-3", everything),
            ("T-4", everything),
            ("T-5", everything),
        ]);

        let unordered = plan_schedule(&board, &lanes, &limits()).unwrap();
        let ordered = plan_schedule_with_overlap(&board, &lanes, &limits(), Some(&inputs)).unwrap();

        assert_eq!(
            ordered.dispatch, unordered.dispatch,
            "a clique must dispatch exactly what it would have without ordering"
        );
        assert_eq!(ordered.dispatch.len(), 5, "no card is starved");
        assert!(
            ordered
                .lanes
                .iter()
                .flat_map(|lane| lane.stages.iter())
                .all(|stage| stage.decision == StageDecision::Dispatch),
            "ordering must not turn a dispatch into a refusal"
        );
        assert_eq!(ordered.admitted_wave, unordered.admitted_wave);
        assert_eq!(
            ordered.submission_groups().len(),
            5,
            "a clique is fully serialised, and every group still runs"
        );
        assert_eq!(
            ordered
                .submission_groups()
                .into_iter()
                .flatten()
                .collect::<Vec<String>>(),
            ordered.dispatch,
            "the groups partition the dispatch set exactly"
        );
    }

    #[test]
    fn overlap_order_keeps_declared_and_observed_apart_in_the_record() {
        let board = board(vec![
            card("T-1", "planned", &[]),
            card("T-2", "planned", &[]),
        ]);
        let lanes = vec![parallel_lane("roots", &["T-1", "T-2"])];
        let mut inputs = overlapping(&[("T-1", &["/repo/a.rs"]), ("T-2", &["/repo/a.rs"])]);
        inputs.observe("T-1", ["/repo/b.rs".to_string()]);
        inputs.observe("T-2", ["/repo/b.rs".to_string()]);

        let schedule =
            plan_schedule_with_overlap(&board, &lanes, &limits(), Some(&inputs)).unwrap();
        let order = &schedule.overlap_order[0];
        let later = order
            .cards
            .iter()
            .find(|entry| entry.card_id == "T-2")
            .unwrap();

        assert_eq!(order.wave, 0);
        assert_eq!(
            later
                .overlaps
                .iter()
                .map(|edge| (edge.source, edge.files.clone()))
                .collect::<Vec<_>>(),
            vec![
                (OverlapSource::Declared, vec!["/repo/a.rs".to_string()]),
                (OverlapSource::Observed, vec!["/repo/b.rs".to_string()]),
            ]
        );
        assert!(order.note.contains("not permission or a guarantee"));
    }

    #[test]
    fn overlap_order_does_not_reorder_across_waves() {
        let board = fixture_dag(&[
            ("T-1", "done"),
            ("T-2", "done"),
            ("T-3", "planned"),
            ("T-4", "planned"),
            ("T-5", "planned"),
        ]);
        let lanes = vec![
            Lane {
                name: "late".to_string(),
                kind: LaneKind::Parallel,
                stages: vec![stage("T-5", None)],
                optional: true,
            },
            parallel_lane("now", &["T-3", "T-4"]),
        ];
        let inputs = overlapping(&[
            ("T-3", &["/repo/shared.rs"]),
            ("T-4", &["/repo/shared.rs"]),
            ("T-5", &["/repo/shared.rs"]),
        ]);

        let schedule =
            plan_schedule_with_overlap(&board, &lanes, &limits(), Some(&inputs)).unwrap();

        assert_eq!(
            schedule.dispatch,
            vec!["T-3".to_string(), "T-4".to_string()],
            "a later wave stays later; ordering only acts inside a wave"
        );
        assert_eq!(schedule.overlap_order.len(), 1);
        assert_eq!(schedule.overlap_order[0].wave, 1);
    }

    #[test]
    fn overlap_order_decides_which_cards_the_width_limit_throttles() {
        let board = board(vec![
            card("T-1", "planned", &[]),
            card("T-2", "planned", &[]),
            card("T-3", "planned", &[]),
        ]);
        let lanes = vec![parallel_lane("roots", &["T-1", "T-2", "T-3"])];
        let inputs = overlapping(&[
            ("T-1", &["/repo/shared.rs"]),
            ("T-2", &["/repo/shared.rs"]),
            ("T-3", &["/repo/other.rs"]),
        ]);
        let narrow = WaveLimits {
            max_wave_width: 2,
            max_pipeline_depth: 16,
        };

        let schedule = plan_schedule_with_overlap(&board, &lanes, &narrow, Some(&inputs)).unwrap();

        assert_eq!(
            schedule.dispatch,
            vec!["T-1".to_string(), "T-3".to_string()],
            "the throttle keeps the two cards that do not collide"
        );
        assert_eq!(
            schedule.lanes[0].stages[1].decision,
            StageDecision::NotRun {
                reason: "wave width limit of 2 card(s) per submission was reached".to_string(),
            },
            "the deferral is the pre-existing width throttle, not a new refusal"
        );
    }

    #[test]
    fn overlap_order_ignores_a_card_the_wave_did_not_admit() {
        let board = fixture_dag(&[
            ("T-1", "planned"),
            ("T-2", "planned"),
            ("T-3", "planned"),
            ("T-4", "planned"),
            ("T-5", "planned"),
        ]);
        let lanes = vec![parallel_lane("roots", &["T-1", "T-2"]), {
            let mut lane = parallel_lane("later", &["T-5"]);
            lane.optional = true;
            lane
        }];
        let inputs = overlapping(&[("T-1", &["/repo/shared.rs"]), ("T-5", &["/repo/shared.rs"])]);

        let schedule =
            plan_schedule_with_overlap(&board, &lanes, &limits(), Some(&inputs)).unwrap();

        assert_eq!(
            schedule.overlap_order[0]
                .cards
                .iter()
                .map(|entry| entry.card_id.clone())
                .collect::<Vec<String>>(),
            vec!["T-1".to_string(), "T-2".to_string()],
            "an undispatched card is not ranked and cannot delay one that is"
        );
        assert_eq!(schedule.lanes[1].stages[0].serial_rank, None);
    }
}
