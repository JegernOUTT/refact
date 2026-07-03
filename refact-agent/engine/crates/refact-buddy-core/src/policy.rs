use crate::queue::OpportunityQueue;
use crate::settings::{BuddySettings, HumorLevel};
use crate::types::{BuddyOpportunity, BuddyPriority};

/// Result of evaluating a proposed opportunity against the policy gate.
#[derive(Debug, Clone)]
pub enum PolicyDecision {
    /// Opportunity dropped; will not surface to the user.
    Drop { reason: &'static str },
    /// Opportunity may be shown to the user.
    Surface { humor_allowed: bool },
}

/// Evaluate whether `opp` should surface to the user given current settings and queue state.
pub fn evaluate(
    opp: &BuddyOpportunity,
    settings: &BuddySettings,
    queue: &OpportunityQueue,
) -> PolicyDecision {
    evaluate_with_mutes(opp, settings, queue, &[])
}

pub fn evaluate_with_mutes(
    opp: &BuddyOpportunity,
    settings: &BuddySettings,
    queue: &OpportunityQueue,
    muted_rules: &[String],
) -> PolicyDecision {
    if !settings.proactive_enabled {
        return PolicyDecision::Drop {
            reason: "proactive_disabled",
        };
    }
    if crate::verdicts::is_rule_muted(muted_rules, &opp.cooldown_key) {
        return PolicyDecision::Drop {
            reason: "rule_muted",
        };
    }
    if settings.quiet_mode && opp.priority < BuddyPriority::Critical {
        return PolicyDecision::Drop {
            reason: "quiet_mode",
        };
    }
    if queue.unread_count() >= crate::queue::MAX_UNREAD {
        return PolicyDecision::Drop {
            reason: "unread_cap",
        };
    }
    if queue.recently_dismissed(&opp.cooldown_key, crate::queue::DISMISS_MEMORY) {
        return PolicyDecision::Drop {
            reason: "dismissed_24h",
        };
    }
    if queue.cooldown_active(&opp.cooldown_key) {
        return PolicyDecision::Drop { reason: "cooldown" };
    }

    let humor_allowed = settings.humor_enabled
        && !matches!(settings.humor_level, HumorLevel::Off)
        && opp.priority <= BuddyPriority::Normal
        && !humor_blocked_by_topic(opp);
    PolicyDecision::Surface { humor_allowed }
}

/// Return `true` when the opportunity's topic is too sensitive or severe for humor.
fn humor_blocked_by_topic(opp: &BuddyOpportunity) -> bool {
    let kw = [
        "auth",
        "token",
        "security",
        "panic",
        "critical",
        "frustration",
    ];
    let summary_lower = opp.summary.to_lowercase();
    if kw.iter().any(|k| summary_lower.contains(k)) {
        return true;
    }
    opp.fact_keys
        .iter()
        .any(|k| k.starts_with("chat:retry_streak"))
}
