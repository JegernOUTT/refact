use chrono::{DateTime, Duration, Utc};

use crate::state::{IntentBudgetState, SpeechRotationState};
use crate::types::BuddySpeechItem;
use crate::voice_service::SpeechIntent;

pub struct SpeechBudget {
    pub per_hour: u32,
    pub per_day: u32,
    pub priority: u8,
}

pub fn budget_for(intent: SpeechIntent) -> SpeechBudget {
    match intent {
        SpeechIntent::Humor => SpeechBudget {
            per_hour: 5,
            per_day: 60,
            priority: 2,
        },
        SpeechIntent::Suggestion => SpeechBudget {
            per_hour: 3,
            per_day: 30,
            priority: 6,
        },
        SpeechIntent::Insight => SpeechBudget {
            per_hour: 2,
            per_day: 20,
            priority: 5,
        },
        SpeechIntent::ErrorAlert => SpeechBudget {
            per_hour: 2,
            per_day: 20,
            priority: 9,
        },
        SpeechIntent::Greeting => SpeechBudget {
            per_hour: u32::MAX,
            per_day: 1,
            priority: 1,
        },
        SpeechIntent::Win => SpeechBudget {
            per_hour: 1,
            per_day: 8,
            priority: 4,
        },
        SpeechIntent::Tour => SpeechBudget {
            per_hour: 1,
            per_day: 1,
            priority: 3,
        },
        SpeechIntent::Milestone => SpeechBudget {
            per_hour: 1,
            per_day: 8,
            priority: 4,
        },
        SpeechIntent::MemoryPulseCommentary => SpeechBudget {
            per_hour: 1,
            per_day: 6,
            priority: 3,
        },
        SpeechIntent::QuestAccept | SpeechIntent::QuestComplete => SpeechBudget {
            per_hour: 1,
            per_day: 4,
            priority: 4,
        },
        SpeechIntent::ChatReaction => SpeechBudget {
            per_hour: 4,
            per_day: 30,
            priority: 2,
        },
    }
}

pub fn pick_speech_intent(
    candidates: &[(SpeechIntent, BuddySpeechItem)],
    rotation: &SpeechRotationState,
    now: DateTime<Utc>,
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter_map(|(idx, (intent, _))| {
            let budget = budget_for(*intent);
            let state = rotation.by_intent.get(intent_key(*intent));
            let hour_count = effective_hour_count(state, now);
            let day_count = effective_day_count(state, now);
            if hour_count >= budget.per_hour || day_count >= budget.per_day {
                return None;
            }
            let remaining_budget = budget.per_day.saturating_sub(day_count).max(1) as u128;
            let hours_since_last = state
                .and_then(|state| state.last_emitted_at)
                .map(|last| now.signed_duration_since(last).num_seconds().max(0) as u128 / 3600)
                .unwrap_or(24)
                .max(1);
            let score = remaining_budget * hours_since_last * u128::from(budget.priority);
            Some((idx, score, budget.priority))
        })
        .max_by(|left, right| {
            left.1
                .cmp(&right.1)
                .then_with(|| left.2.cmp(&right.2))
                .then_with(|| right.0.cmp(&left.0))
        })
        .map(|(idx, _, _)| idx)
}

pub fn record_emission(
    rotation: &mut SpeechRotationState,
    intent: SpeechIntent,
    now: DateTime<Utc>,
) {
    let state = rotation
        .by_intent
        .entry(intent_key(intent).to_string())
        .or_default();
    reset_windows(state, now);
    state.last_emitted_at = Some(now);
    state.hour_count = state.hour_count.saturating_add(1);
    state.day_count = state.day_count.saturating_add(1);
}

pub fn intent_key(intent: SpeechIntent) -> &'static str {
    match intent {
        SpeechIntent::Humor => "humor",
        SpeechIntent::Suggestion => "suggestion",
        SpeechIntent::Insight => "insight",
        SpeechIntent::Win => "win",
        SpeechIntent::ErrorAlert => "error_alert",
        SpeechIntent::Greeting => "greeting",
        SpeechIntent::Tour => "tour",
        SpeechIntent::Milestone => "milestone",
        SpeechIntent::MemoryPulseCommentary => "memory_pulse_commentary",
        SpeechIntent::QuestAccept => "quest_accept",
        SpeechIntent::QuestComplete => "quest_complete",
        SpeechIntent::ChatReaction => "chat_reaction",
    }
}
pub fn parse_intent_key(token: &str) -> Option<SpeechIntent> {
    match token {
        "humor" => Some(SpeechIntent::Humor),
        "suggestion" => Some(SpeechIntent::Suggestion),
        "insight" => Some(SpeechIntent::Insight),
        "win" => Some(SpeechIntent::Win),
        "error_alert" => Some(SpeechIntent::ErrorAlert),
        "greeting" => Some(SpeechIntent::Greeting),
        "tour" => Some(SpeechIntent::Tour),
        "milestone" => Some(SpeechIntent::Milestone),
        "memory_pulse_commentary" => Some(SpeechIntent::MemoryPulseCommentary),
        "quest_accept" => Some(SpeechIntent::QuestAccept),
        "quest_complete" => Some(SpeechIntent::QuestComplete),
        "chat_reaction" => Some(SpeechIntent::ChatReaction),
        _ => None,
    }
}

