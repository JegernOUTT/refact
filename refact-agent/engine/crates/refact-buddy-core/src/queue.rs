use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

use crate::types::{BuddyOpportunity, DismissEntry, OpportunityStatus};

pub const MAX_OPPORTUNITIES: usize = 200;
pub const MAX_UNREAD: usize = 3;
pub const DISMISS_MEMORY: Duration = Duration::hours(24);
pub const DEFAULT_COOLDOWN: Duration = Duration::minutes(30);

pub fn is_terminal_status(status: OpportunityStatus) -> bool {
    matches!(
        status,
        OpportunityStatus::Dismissed
            | OpportunityStatus::Accepted
            | OpportunityStatus::Completed
            | OpportunityStatus::Expired
    )
}

pub struct OpportunityQueue {
    pub items: Vec<BuddyOpportunity>,
    pub cooldowns: HashMap<String, DateTime<Utc>>,
    pub dismissed_history: HashMap<String, DateTime<Utc>>,
}

impl OpportunityQueue {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            cooldowns: HashMap::new(),
            dismissed_history: HashMap::new(),
        }
    }

    pub fn from_state(opps: Vec<BuddyOpportunity>, dismissed: Vec<DismissEntry>) -> Self {
        let mut q = Self::new();
        let now = Utc::now();
        let dismissed_cutoff = now - DISMISS_MEMORY;
        for entry in dismissed {
            if entry.dismissed_at >= dismissed_cutoff {
                q.dismissed_history
                    .insert(entry.cooldown_key, entry.dismissed_at);
            }
        }
        for opp in opps {
            let expires = opp.created_at + Duration::seconds(opp.cooldown_secs as i64);
            if expires > now {
                q.cooldowns.insert(opp.cooldown_key.clone(), expires);
            }
            q.items.push(opp);
            q.cap_items();
        }
        q
    }

    fn cap_items(&mut self) {
        while self.items.len() > MAX_OPPORTUNITIES {
            if let Some(pos) = self.items.iter().position(|o| is_terminal_status(o.status)) {
                self.items.remove(pos);
            } else if let Some(pos) = self
                .items
                .iter()
                .enumerate()
                .min_by_key(|(_, o)| o.created_at)
                .map(|(i, _)| i)
            {
                self.items.remove(pos);
            } else {
                break;
            }
        }
    }

    pub fn push_with_cooldown(&mut self, mut opp: BuddyOpportunity, cooldown_secs: u64) {
        opp.cooldown_secs = cooldown_secs;
        let expires = Utc::now() + Duration::seconds(cooldown_secs as i64);
        self.cooldowns.insert(opp.cooldown_key.clone(), expires);
        self.items.push(opp);
        self.cap_items();
    }

    pub fn unread_count(&self) -> usize {
        self.items
            .iter()
            .filter(|o| matches!(o.status, OpportunityStatus::New | OpportunityStatus::Shown))
            .count()
    }

    pub fn cooldown_active(&self, key: &str) -> bool {
        self.cooldowns
            .get(key)
            .map(|&exp| exp > Utc::now())
            .unwrap_or(false)
    }

    pub fn recently_dismissed(&self, key: &str, window: Duration) -> bool {
        let cutoff = Utc::now() - window;
        self.dismissed_history
            .get(key)
            .map(|&t| t >= cutoff)
            .unwrap_or(false)
    }

    pub fn mark_status(&mut self, id: &str, status: OpportunityStatus) -> bool {
        let Some(opp) = self.items.iter_mut().find(|o| o.id == id) else {
            return false;
        };
        let mut changed = false;
        if opp.status != status {
            opp.status = status;
            changed = true;
        }
        if is_terminal_status(status) && opp.resolved_at.is_none() {
            opp.resolved_at = Some(Utc::now());
            changed = true;
        }
        changed
    }

    pub fn dismiss(&mut self, id: &str) -> bool {
        let Some(opp) = self.items.iter_mut().find(|o| o.id == id) else {
            return false;
        };
        if opp.status == OpportunityStatus::Dismissed
            && opp.resolved_at.is_some()
            && self.dismissed_history.contains_key(&opp.cooldown_key)
        {
            return false;
        }
        let now = Utc::now();
        let mut changed = false;
        if opp.status != OpportunityStatus::Dismissed {
            opp.status = OpportunityStatus::Dismissed;
            changed = true;
        }
        if opp.resolved_at.is_none() {
            opp.resolved_at = Some(now);
            changed = true;
        }
        if self.dismissed_history.insert(opp.cooldown_key.clone(), now) != Some(now) {
            changed = true;
        }
        changed
    }

    pub fn expire_old(&mut self, now: DateTime<Utc>) -> bool {
        let mut changed = false;
        for opp in self.items.iter_mut() {
            if opp.expires_at <= now && !is_terminal_status(opp.status) {
                opp.status = OpportunityStatus::Expired;
                opp.resolved_at.get_or_insert(now);
                changed = true;
            }
        }
        let cutoff = now - DISMISS_MEMORY;
        let before_items = self.items.len();
        self.items.retain(|o| {
            if !is_terminal_status(o.status) {
                return true;
            }
            let terminal_since = o.resolved_at.unwrap_or(o.created_at);
            terminal_since >= cutoff
        });
        changed |= self.items.len() != before_items;
        let before_history = self.dismissed_history.len();
        self.dismissed_history
            .retain(|_, dismissed_at| *dismissed_at >= cutoff);
        changed |= self.dismissed_history.len() != before_history;
        changed
    }

    pub fn refresh_cooldowns(&mut self, now: DateTime<Utc>) {
        self.cooldowns.retain(|_, exp| *exp > now);
    }

    pub fn iter(&self) -> impl Iterator<Item = &BuddyOpportunity> {
        self.items.iter()
    }

    pub fn snapshot(&self) -> Vec<BuddyOpportunity> {
        self.items.clone()
    }

    pub fn get(&self, id: &str) -> Option<&BuddyOpportunity> {
        self.items.iter().find(|o| o.id == id)
    }

    pub fn dismissed_history_snapshot(&self) -> Vec<DismissEntry> {
        self.dismissed_history
            .iter()
            .map(|(k, v)| DismissEntry {
                cooldown_key: k.clone(),
                dismissed_at: *v,
            })
            .collect()
    }
}

