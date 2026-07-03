use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const VERDICT_RING_CAP: usize = 500;
pub const DECAY_WINDOW: usize = 10;
pub const DECAY_RATIO: f32 = 0.8;
pub const DECAY_COOLDOWN_MULTIPLIER: u64 = 4;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerdictOutcome {
    Accept,
    Dismiss,
    Never,
    Undo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyVerdict {
    pub rule_key: String,
    pub kind: String,
    pub action_kind: String,
    pub verdict: VerdictOutcome,
    pub at: DateTime<Utc>,
}

pub fn rule_key_of(cooldown_key: &str) -> String {
    cooldown_key
        .split(':')
        .take(2)
        .collect::<Vec<_>>()
        .join(":")
}

pub const LEGACY_MEMORY_BATCH_PREFIX: &str = "memory_ops:batch";
pub const MEMORY_BATCH_PREFIX: &str = "memory_ops_batch";

fn migrate_legacy_batch_key(key: &str) -> Option<String> {
    let rest = key.strip_prefix(LEGACY_MEMORY_BATCH_PREFIX)?;
    if rest.is_empty() {
        return Some(MEMORY_BATCH_PREFIX.to_string());
    }
    rest.strip_prefix(':')
        .map(|suffix| format!("{}:{}", MEMORY_BATCH_PREFIX, suffix))
}

pub fn migrate_legacy_batch_keys(state: &mut crate::types::BuddyState) -> bool {
    let mut changed = false;
    for opp in state.opportunities.iter_mut() {
        if let Some(migrated) = migrate_legacy_batch_key(&opp.cooldown_key) {
            opp.cooldown_key = migrated;
            changed = true;
        }
    }
    for entry in state.dismissed_history.iter_mut() {
        if let Some(migrated) = migrate_legacy_batch_key(&entry.cooldown_key) {
            entry.cooldown_key = migrated;
            changed = true;
        }
    }
    for verdict in state.verdicts.iter_mut() {
        if let Some(migrated) = migrate_legacy_batch_key(&verdict.rule_key) {
            verdict.rule_key = migrated;
            changed = true;
        }
    }
    let mut migrated_mutes: Vec<String> = Vec::new();
    state.muted_rules.retain(|rule| {
        if rule == LEGACY_MEMORY_BATCH_PREFIX {
            for class in crate::memory_lifecycle_model::MEMORY_BATCH_KEYS {
                migrated_mutes.push(format!("{}:{}", MEMORY_BATCH_PREFIX, class));
            }
            changed = true;
            false
        } else if let Some(migrated) = migrate_legacy_batch_key(rule) {
            migrated_mutes.push(migrated);
            changed = true;
            false
        } else {
            true
        }
    });
    for mute in migrated_mutes {
        if !state.muted_rules.iter().any(|rule| *rule == mute) {
            state.muted_rules.push(mute);
        }
    }
    changed
}

pub fn record_verdict(verdicts: &mut Vec<BuddyVerdict>, verdict: BuddyVerdict) {
    verdicts.push(verdict);
    let overflow = verdicts.len().saturating_sub(VERDICT_RING_CAP);
    if overflow > 0 {
        verdicts.drain(..overflow);
    }
}

pub fn dismiss_ratio(verdicts: &[BuddyVerdict], rule_key: &str) -> Option<f32> {
    let recent: Vec<&BuddyVerdict> = verdicts
        .iter()
        .rev()
        .filter(|v| v.rule_key == rule_key)
        .filter(|v| {
            matches!(
                v.verdict,
                VerdictOutcome::Accept | VerdictOutcome::Dismiss | VerdictOutcome::Never
            )
        })
        .take(DECAY_WINDOW)
        .collect();
    if recent.len() < DECAY_WINDOW {
        return None;
    }
    let dismissed = recent
        .iter()
        .filter(|v| matches!(v.verdict, VerdictOutcome::Dismiss | VerdictOutcome::Never))
        .count();
    Some(dismissed as f32 / recent.len() as f32)
}

pub fn cooldown_multiplier_for_rule(verdicts: &[BuddyVerdict], rule_key: &str) -> u64 {
    match dismiss_ratio(verdicts, rule_key) {
        Some(ratio) if ratio >= DECAY_RATIO => DECAY_COOLDOWN_MULTIPLIER,
        _ => 1,
    }
}

pub fn is_rule_muted(muted_rules: &[String], cooldown_key: &str) -> bool {
    let rule_key = rule_key_of(cooldown_key);
    muted_rules.iter().any(|m| *m == rule_key)
}

pub fn mute_rule(muted_rules: &mut Vec<String>, cooldown_key: &str) -> bool {
    let rule_key = rule_key_of(cooldown_key);
    if muted_rules.iter().any(|m| *m == rule_key) {
        return false;
    }
    muted_rules.push(rule_key);
    true
}

pub fn unmute_rule(muted_rules: &mut Vec<String>, rule_key: &str) -> bool {
    let before = muted_rules.len();
    muted_rules.retain(|m| m != rule_key);
    muted_rules.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(rule_key: &str, outcome: VerdictOutcome) -> BuddyVerdict {
        BuddyVerdict {
            rule_key: rule_key.to_string(),
            kind: "task_health".to_string(),
            action_kind: "dismiss".to_string(),
            verdict: outcome,
            at: Utc::now(),
        }
    }

    #[test]
    fn rule_key_strips_to_two_segments() {
        assert_eq!(
            rule_key_of("task_health:stuck:task-42"),
            "task_health:stuck"
        );
        assert_eq!(rule_key_of("memory_ops:batch"), "memory_ops:batch");
        assert_eq!(rule_key_of("single"), "single");
    }

    #[test]
    fn migrates_legacy_batch_keys_across_state() {
        let mut state = crate::state::default_buddy_state();
        state.dismissed_history.push(crate::types::DismissEntry {
            cooldown_key: "memory_ops:batch:archive".to_string(),
            dismissed_at: Utc::now(),
        });
        state
            .verdicts
            .push(verdict("memory_ops:batch", VerdictOutcome::Dismiss));
        state.muted_rules.push("memory_ops:batch".to_string());
        state.muted_rules.push("task_health:stuck".to_string());

        assert!(migrate_legacy_batch_keys(&mut state));

        assert_eq!(
            state.dismissed_history.last().unwrap().cooldown_key,
            "memory_ops_batch:archive"
        );
        assert_eq!(state.verdicts.last().unwrap().rule_key, "memory_ops_batch");
        assert!(state
            .muted_rules
            .iter()
            .all(|rule| !rule.starts_with("memory_ops:batch")));
        for class in crate::memory_lifecycle_model::MEMORY_BATCH_KEYS {
            let expected = format!("{}:{}", MEMORY_BATCH_PREFIX, class);
            assert!(state.muted_rules.contains(&expected), "missing {}", expected);
        }
        assert!(state.muted_rules.contains(&"task_health:stuck".to_string()));

        assert!(!migrate_legacy_batch_keys(&mut state));
    }

    #[test]
    fn legacy_batch_mute_still_gates_new_batch_opportunities() {
        let mut state = crate::state::default_buddy_state();
        state.muted_rules.push("memory_ops:batch".to_string());
        migrate_legacy_batch_keys(&mut state);
        assert!(is_rule_muted(
            &state.muted_rules,
            "memory_ops_batch:merge_exact_duplicate"
        ));
    }

    #[test]
    fn dismiss_ratio_requires_full_window() {
        let mut verdicts = Vec::new();
        for _ in 0..(DECAY_WINDOW - 1) {
            record_verdict(&mut verdicts, verdict("a:b", VerdictOutcome::Dismiss));
        }
        assert_eq!(dismiss_ratio(&verdicts, "a:b"), None);
        assert_eq!(cooldown_multiplier_for_rule(&verdicts, "a:b"), 1);

        record_verdict(&mut verdicts, verdict("a:b", VerdictOutcome::Dismiss));
        assert_eq!(dismiss_ratio(&verdicts, "a:b"), Some(1.0));
        assert_eq!(
            cooldown_multiplier_for_rule(&verdicts, "a:b"),
            DECAY_COOLDOWN_MULTIPLIER
        );
    }

    #[test]
    fn accepts_hold_decay_off() {
        let mut verdicts = Vec::new();
        for i in 0..DECAY_WINDOW {
            let outcome = if i % 2 == 0 {
                VerdictOutcome::Accept
            } else {
                VerdictOutcome::Dismiss
            };
            record_verdict(&mut verdicts, verdict("a:b", outcome));
        }
        assert_eq!(dismiss_ratio(&verdicts, "a:b"), Some(0.5));
        assert_eq!(cooldown_multiplier_for_rule(&verdicts, "a:b"), 1);
    }

    #[test]
    fn undo_verdicts_do_not_count_in_ratio() {
        let mut verdicts = Vec::new();
        for _ in 0..DECAY_WINDOW {
            record_verdict(&mut verdicts, verdict("a:b", VerdictOutcome::Undo));
        }
        assert_eq!(dismiss_ratio(&verdicts, "a:b"), None);
    }

    #[test]
    fn ring_caps_at_limit() {
        let mut verdicts = Vec::new();
        for _ in 0..(VERDICT_RING_CAP + 50) {
            record_verdict(&mut verdicts, verdict("a:b", VerdictOutcome::Accept));
        }
        assert_eq!(verdicts.len(), VERDICT_RING_CAP);
    }

    #[test]
    fn mute_and_unmute_round_trip() {
        let mut muted = Vec::new();
        assert!(mute_rule(&mut muted, "diag:cluster:llm_error:src"));
        assert!(!mute_rule(&mut muted, "diag:cluster:other:x"));
        assert!(is_rule_muted(&muted, "diag:cluster:whatever:y"));
        assert!(!is_rule_muted(&muted, "task_health:stuck:t1"));
        assert!(unmute_rule(&mut muted, "diag:cluster"));
        assert!(muted.is_empty());
        assert!(!unmute_rule(&mut muted, "diag:cluster"));
    }
}
