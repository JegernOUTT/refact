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
    }
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