pub const ALL_INTENT_KEYS: &[&str] = &[
    "humor",
    "suggestion",
    "insight",
    "win",
    "error_alert",
    "greeting",
    "tour",
    "milestone",
    "memory_pulse_commentary",
    "quest_accept",
    "quest_complete",
    "chat_reaction",
];

pub fn hour_in_quiet_window(hour: u32, start: u8, end: u8) -> bool {
    let start = u32::from(start) % 24;
    let end = u32::from(end) % 24;
    let hour = hour % 24;
    if start == end {
        return false;
    }
    if start < end {
        hour >= start && hour < end
    } else {
        hour >= start || hour < end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechGateDecision {
    pub allowed: bool,
    pub reason: &'static str,
}

impl SpeechGateDecision {
    fn allow(reason: &'static str) -> Self {
        Self {
            allowed: true,
            reason,
        }
    }

    fn drop(reason: &'static str) -> Self {
        Self {
            allowed: false,
            reason,
        }
    }
}

pub fn gate_speech(
    settings: &crate::settings::BuddySettings,
    rotation: &SpeechRotationState,
    intent: Option<SpeechIntent>,
    chat_id: Option<&str>,
    local_hour: u32,
    auto_quiet_window: Option<(u8, u8)>,
    now: DateTime<Utc>,
) -> SpeechGateDecision {
    if let Some(chat_id) = chat_id {
        if settings.muted_chat_ids.iter().any(|id| id == chat_id) {
            return SpeechGateDecision::drop("chat_muted");
        }
    }
    if let Some(intent) = intent {
        let key = intent_key(intent);
        if settings.muted_intents.iter().any(|muted| muted == key) {
            return SpeechGateDecision::drop("intent_muted");
        }
    }
    let critical = matches!(intent, Some(SpeechIntent::ErrorAlert));
    if !critical {
        let quiet_window = match settings.quiet_hours_mode {
            crate::settings::QuietHoursMode::Off => None,
            crate::settings::QuietHoursMode::Fixed => {
                Some((settings.quiet_hours_start, settings.quiet_hours_end))
            }
            crate::settings::QuietHoursMode::Auto => auto_quiet_window,
        };
        if let Some((start, end)) = quiet_window {
            if hour_in_quiet_window(local_hour, start, end) {
                return SpeechGateDecision::drop("quiet_hours");
            }
        }
    }
    if let Some(intent) = intent {
        let budget = budget_for(intent);
        let state = rotation.by_intent.get(intent_key(intent));
        let hour_count = effective_hour_count(state, now);
        let day_count = effective_day_count(state, now);
        if hour_count >= budget.per_hour || day_count >= budget.per_day {
            return SpeechGateDecision::drop("intent_budget");
        }
    }
    SpeechGateDecision::allow("allowed")
}

pub fn gate_speech_user_initiated(
    settings: &crate::settings::BuddySettings,
    intent: Option<SpeechIntent>,
    chat_id: Option<&str>,
) -> SpeechGateDecision {
    if let Some(chat_id) = chat_id {
        if settings.muted_chat_ids.iter().any(|id| id == chat_id) {
            return SpeechGateDecision::drop("chat_muted");
        }
    }
    if let Some(intent) = intent {
        let key = intent_key(intent);
        if settings.muted_intents.iter().any(|muted| muted == key) {
            return SpeechGateDecision::drop("intent_muted");
        }
    }
    SpeechGateDecision::allow("user_initiated")
}

pub fn auto_quiet_window_from_actions(
    actions: &[crate::user_action::UserAction],
) -> Option<(u8, u8)> {
    if actions.len() < 10 {
        return None;
    }
    let mut hours = [0usize; 24];
    for action in actions {
        let local = action.ts().with_timezone(&chrono::Local);
        hours[chrono::Timelike::hour(&local) as usize] += 1;
    }
    let active: Vec<usize> = (0..24).filter(|&h| hours[h] > 0).collect();
    if active.is_empty() || active.len() >= 20 {
        return None;
    }
    let mut best_gap = 0usize;
    let mut best_last = active[0];
    let mut best_next = active[0];
    for (i, &hour) in active.iter().enumerate() {
        let next = active[(i + 1) % active.len()];
        let gap = (next + 24 - hour - 1) % 24;
        if gap > best_gap {
            best_gap = gap;
            best_last = hour;
            best_next = next;
        }
    }
    if best_gap < 3 {
        return None;
    }
    let quiet_start = ((best_last + 2) % 24) as u8;
    let quiet_end = best_next as u8;
    if quiet_start == quiet_end {
        return None;
    }
    Some((quiet_start, quiet_end))
}

fn effective_hour_count(state: Option<&IntentBudgetState>, now: DateTime<Utc>) -> u32 {
    state
        .filter(|state| window_active(state.hour_window_start, now, Duration::hours(1)))
        .map(|state| state.hour_count)
        .unwrap_or(0)
}

fn effective_day_count(state: Option<&IntentBudgetState>, now: DateTime<Utc>) -> u32 {
    state
        .filter(|state| window_active(state.day_window_start, now, Duration::hours(24)))
        .map(|state| state.day_count)
        .unwrap_or(0)
}

fn reset_windows(state: &mut IntentBudgetState, now: DateTime<Utc>) {
    if !window_active(state.hour_window_start, now, Duration::hours(1)) {
        state.hour_count = 0;
        state.hour_window_start = Some(now);
    }
    if !window_active(state.day_window_start, now, Duration::hours(24)) {
        state.day_count = 0;
        state.day_window_start = Some(now);
    }
}

fn window_active(
    window_start: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    duration: Duration,
) -> bool {
    window_start
        .map(|start| now.signed_duration_since(start) < duration)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn speech(id: &str) -> BuddySpeechItem {
        BuddySpeechItem {
            id: id.to_string(),
            text: id.to_string(),
            mood: "happy".to_string(),
            scope: "global".to_string(),
            persistent: false,
            ttl_seconds: 10,
            dedupe_key: None,
            speech_intent: None,
            created_at: now().to_rfc3339(),
            controls: vec![],
            chat_id: None,
        }
    }

    fn budget_state(
        intent: SpeechIntent,
        hour_count: u32,
        day_count: u32,
        last_emitted_at: Option<DateTime<Utc>>,
        clock: DateTime<Utc>,
    ) -> SpeechRotationState {
        let mut rotation = SpeechRotationState::default();
        rotation.by_intent.insert(
            intent_key(intent).to_string(),
            IntentBudgetState {
                last_emitted_at,
                hour_count,
                day_count,
                hour_window_start: Some(clock),
                day_window_start: Some(clock),
            },
        );
        rotation
    }

    #[test]
    fn pick_speech_intent_chooses_highest_score() {
        let clock = now();
        let candidates = vec![
            (SpeechIntent::Humor, speech("humor")),
            (SpeechIntent::ErrorAlert, speech("error")),
        ];

        let picked = pick_speech_intent(&candidates, &SpeechRotationState::default(), clock);

        assert_eq!(picked, Some(1));
    }

    #[test]
    fn pick_speech_intent_respects_per_hour_budget() {
        let clock = now();
        let candidates = vec![(SpeechIntent::Win, speech("win"))];
        let rotation = budget_state(SpeechIntent::Win, 1, 1, Some(clock), clock);

        let picked = pick_speech_intent(&candidates, &rotation, clock);

        assert_eq!(picked, None);
    }

    #[test]
    fn pick_speech_intent_respects_per_day_budget() {
        let clock = now();
        let candidates = vec![(SpeechIntent::Greeting, speech("greeting"))];
        let rotation = budget_state(SpeechIntent::Greeting, 0, 1, Some(clock), clock);

        let picked = pick_speech_intent(&candidates, &rotation, clock);

        assert_eq!(picked, None);
    }

    #[test]
    fn pick_speech_intent_returns_none_when_all_over_budget() {
        let clock = now();
        let candidates = vec![
            (SpeechIntent::Win, speech("win")),
            (SpeechIntent::Tour, speech("tour")),
        ];
        let mut rotation = budget_state(SpeechIntent::Win, 1, 1, Some(clock), clock);
        rotation.by_intent.insert(
            intent_key(SpeechIntent::Tour).to_string(),
            IntentBudgetState {
                last_emitted_at: Some(clock),
                hour_count: 1,
                day_count: 1,
                hour_window_start: Some(clock),
                day_window_start: Some(clock),
            },
        );

        let picked = pick_speech_intent(&candidates, &rotation, clock);

        assert_eq!(picked, None);
    }

    #[test]
    fn pick_speech_intent_stable_tiebreak() {
        let clock = now();
        let candidates = vec![
            (SpeechIntent::Win, speech("first")),
            (SpeechIntent::Milestone, speech("second")),
        ];

        let picked = pick_speech_intent(&candidates, &SpeechRotationState::default(), clock);

        assert_eq!(picked, Some(0));
    }

    #[test]
    fn record_emission_resets_hour_window_after_60_min() {
        let clock = now();
        let mut rotation = budget_state(
            SpeechIntent::Humor,
            5,
            5,
            Some(clock),
            clock - Duration::minutes(60),
        );

        record_emission(&mut rotation, SpeechIntent::Humor, clock);

        let state = rotation
            .by_intent
            .get(intent_key(SpeechIntent::Humor))
            .unwrap();
        assert_eq!(state.hour_count, 1);
        assert_eq!(state.day_count, 6);
        assert_eq!(state.hour_window_start, Some(clock));
    }
}
#[cfg(test)]
mod gate_tests {
    use super::*;
    use crate::settings::{BuddySettings, QuietHoursMode};

    fn clock() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn quiet_window_handles_wraparound() {
        assert!(hour_in_quiet_window(23, 22, 8));
        assert!(hour_in_quiet_window(3, 22, 8));
        assert!(!hour_in_quiet_window(12, 22, 8));
        assert!(hour_in_quiet_window(10, 9, 17));
        assert!(!hour_in_quiet_window(8, 9, 17));
        assert!(!hour_in_quiet_window(5, 6, 6));
    }

    #[test]
    fn gate_blocks_muted_chat_and_intent() {
        let mut settings = BuddySettings::default();
        settings.quiet_hours_mode = QuietHoursMode::Off;
        settings.muted_chat_ids.push("chat-9".to_string());
        settings.muted_intents.push("humor".to_string());
        let rotation = SpeechRotationState::default();

        let by_chat = gate_speech(
            &settings,
            &rotation,
            Some(SpeechIntent::Insight),
            Some("chat-9"),
            12,
            None,
            clock(),
        );
        assert!(!by_chat.allowed);
        assert_eq!(by_chat.reason, "chat_muted");

        let by_intent = gate_speech(
            &settings,
            &rotation,
            Some(SpeechIntent::Humor),
            None,
            12,
            None,
            clock(),
        );
        assert!(!by_intent.allowed);
        assert_eq!(by_intent.reason, "intent_muted");

        let ok = gate_speech(
            &settings,
            &rotation,
            Some(SpeechIntent::Insight),
            Some("chat-other"),
            12,
            None,
            clock(),
        );
        assert!(ok.allowed);
    }

    #[test]
    fn gate_quiet_hours_fixed_blocks_but_exempts_error_alert() {
        let mut settings = BuddySettings::default();
        settings.quiet_hours_mode = QuietHoursMode::Fixed;
        settings.quiet_hours_start = 22;
        settings.quiet_hours_end = 8;
        let rotation = SpeechRotationState::default();

        let humor = gate_speech(
            &settings,
            &rotation,
            Some(SpeechIntent::Humor),
            None,
            23,
            None,
            clock(),
        );
        assert!(!humor.allowed);
        assert_eq!(humor.reason, "quiet_hours");

        let alert = gate_speech(
            &settings,
            &rotation,
            Some(SpeechIntent::ErrorAlert),
            None,
            23,
            None,
            clock(),
        );
        assert!(alert.allowed);
    }

    #[test]
    fn gate_auto_mode_uses_derived_window_only() {
        let settings = BuddySettings::default();
        let rotation = SpeechRotationState::default();

        let without_data = gate_speech(
            &settings,
            &rotation,
            Some(SpeechIntent::Humor),
            None,
            3,
            None,
            clock(),
        );
        assert!(without_data.allowed);

        let with_window = gate_speech(
            &settings,
            &rotation,
            Some(SpeechIntent::Humor),
            None,
            3,
            Some((22, 8)),
            clock(),
        );
        assert!(!with_window.allowed);
        assert_eq!(with_window.reason, "quiet_hours");
    }

    #[test]
    fn gate_enforces_intent_budget() {
        let mut settings = BuddySettings::default();
        settings.quiet_hours_mode = QuietHoursMode::Off;
        let mut rotation = SpeechRotationState::default();
        rotation.by_intent.insert(
            intent_key(SpeechIntent::Win).to_string(),
            IntentBudgetState {
                last_emitted_at: Some(clock()),
                hour_count: 1,
                day_count: 1,
                hour_window_start: Some(clock()),
                day_window_start: Some(clock()),
            },
        );

        let decision = gate_speech(
            &settings,
            &rotation,
            Some(SpeechIntent::Win),
            None,
            12,
            None,
            clock(),
        );
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "intent_budget");
    }

    #[test]
    fn auto_quiet_window_derives_from_hour_histogram() {
        use crate::user_action::UserAction;
        let base = DateTime::parse_from_rfc3339("2026-05-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut actions = Vec::new();
        for i in 0..12 {
            let local_hour = 9 + (i % 6) as i64;
            let ts_local = chrono::Local::now()
                .date_naive()
                .and_hms_opt(local_hour as u32, 0, 0)
                .unwrap()
                .and_local_timezone(chrono::Local)
                .unwrap()
                .with_timezone(&Utc);
            actions.push(UserAction::FileOpened {
                path: format!("f{}", i),
                ts: ts_local,
            });
        }
        let window = auto_quiet_window_from_actions(&actions).unwrap();
        assert_eq!(window, (16, 9));
        assert!(auto_quiet_window_from_actions(&actions[..5]).is_none());
        let _ = base;
    }

    #[test]
    fn auto_quiet_window_handles_midnight_crossing_activity() {
        use crate::user_action::UserAction;
        let mut actions = Vec::new();
        for i in 0..15 {
            let local_hour = [22u32, 23, 0, 1, 2][i % 5];
            let ts = chrono::Local::now()
                .date_naive()
                .and_hms_opt(local_hour, 0, 0)
                .unwrap()
                .and_local_timezone(chrono::Local)
                .unwrap()
                .with_timezone(&Utc);
            actions.push(UserAction::FileOpened {
                path: format!("f{}", i),
                ts,
            });
        }
        let window = auto_quiet_window_from_actions(&actions).unwrap();
        assert_eq!(window, (4, 22));
        for hour in [22, 23, 0, 1, 2] {
            assert!(!hour_in_quiet_window(hour, window.0, window.1));
        }
        assert!(hour_in_quiet_window(12, window.0, window.1));
    }

    #[test]
    fn auto_quiet_window_edge_gaps_table() {
        use crate::user_action::UserAction;
        fn actions_at(hours: &[u32], min_len: usize) -> Vec<UserAction> {
            let mut actions = Vec::new();
            let mut i = 0usize;
            while actions.len() < min_len.max(10) {
                let hour = hours[i % hours.len()];
                let ts = chrono::Local::now()
                    .date_naive()
                    .and_hms_opt(hour, 0, 0)
                    .unwrap()
                    .and_local_timezone(chrono::Local)
                    .unwrap()
                    .with_timezone(&Utc);
                actions.push(UserAction::FileOpened {
                    path: format!("f{}", i),
                    ts,
                });
                i += 1;
            }
            actions
        }

        assert_eq!(
            auto_quiet_window_from_actions(&actions_at(&[10], 10)),
            Some((12, 10))
        );
        assert_eq!(
            auto_quiet_window_from_actions(&actions_at(&[10, 11], 10)),
            Some((13, 10))
        );
        assert_eq!(
            auto_quiet_window_from_actions(&actions_at(&[10, 12], 10)),
            Some((14, 10))
        );
        assert_eq!(
            auto_quiet_window_from_actions(&actions_at(&[10, 13], 10)),
            Some((15, 10))
        );

        let dense: Vec<u32> = (0..24u32).step_by(3).collect();
        assert_eq!(
            auto_quiet_window_from_actions(&actions_at(&dense, dense.len() * 2)),
            None
        );

        let sparse: Vec<u32> = (0..24u32).step_by(4).collect();
        assert_eq!(
            auto_quiet_window_from_actions(&actions_at(&sparse, sparse.len() * 2)),
            Some((2, 4))
        );
    }

    #[test]
    fn intent_keys_round_trip() {
        for key in ALL_INTENT_KEYS {
            let intent = parse_intent_key(key).unwrap();
            assert_eq!(intent_key(intent), *key);
        }
        assert!(parse_intent_key("bogus").is_none());
    }
}
