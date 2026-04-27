use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

use crate::buddy::types::{BuddyOpportunity, BuddyPulse, OpportunityStatus};

pub const MAX_OPPORTUNITIES: usize = 200;
pub const MAX_UNREAD: usize = 3;
pub const DISMISS_MEMORY: Duration = Duration::hours(24);
pub const DEFAULT_COOLDOWN: Duration = Duration::minutes(30);

/// Priority-ordered queue of `BuddyOpportunity` values with cooldown and dismissal tracking.
pub struct OpportunityQueue {
    items: Vec<BuddyOpportunity>,
    cooldowns: HashMap<String, DateTime<Utc>>,
    dismissed_history: HashMap<String, DateTime<Utc>>,
}

impl OpportunityQueue {
    /// Create an empty queue.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            cooldowns: HashMap::new(),
            dismissed_history: HashMap::new(),
        }
    }

    /// Rebuild a queue from persisted opportunities, reconstructing cooldowns from items.
    pub fn from_state(opps: Vec<BuddyOpportunity>) -> Self {
        let mut q = Self::new();
        let now = Utc::now();
        for opp in opps {
            let expires = opp.created_at + DEFAULT_COOLDOWN;
            if expires > now {
                q.cooldowns.insert(opp.cooldown_key.clone(), expires);
            }
            q.items.push(opp);
        }
        q
    }

    /// Push a new opportunity, setting `DEFAULT_COOLDOWN` on its cooldown key.
    ///
    /// Caps the queue at `MAX_OPPORTUNITIES`, evicting oldest terminal items first,
    /// then oldest by `created_at`.
    pub fn push(&mut self, opp: BuddyOpportunity) {
        let expires = Utc::now() + DEFAULT_COOLDOWN;
        self.cooldowns.insert(opp.cooldown_key.clone(), expires);
        self.items.push(opp);

        if self.items.len() > MAX_OPPORTUNITIES {
            let terminal = [
                OpportunityStatus::Expired,
                OpportunityStatus::Completed,
                OpportunityStatus::Dismissed,
            ];
            if let Some(pos) = self.items.iter().position(|o| terminal.contains(&o.status)) {
                self.items.remove(pos);
            } else if let Some(pos) = self
                .items
                .iter()
                .enumerate()
                .min_by_key(|(_, o)| o.created_at)
                .map(|(i, _)| i)
            {
                self.items.remove(pos);
            }
        }
    }

    /// Count opportunities with `New` or `Shown` status.
    pub fn unread_count(&self) -> usize {
        self.items
            .iter()
            .filter(|o| matches!(o.status, OpportunityStatus::New | OpportunityStatus::Shown))
            .count()
    }

    /// Return `true` if a cooldown is currently active for `key`.
    pub fn cooldown_active(&self, key: &str) -> bool {
        self.cooldowns
            .get(key)
            .map(|&exp| exp > Utc::now())
            .unwrap_or(false)
    }

    /// Return `true` if `key` was dismissed within `window` of now.
    pub fn recently_dismissed(&self, key: &str, window: Duration) -> bool {
        let cutoff = Utc::now() - window;
        self.dismissed_history
            .get(key)
            .map(|&t| t >= cutoff)
            .unwrap_or(false)
    }

    /// Update the status of the opportunity with `id`.
    pub fn mark_status(&mut self, id: &str, status: OpportunityStatus) {
        if let Some(opp) = self.items.iter_mut().find(|o| o.id == id) {
            opp.status = status;
        }
    }

    /// Dismiss the opportunity with `id`, recording the dismissal in history.
    pub fn dismiss(&mut self, id: &str) {
        if let Some(opp) = self.items.iter_mut().find(|o| o.id == id) {
            opp.status = OpportunityStatus::Dismissed;
            self.dismissed_history
                .insert(opp.cooldown_key.clone(), Utc::now());
        }
    }

    /// Mark items with `expires_at <= now` as `Expired`, then remove terminal
    /// items whose `created_at` is older than 24 hours before `now`.
    pub fn expire_old(&mut self, now: DateTime<Utc>) {
        let terminal = [
            OpportunityStatus::Expired,
            OpportunityStatus::Completed,
            OpportunityStatus::Dismissed,
        ];
        for opp in self.items.iter_mut() {
            if opp.expires_at <= now && !terminal.contains(&opp.status) {
                opp.status = OpportunityStatus::Expired;
            }
        }
        let cutoff = now - Duration::hours(24);
        self.items
            .retain(|o| !(terminal.contains(&o.status) && o.created_at < cutoff));
    }

    /// Remove stale (already-expired) entries from the cooldown map.
    pub fn refresh_cooldowns(&mut self, now: DateTime<Utc>) {
        self.cooldowns.retain(|_, exp| *exp > now);
    }

    /// Iterate over all opportunities.
    pub fn iter(&self) -> impl Iterator<Item = &BuddyOpportunity> {
        self.items.iter()
    }

    /// Clone all opportunities for persistence in state.json.
    pub fn snapshot(&self) -> Vec<BuddyOpportunity> {
        self.items.clone()
    }

    /// Look up an opportunity by id.
    pub fn get(&self, id: &str) -> Option<&BuddyOpportunity> {
        self.items.iter().find(|o| o.id == id)
    }

    /// Look up an opportunity mutably by id.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut BuddyOpportunity> {
        self.items.iter_mut().find(|o| o.id == id)
    }
}

impl Default for OpportunityQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Stub detector — returns an empty vec until T-7 implements rule evaluation.
// TODO(T-7): implement real detector rules using fact_store and pulse
pub struct OpportunityDetector;

impl OpportunityDetector {
    pub fn new() -> Self {
        Self
    }

    pub fn detect(
        &self,
        _fact_store: &crate::buddy::facts::FactStore,
        _pulse: &BuddyPulse,
        _queue: &OpportunityQueue,
    ) -> Vec<BuddyOpportunity> {
        vec![]
    }
}

impl Default for OpportunityDetector {
    fn default() -> Self {
        Self::new()
    }
}
