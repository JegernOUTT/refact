//! Telling "quiet" apart from "stuck".
//!
//! A silent run can be waiting by design, blocked behind a failing provider,
//! looping without achieving anything, or doing one long legitimate thing.
//! Collapsing those into a single stall verdict produces both false alarms and
//! missed loops, so they stay separate here.
//!
//! A no-material-progress verdict is only ever reached from *content* evidence:
//! identical work repeated, or material output that stopped changing. Elapsed
//! silence on its own is reported as [`ActivityClass::NoObservedActivity`], an
//! observation, and never as a verdict — see
//! [`ActivityClass::is_no_progress_verdict`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

use refact_chat_api::SessionState;
use refact_core::retry_policy::{UserErrorCategory, classify_user_error, user_error_info};
use refact_work_core::CoordinationSettings;

use crate::agents::execution::ExecutionState;
use crate::agents::types::BgAgentStatus;
use crate::call_validation::ChatMessage;

/// Why a run is not working right now, when that is deliberate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitingKind {
    Parked,
    Queued,
    Suspending,
    AwaitingApproval,
    /// The run's goal is parked on a typed subject and will wake when it
    /// settles. Declared waiting, never a stall.
    GoalWait,
}

/// The content evidence a no-material-progress verdict rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoProgressSignal {
    RepeatedToolCall { repeats: u32 },
    RepeatedOutput { repeats: u32 },
    UnchangedEvidence { turns: u32 },
}

/// What the run is observably doing. None of these is ever flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkingSignal {
    ToolCallInFlight,
    Streaming,
    RecentActivity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityClass {
    IntentionallyWaiting(WaitingKind),
    ProviderFailure(UserErrorCategory),
    NoMaterialProgress(NoProgressSignal),
    Working(WorkingSignal),
    NoObservedActivity { silence_window: Duration },
}