impl Default for OpportunityQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BuddyOpportunityKind, BuddyOpportunityLinks, BuddyPriority};

    fn make_opportunity(id: &str, cooldown_key: &str) -> BuddyOpportunity {
        let now = Utc::now();
        BuddyOpportunity {
            id: id.to_string(),
            kind: BuddyOpportunityKind::TaskHealth,
            summary: "test".to_string(),
            priority: BuddyPriority::Normal,
            confidence: 0.9,
            fact_keys: vec![],
            cooldown_key: cooldown_key.to_string(),
            cooldown_secs: 1800,
            status: OpportunityStatus::New,
            proposed_actions: vec![],
            humor: None,
            humor_allowed: false,
            related: BuddyOpportunityLinks::default(),
            created_at: now,
            expires_at: now + Duration::hours(1),
            resolved_at: None,
        }
    }

    fn push_opportunity(queue: &mut OpportunityQueue, opp: BuddyOpportunity) {
        queue.push_with_cooldown(opp, DEFAULT_COOLDOWN.num_seconds() as u64);
    }

    #[test]
    fn unread_count_ignores_terminal_statuses() {
        let mut q = OpportunityQueue::new();
        push_opportunity(&mut q, make_opportunity("opp1", "ck1"));
        assert_eq!(q.unread_count(), 1);
        q.mark_status("opp1", OpportunityStatus::Dismissed);
        assert_eq!(q.unread_count(), 0);
    }

    #[test]
    fn cooldown_active_uses_per_push_duration() {
        let mut q = OpportunityQueue::new();
        q.push_with_cooldown(make_opportunity("opp-zero", "ck-zero"), 0);
        assert!(!q.cooldown_active("ck-zero"));
        q.push_with_cooldown(make_opportunity("opp-long", "ck-long"), 3600);
        assert!(q.cooldown_active("ck-long"));
    }

    #[test]
    fn expire_old_marks_and_prunes_expired_opportunities() {
        let now = Utc::now();
        let mut q = OpportunityQueue::new();
        let mut opp = make_opportunity("opp1", "ck1");
        opp.expires_at = now - Duration::hours(1);
        opp.created_at = now - Duration::minutes(5);
        push_opportunity(&mut q, opp);
        assert!(q.expire_old(now));
        assert_eq!(
            q.get("opp1").map(|o| o.status),
            Some(OpportunityStatus::Expired)
        );
        assert!(q.expire_old(now + Duration::hours(25)));
        assert!(q.get("opp1").is_none());
    }

    #[test]
    fn cap_removes_oldest_when_no_terminal_items_exist() {
        let mut q = OpportunityQueue::new();
        for i in 0..=MAX_OPPORTUNITIES {
            push_opportunity(
                &mut q,
                make_opportunity(&format!("opp-{i}"), &format!("ck-{i}")),
            );
        }
        assert_eq!(q.iter().count(), MAX_OPPORTUNITIES);
    }

    #[test]
    fn dismiss_marks_recent_history() {
        let mut q = OpportunityQueue::new();
        push_opportunity(&mut q, make_opportunity("opp-dm", "ck-dm"));
        assert!(q.dismiss("opp-dm"));
        assert!(q.recently_dismissed("ck-dm", Duration::hours(24)));
        assert!(!q.dismiss("opp-dm"));
    }

    #[test]
    fn terminal_retention_uses_resolved_at() {
        let now = Utc::now();
        let mut opp1 = make_opportunity("opp-rt1", "ck-rt1");
        opp1.created_at = now - Duration::hours(48);
        opp1.expires_at = now - Duration::hours(47);
        opp1.status = OpportunityStatus::Dismissed;
        opp1.resolved_at = Some(now);

        let mut q1 = OpportunityQueue::new();
        q1.items.push(opp1);
        q1.expire_old(now + Duration::minutes(23 * 60 + 59));
        assert!(q1.get("opp-rt1").is_some());

        let mut opp2 = make_opportunity("opp-rt2", "ck-rt2");
        opp2.created_at = now - Duration::hours(48);
        opp2.expires_at = now - Duration::hours(47);
        opp2.status = OpportunityStatus::Completed;
        opp2.resolved_at = Some(now);

        let mut q2 = OpportunityQueue::new();
        q2.items.push(opp2);
        q2.expire_old(now + Duration::minutes(24 * 60 + 1));
        assert!(q2.get("opp-rt2").is_none());
    }

    #[test]
    fn dismissed_history_prunes_old_entries() {
        let now = Utc::now();
        let mut q = OpportunityQueue::new();
        q.dismissed_history.insert(
            "old".to_string(),
            now - DISMISS_MEMORY - Duration::seconds(1),
        );
        q.dismissed_history.insert("fresh".to_string(), now);

        assert!(q.expire_old(now));
        assert!(!q.dismissed_history.contains_key("old"));
        assert!(q.dismissed_history.contains_key("fresh"));
    }

    #[test]
    fn from_state_caps_oversized_opportunities() {
        let now = Utc::now();
        let mut opps = vec![];
        for i in 0..(MAX_OPPORTUNITIES + 25) {
            let mut opp = make_opportunity(&format!("opp-cap-{i}"), &format!("ck-cap-{i}"));
            opp.created_at = now - Duration::minutes(i as i64);
            opps.push(opp);
        }

        let queue = OpportunityQueue::from_state(opps, vec![]);
        assert_eq!(queue.snapshot().len(), MAX_OPPORTUNITIES);
    }

    #[test]
    fn from_state_preserves_per_rule_cooldown() {
        let now = Utc::now();
        let cooldown_secs = 7200u64;
        let mut opp = make_opportunity("opp-cd-persist", "ck-cd-persist");
        opp.cooldown_secs = cooldown_secs;
        opp.created_at = now;

        let queue = OpportunityQueue::from_state(vec![opp], vec![]);
        let exp = queue
            .cooldowns
            .get("ck-cd-persist")
            .copied()
            .expect("cooldown must be present");
        let expected_exp = now + Duration::seconds(cooldown_secs as i64);
        let delta = (exp - expected_exp).num_seconds().abs();
        assert!(delta <= 2);
        assert!(exp > now + Duration::minutes(90));
    }

    #[test]
    fn mark_status_sets_resolved_at_for_terminal_statuses() {
        let mut q = OpportunityQueue::new();
        push_opportunity(&mut q, make_opportunity("opp-accepted", "ck-accepted"));
        assert!(q.mark_status("opp-accepted", OpportunityStatus::Accepted));
        assert!(q.get("opp-accepted").and_then(|o| o.resolved_at).is_some());
    }
}
