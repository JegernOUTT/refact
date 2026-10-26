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
}