impl ActivityClass {
    /// A stable, inspectable name per classification.
    pub fn reason(&self) -> &'static str {
        match self {
            ActivityClass::IntentionallyWaiting(WaitingKind::Parked) => "waiting_parked",
            ActivityClass::IntentionallyWaiting(WaitingKind::Queued) => "waiting_queued",
            ActivityClass::IntentionallyWaiting(WaitingKind::Suspending) => "waiting_suspending",
            ActivityClass::IntentionallyWaiting(WaitingKind::AwaitingApproval) => {
                "waiting_approval"
            }
            ActivityClass::IntentionallyWaiting(WaitingKind::GoalWait) => "waiting_goal_subject",
            ActivityClass::ProviderFailure(_) => "provider_failure",
            ActivityClass::NoMaterialProgress(NoProgressSignal::RepeatedToolCall { .. }) => {
                "no_progress_repeated_tool_call"
            }
            ActivityClass::NoMaterialProgress(NoProgressSignal::RepeatedOutput { .. }) => {
                "no_progress_repeated_output"
            }
            ActivityClass::NoMaterialProgress(NoProgressSignal::UnchangedEvidence { .. }) => {
                "no_progress_unchanged_evidence"
            }
            ActivityClass::Working(WorkingSignal::ToolCallInFlight) => {
                "working_tool_call_in_flight"
            }
            ActivityClass::Working(WorkingSignal::Streaming) => "working_streaming",
            ActivityClass::Working(WorkingSignal::RecentActivity) => "working_recent_activity",
            ActivityClass::NoObservedActivity { .. } => "no_observed_activity",
        }
    }

    /// True only where content evidence says the run is repeating itself. A
    /// silence window alone can never make this true.
    pub fn is_no_progress_verdict(&self) -> bool {
        matches!(self, ActivityClass::NoMaterialProgress(_))
    }

    /// The line surfaced on the agent record, or `None` when there is nothing
    /// worth reporting.
    pub fn progress_note(&self) -> Option<String> {
        match self {
            ActivityClass::IntentionallyWaiting(_) | ActivityClass::Working(_) => None,
            ActivityClass::ProviderFailure(category) => Some(format!(
                "⚠ provider problem: {} — waiting on the provider, not stalled",
                user_error_info(*category).title
            )),
            ActivityClass::NoMaterialProgress(NoProgressSignal::RepeatedToolCall { repeats }) => {
                Some(format!(
                    "⚠ no material progress: the same tool call and result repeated {repeats} times"
                ))
            }
            ActivityClass::NoMaterialProgress(NoProgressSignal::RepeatedOutput { repeats }) => {
                Some(format!(
                    "⚠ no material progress: the same reply repeated {repeats} times"
                ))
            }
            ActivityClass::NoMaterialProgress(NoProgressSignal::UnchangedEvidence { turns }) => {
                Some(format!(
                    "⚠ no material progress: nothing changed across {turns} turns"
                ))
            }
            ActivityClass::NoObservedActivity { silence_window } => Some(format!(
                "⚠ no activity for {}+ minutes; no evidence of repeated work, so this is not a stall verdict",
                silence_window.as_secs() / 60
            )),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProgressThresholds {
    pub repeat_threshold: u32,
    pub unchanged_turns: u32,
    pub window_turns: usize,
    pub silence_window: Duration,
}

impl ProgressThresholds {
    pub fn from_settings(settings: &CoordinationSettings, silence_window: Duration) -> Self {
        Self {
            repeat_threshold: settings.no_progress_repeat_threshold as u32,
            unchanged_turns: settings.no_progress_unchanged_turns as u32,
            window_turns: settings.no_progress_window_turns,
            silence_window,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallEvidence {
    pub name: String,
    pub arguments: String,
    /// `None` while the call is still running: an unfinished call is never
    /// counted as a repeat.
    pub result: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnEvidence {
    pub tool_calls: Vec<ToolCallEvidence>,
    pub output: Option<String>,
}

/// One observation of a live run. Terminal records are not classified.
#[derive(Debug, Clone)]
pub struct RunActivity {
    pub execution: Option<ExecutionState>,
    pub status: BgAgentStatus,
    pub session_state: SessionState,
    pub session_error: Option<String>,
    pub tool_call_in_flight: bool,
    /// The run's goal is parked on a typed wait subject.
    pub goal_waiting: bool,
    pub quiet_for: Duration,
    /// Recent turns, oldest first.
    pub turns: Vec<TurnEvidence>,
    /// Turns completed since the run's material output last changed. `None`
    /// when nothing material has been observed, which yields no verdict.
    pub turns_since_evidence_changed: Option<u32>,
}

impl Default for RunActivity {
    fn default() -> Self {
        Self {
            execution: None,
            status: BgAgentStatus::Running,
            session_state: SessionState::Idle,
            session_error: None,
            tool_call_in_flight: false,
            goal_waiting: false,
            quiet_for: Duration::ZERO,
            turns: Vec::new(),
            turns_since_evidence_changed: None,
        }
    }
}

pub fn classify(activity: &RunActivity, thresholds: ProgressThresholds) -> ActivityClass {
    if let Some(kind) = waiting_kind(activity) {
        return ActivityClass::IntentionallyWaiting(kind);
    }
    if let Some(category) = provider_failure(activity) {
        return ActivityClass::ProviderFailure(category);
    }
    if let Some(signal) = no_material_progress(activity, thresholds) {
        return ActivityClass::NoMaterialProgress(signal);
    }
    if activity.tool_call_in_flight {
        return ActivityClass::Working(WorkingSignal::ToolCallInFlight);
    }
    if activity.quiet_for < thresholds.silence_window {
        return match activity.session_state {
            SessionState::Generating => ActivityClass::Working(WorkingSignal::Streaming),
            _ => ActivityClass::Working(WorkingSignal::RecentActivity),
        };
    }
    ActivityClass::NoObservedActivity {
        silence_window: thresholds.silence_window,
    }
}

fn waiting_kind(activity: &RunActivity) -> Option<WaitingKind> {
    if activity.goal_waiting {
        return Some(WaitingKind::GoalWait);
    }
    match activity.status {
        BgAgentStatus::WaitingForApproval => return Some(WaitingKind::AwaitingApproval),
        BgAgentStatus::Queued => return Some(WaitingKind::Queued),
        _ => {}
    }
    match activity.execution? {
        ExecutionState::Parked => Some(WaitingKind::Parked),
        ExecutionState::Acquiring => Some(WaitingKind::Queued),
        ExecutionState::Suspending => Some(WaitingKind::Suspending),
        _ => None,
    }
}

fn provider_failure(activity: &RunActivity) -> Option<UserErrorCategory> {
    if activity.session_state != SessionState::Error {
        return None;
    }
    let category = classify_user_error(activity.session_error.as_deref()?);
    provider_side(category).then_some(category)
}

fn provider_side(category: UserErrorCategory) -> bool {
    matches!(
        category,
        UserErrorCategory::ProviderTransient
            | UserErrorCategory::ProviderRateLimit
            | UserErrorCategory::NetworkFailure
            | UserErrorCategory::StreamCorrupted
            | UserErrorCategory::ModelUnavailable
            | UserErrorCategory::AuthenticationFailed
            | UserErrorCategory::BillingQuota
    )
}

fn no_material_progress(
    activity: &RunActivity,
    thresholds: ProgressThresholds,
) -> Option<NoProgressSignal> {
    let start = activity.turns.len().saturating_sub(thresholds.window_turns);
    let turns = &activity.turns[start..];
    if let Some(repeats) = repeated_tool_call(turns, thresholds.repeat_threshold) {
        return Some(NoProgressSignal::RepeatedToolCall { repeats });
    }
    if let Some(repeats) = repeated_output(turns, thresholds.repeat_threshold) {
        return Some(NoProgressSignal::RepeatedOutput { repeats });
    }
    let turns_unchanged = activity.turns_since_evidence_changed?;
    (generating(activity.session_state) && turns_unchanged >= thresholds.unchanged_turns).then_some(
        NoProgressSignal::UnchangedEvidence {
            turns: turns_unchanged,
        },
    )
}

fn generating(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Generating | SessionState::ExecutingTools
    )
}

fn repeated_tool_call(turns: &[TurnEvidence], threshold: u32) -> Option<u32> {
    let mut counts: BTreeMap<(&str, &str, &str), u32> = BTreeMap::new();
    for call in turns.iter().flat_map(|turn| turn.tool_calls.iter()) {
        let Some(result) = call.result.as_deref() else {
            continue;
        };
        *counts
            .entry((call.name.as_str(), call.arguments.as_str(), result))
            .or_default() += 1;
    }
    counts
        .into_values()
        .max()
        .filter(|repeats| *repeats >= threshold)
}

fn repeated_output(turns: &[TurnEvidence], threshold: u32) -> Option<u32> {
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for turn in turns.iter().filter(|turn| turn.tool_calls.is_empty()) {
        let output = turn.output.as_deref().unwrap_or_default().trim();
        if output.is_empty() {
            continue;
        }
        *counts.entry(output).or_default() += 1;
    }
    counts
        .into_values()
        .max()
        .filter(|repeats| *repeats >= threshold)
}

/// Rebuild recent turns from a child transcript. A turn is one assistant
/// message plus the results of the tool calls it issued.
pub fn turns_from_messages(messages: &[ChatMessage], window_turns: usize) -> Vec<TurnEvidence> {
    let start = window_start(messages, window_turns);
    let mut turns: Vec<TurnEvidence> = Vec::new();
    let mut pending: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for message in &messages[start..] {
        match message.role.as_str() {
            "assistant" => {
                let text = message.content.content_text_only();
                let mut turn = TurnEvidence {
                    tool_calls: Vec::new(),
                    output: (!text.trim().is_empty())
                        .then(|| crate::file_activity::digest_of(&text)),
                };
                for call in message.tool_calls.iter().flatten() {
                    pending.insert(call.id.clone(), (turns.len(), turn.tool_calls.len()));
                    turn.tool_calls.push(ToolCallEvidence {
                        name: call.function.name.clone(),
                        arguments: crate::file_activity::digest_of(&call.function.arguments),
                        result: None,
                    });
                }
                turns.push(turn);
            }
            "tool" => {
                if let Some((turn_index, call_index)) = pending.remove(&message.tool_call_id) {
                    turns[turn_index].tool_calls[call_index].result = Some(
                        crate::file_activity::digest_of(&message.content.content_text_only()),
                    );
                }
            }
            _ => {}
        }
    }
    turns
}

pub fn turn_count(messages: &[ChatMessage]) -> u32 {
    messages
        .iter()
        .filter(|message| message.role == "assistant")
        .count() as u32
}

fn window_start(messages: &[ChatMessage], window_turns: usize) -> usize {
    let mut seen = 0;
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role == "assistant" {
            seen += 1;
            if seen > window_turns {
                return index + 1;
            }
        }
    }
    0
}

struct EvidenceMark {
    digest: String,
    turns_at_change: u32,
}

static EVIDENCE_JOURNAL: Mutex<BTreeMap<String, EvidenceMark>> = Mutex::new(BTreeMap::new());

/// Record what a run has materially produced and report how many turns have
/// passed since that last changed. Without a digest there is no evidence, and
/// the caller gets `None` rather than a guess.
pub fn observe_evidence(agent_id: &str, digest: Option<String>, turns: u32) -> Option<u32> {
    let digest = digest?;
    let mut journal = EVIDENCE_JOURNAL.lock().ok()?;
    let mark = journal
        .entry(agent_id.to_string())
        .or_insert_with(|| EvidenceMark {
            digest: digest.clone(),
            turns_at_change: turns,
        });
    if mark.digest != digest {
        mark.digest = digest;
        mark.turns_at_change = turns;
    }
    Some(turns.saturating_sub(mark.turns_at_change))
}

/// Drop journal entries for runs that no longer exist, so the journal is
/// bounded by the live agents rather than by history.
pub fn retain_agents(live: &BTreeSet<String>) {
    if let Ok(mut journal) = EVIDENCE_JOURNAL.lock() {
        journal.retain(|agent_id, _| live.contains(agent_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

    fn thresholds() -> ProgressThresholds {
        ProgressThresholds {
            repeat_threshold: 3,
            unchanged_turns: 4,
            window_turns: 24,
            silence_window: Duration::from_secs(1_200),
        }
    }

    fn tool_turn(name: &str, arguments: &str, result: Option<&str>) -> TurnEvidence {
        TurnEvidence {
            tool_calls: vec![ToolCallEvidence {
                name: name.to_string(),
                arguments: arguments.to_string(),
                result: result.map(str::to_string),
            }],
            output: None,
        }
    }

    fn text_turn(output: &str) -> TurnEvidence {
        TurnEvidence {
            tool_calls: Vec::new(),
            output: Some(output.to_string()),
        }
    }

    fn quiet(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    /// Parked, queued, suspending and awaiting-approval runs are waiting by
    /// design. This is the invariant card G-1 established; it holds no matter
    /// how loud the other evidence is.
    #[test]
    fn waiting_by_design_is_never_flagged() {
        let looping = vec![
            tool_turn("cat", "{}", Some("same")),
            tool_turn("cat", "{}", Some("same")),
            tool_turn("cat", "{}", Some("same")),
        ];
        let cases = [
            (
                Some(ExecutionState::Parked),
                BgAgentStatus::Running,
                WaitingKind::Parked,
            ),
            (
                Some(ExecutionState::Acquiring),
                BgAgentStatus::Running,
                WaitingKind::Queued,
            ),
            (
                Some(ExecutionState::Suspending),
                BgAgentStatus::Running,
                WaitingKind::Suspending,
            ),
            (None, BgAgentStatus::Queued, WaitingKind::Queued),
            (
                None,
                BgAgentStatus::WaitingForApproval,
                WaitingKind::AwaitingApproval,
            ),
        ];
        for (execution, status, expected) in cases {
            let activity = RunActivity {
                execution,
                status,
                quiet_for: quiet(9_000),
                turns: looping.clone(),
                turns_since_evidence_changed: Some(99),
                session_state: SessionState::Generating,
                ..Default::default()
            };
            let class = classify(&activity, thresholds());
            assert_eq!(
                class,
                ActivityClass::IntentionallyWaiting(expected),
                "{execution:?}/{status:?}"
            );
            assert!(!class.is_no_progress_verdict());
            assert!(class.progress_note().is_none());
        }
    }

    /// A goal parked on a named subject is waiting on purpose. Even a run that
    /// otherwise looks like a loop must not be called stalled while it waits.
    #[test]
    fn a_goal_waiting_on_a_subject_is_intentional_waiting_not_a_stall() {
        let activity = RunActivity {
            goal_waiting: true,
            session_state: SessionState::Generating,
            quiet_for: quiet(9_000),
            turns: vec![
                tool_turn("cat", "{}", Some("same")),
                tool_turn("cat", "{}", Some("same")),
                tool_turn("cat", "{}", Some("same")),
            ],
            turns_since_evidence_changed: Some(99),
            ..Default::default()
        };

        let class = classify(&activity, thresholds());

        assert_eq!(
            class,
            ActivityClass::IntentionallyWaiting(WaitingKind::GoalWait)
        );
        assert_eq!(class.reason(), "waiting_goal_subject");
        assert!(!class.is_no_progress_verdict());
        assert!(class.progress_note().is_none());
    }

    /// Negative control: the same evidence without a declared wait is a stall.
    #[test]
    fn the_same_run_without_a_declared_wait_is_still_flagged() {
        let activity = RunActivity {
            goal_waiting: false,
            session_state: SessionState::Generating,
            turns: vec![
                tool_turn("cat", "{}", Some("same")),
                tool_turn("cat", "{}", Some("same")),
                tool_turn("cat", "{}", Some("same")),
            ],
            ..Default::default()
        };

        assert!(classify(&activity, thresholds()).is_no_progress_verdict());
    }

    /// A provider that will not answer is a provider problem. Calling the run
    /// stalled would point at the wrong thing.
    #[test]
    fn provider_transport_failure_is_not_an_agent_stall() {
        for error in [
            "HTTP 503 upstream service unavailable",
            "HTTP 429 rate limit exceeded",
            "HTTP 401 invalid api key",
        ] {
            let activity = RunActivity {
                session_state: SessionState::Error,
                session_error: Some(error.to_string()),
                quiet_for: quiet(9_000),
                ..Default::default()
            };
            let class = classify(&activity, thresholds());
            assert_eq!(class.reason(), "provider_failure", "{error}");
            assert!(!class.is_no_progress_verdict(), "{error}");
            let note = class.progress_note().unwrap();
            assert!(note.contains("provider"), "{note}");
            assert!(!note.contains("no material progress"), "{note}");
        }
    }

    /// A run-side error is not a provider failure and must not be reported as
    /// one; it falls through to the activity-based classes.
    #[test]
    fn a_run_side_error_is_not_reported_as_a_provider_failure() {
        let activity = RunActivity {
            session_state: SessionState::Error,
            session_error: Some("HTTP 400 invalid request: bad tool schema".to_string()),
            quiet_for: quiet(9_000),
            ..Default::default()
        };
        assert_eq!(
            classify(&activity, thresholds()).reason(),
            "no_observed_activity"
        );
    }

    #[test]
    fn repeated_identical_tool_call_and_result_is_no_material_progress() {
        let activity = RunActivity {
            session_state: SessionState::ExecutingTools,
            tool_call_in_flight: true,
            turns: vec![
                tool_turn("cargo_check", "{\"crate\":\"lsp\"}", Some("117 errors")),
                tool_turn("cargo_check", "{\"crate\":\"lsp\"}", Some("117 errors")),
                tool_turn("cargo_check", "{\"crate\":\"lsp\"}", Some("117 errors")),
            ],
            ..Default::default()
        };
        let class = classify(&activity, thresholds());
        assert_eq!(
            class,
            ActivityClass::NoMaterialProgress(NoProgressSignal::RepeatedToolCall { repeats: 3 })
        );
        assert!(class.is_no_progress_verdict());
        assert!(class
            .progress_note()
            .unwrap()
            .contains("no material progress"));
    }

    /// Below the threshold, and while a repeat is still unfinished, there is no
    /// verdict: the same call twice is ordinary work.
    #[test]
    fn repeats_below_the_threshold_or_still_running_are_not_flagged() {
        let twice = RunActivity {
            turns: vec![
                tool_turn("grep", "{}", Some("no match")),
                tool_turn("grep", "{}", Some("no match")),
            ],
            ..Default::default()
        };
        assert!(!classify(&twice, thresholds()).is_no_progress_verdict());

        let unfinished = RunActivity {
            turns: vec![
                tool_turn("grep", "{}", Some("no match")),
                tool_turn("grep", "{}", Some("no match")),
                tool_turn("grep", "{}", None),
            ],
            ..Default::default()
        };
        assert!(!classify(&unfinished, thresholds()).is_no_progress_verdict());
    }

    #[test]
    fn repeated_identical_reply_is_no_material_progress() {
        let activity = RunActivity {
            session_state: SessionState::Generating,
            turns: vec![
                text_turn("Let me check that again."),
                text_turn("Let me check that again."),
                text_turn("Let me check that again."),
            ],
            ..Default::default()
        };
        assert_eq!(
            classify(&activity, thresholds()),
            ActivityClass::NoMaterialProgress(NoProgressSignal::RepeatedOutput { repeats: 3 })
        );
    }

    #[test]
    fn unchanged_evidence_across_turns_while_generating_is_no_material_progress() {
        let activity = RunActivity {
            session_state: SessionState::Generating,
            turns_since_evidence_changed: Some(4),
            turns: vec![
                tool_turn("read", "{\"a\":1}", Some("a")),
                tool_turn("read", "{\"b\":2}", Some("b")),
                tool_turn("read", "{\"c\":3}", Some("c")),
                tool_turn("read", "{\"d\":4}", Some("d")),
            ],
            ..Default::default()
        };
        assert_eq!(
            classify(&activity, thresholds()),
            ActivityClass::NoMaterialProgress(NoProgressSignal::UnchangedEvidence { turns: 4 })
        );
    }

    /// Unobservable evidence is not evidence of absence.
    #[test]
    fn absent_evidence_yields_no_verdict() {
        let activity = RunActivity {
            session_state: SessionState::Generating,
            turns_since_evidence_changed: None,
            quiet_for: quiet(10),
            ..Default::default()
        };
        assert_eq!(
            classify(&activity, thresholds()).reason(),
            "working_streaming"
        );
    }

    /// Negative control: a run that changes something every turn is slow, not
    /// stuck, however long it takes.
    #[test]
    fn a_slow_run_that_changes_files_every_turn_is_never_flagged() {
        for turn in 1..=40u32 {
            let activity = RunActivity {
                session_state: SessionState::Generating,
                turns_since_evidence_changed: Some(0),
                quiet_for: quiet(u64::from(turn) * 60),
                turns: (0..turn)
                    .map(|index| {
                        tool_turn("patch", &format!("{{\"file\":{index}}}"), Some("written"))
                    })
                    .collect(),
                ..Default::default()
            };
            let class = classify(&activity, thresholds());
            assert!(!class.is_no_progress_verdict(), "turn {turn}: {class:?}");
        }
    }

    /// Negative control: one long operation is one operation. Nothing here may
    /// turn a slow build into a stall.
    #[test]
    fn one_long_tool_call_is_never_flagged() {
        for minutes in [1u64, 20, 60, 240] {
            let activity = RunActivity {
                session_state: SessionState::ExecutingTools,
                tool_call_in_flight: true,
                quiet_for: quiet(minutes * 60),
                turns: vec![tool_turn("shell", "{\"cmd\":\"cargo build\"}", None)],
                ..Default::default()
            };
            let class = classify(&activity, thresholds());
            assert_eq!(
                class,
                ActivityClass::Working(WorkingSignal::ToolCallInFlight),
                "{minutes} minutes"
            );
            assert!(class.progress_note().is_none());
        }
    }

    /// Silence is an observation, never a verdict. This is the rule that keeps
    /// a wall clock from deciding on its own that a run is stuck.
    #[test]
    fn silence_alone_never_yields_a_no_progress_verdict() {
        for minutes in [0u64, 19, 20, 120, 1_440] {
            let activity = RunActivity {
                quiet_for: quiet(minutes * 60),
                ..Default::default()
            };
            let class = classify(&activity, thresholds());
            assert!(!class.is_no_progress_verdict(), "{minutes} minutes");
            if minutes >= 20 {
                assert_eq!(class.reason(), "no_observed_activity", "{minutes} minutes");
                let note = class.progress_note().unwrap();
                assert!(note.contains("20+ minutes"), "{note}");
                assert!(note.contains("not a stall verdict"), "{note}");
            } else {
                assert!(class.progress_note().is_none(), "{minutes} minutes");
            }
        }
    }

    #[test]
    fn every_class_carries_a_distinct_reason() {
        let classes = [
            ActivityClass::IntentionallyWaiting(WaitingKind::Parked),
            ActivityClass::IntentionallyWaiting(WaitingKind::Queued),
            ActivityClass::IntentionallyWaiting(WaitingKind::Suspending),
            ActivityClass::IntentionallyWaiting(WaitingKind::AwaitingApproval),
            ActivityClass::IntentionallyWaiting(WaitingKind::GoalWait),
            ActivityClass::ProviderFailure(UserErrorCategory::NetworkFailure),
            ActivityClass::NoMaterialProgress(NoProgressSignal::RepeatedToolCall { repeats: 3 }),
            ActivityClass::NoMaterialProgress(NoProgressSignal::RepeatedOutput { repeats: 3 }),
            ActivityClass::NoMaterialProgress(NoProgressSignal::UnchangedEvidence { turns: 8 }),
            ActivityClass::Working(WorkingSignal::ToolCallInFlight),
            ActivityClass::Working(WorkingSignal::Streaming),
            ActivityClass::Working(WorkingSignal::RecentActivity),
            ActivityClass::NoObservedActivity {
                silence_window: Duration::from_secs(1_200),
            },
        ];
        let reasons: BTreeSet<&str> = classes.iter().map(ActivityClass::reason).collect();
        assert_eq!(reasons.len(), classes.len());
    }

    #[test]
    fn only_the_last_window_of_turns_is_examined() {
        let mut turns = vec![
            tool_turn("old", "{}", Some("same")),
            tool_turn("old", "{}", Some("same")),
            tool_turn("old", "{}", Some("same")),
        ];
        turns.extend(
            (0..3).map(|index| {
                tool_turn("fresh", &format!("{{\"n\":{index}}}"), Some("progressing"))
            }),
        );
        let activity = RunActivity {
            turns,
            ..Default::default()
        };
        let narrow = ProgressThresholds {
            window_turns: 3,
            ..thresholds()
        };
        assert!(!classify(&activity, narrow).is_no_progress_verdict());
        assert!(classify(&activity, thresholds()).is_no_progress_verdict());
    }

    #[test]
    fn thresholds_follow_coordination_settings() {
        let mut settings = CoordinationSettings::default();
        settings.no_progress_repeat_threshold = 7;
        settings.no_progress_unchanged_turns = 9;
        settings.no_progress_window_turns = 11;
        let thresholds = ProgressThresholds::from_settings(&settings, Duration::from_secs(1_200));
        assert_eq!(thresholds.repeat_threshold, 7);
        assert_eq!(thresholds.unchanged_turns, 9);
        assert_eq!(thresholds.window_turns, 11);
    }

    fn assistant(text: &str, calls: Vec<(&str, &str, &str)>) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            tool_calls: (!calls.is_empty()).then(|| {
                calls
                    .into_iter()
                    .map(|(id, name, arguments)| ChatToolCall {
                        id: id.to_string(),
                        index: None,
                        function: ChatToolFunction {
                            arguments: arguments.to_string(),
                            name: name.to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                        started_at_ms: None,
                        completed_at_ms: None,
                    })
                    .collect()
            }),
            ..Default::default()
        }
    }

    fn tool_result(id: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            tool_call_id: id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn turns_pair_tool_results_with_the_call_that_issued_them() {
        let messages = vec![
            assistant("checking", vec![("call-1", "grep", "{\"q\":1}")]),
            tool_result("call-1", "no match"),
            assistant("checking", vec![("call-2", "grep", "{\"q\":1}")]),
            tool_result("call-2", "no match"),
        ];
        let turns = turns_from_messages(&messages, 24);
        assert_eq!(turns.len(), 2);
        assert_eq!(turn_count(&messages), 2);
        assert_eq!(turns[0].tool_calls[0].name, "grep");
        assert_eq!(turns[0].tool_calls, turns[1].tool_calls);
        assert!(turns[0].tool_calls[0].result.is_some());
        assert_eq!(turns[0].output, turns[1].output);
    }

    #[test]
    fn turns_are_limited_to_the_requested_window() {
        let mut messages = Vec::new();
        for index in 0..10 {
            let id = format!("call-{index}");
            messages.push(assistant("step", vec![(id.as_str(), "grep", "{}")]));
            messages.push(tool_result(&id, "out"));
        }
        assert_eq!(turns_from_messages(&messages, 3).len(), 3);
        assert_eq!(turn_count(&messages), 10);
    }

    #[test]
    fn evidence_journal_counts_turns_since_the_last_material_change() {
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        assert_eq!(
            observe_evidence(&agent, Some("digest-a".to_string()), 4),
            Some(0)
        );
        assert_eq!(
            observe_evidence(&agent, Some("digest-a".to_string()), 9),
            Some(5)
        );
        assert_eq!(
            observe_evidence(&agent, Some("digest-b".to_string()), 10),
            Some(0)
        );
        assert_eq!(observe_evidence(&agent, None, 20), None);
        retain_agents(&BTreeSet::new());
        assert_eq!(
            observe_evidence(&agent, Some("digest-b".to_string()), 30),
            Some(0)
        );
        retain_agents(&BTreeSet::new());
    }
}
