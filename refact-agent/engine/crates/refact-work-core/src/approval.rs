use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::Notify,
    time::{sleep_until, Instant},
};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PendingApproval {
    pub id: String,
    pub child_work_id: String,
    pub parent_chat_id: String,
    pub run_generation: u64,
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Value,
    pub digest: Option<String>,
    pub created_at: u64,
    pub expires_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionApproval>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionApproval {
    pub command: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub environment_keys: Vec<String>,
    pub fingerprint: String,
    pub privilege: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Rejected,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalError {
    Capacity,
    Duplicate,
    NotFound,
    NotOwner,
    GenerationMismatch,
    Expired,
    AlreadyDecided,
    BindingMismatch,
    NotApproved,
    Consumed,
}

struct UserOnceGrant {
    pending: PendingApproval,
    wrapper_tool_call_id: String,
    consumed: bool,
}

struct Entry {
    pending: PendingApproval,
    decision: Option<ApprovalDecision>,
    user_once: Option<UserOnceGrant>,
    consumed: bool,
}

#[derive(Clone)]
pub struct ApprovalRegistry {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
    notify: Arc<Notify>,
    max_pending: Option<usize>,
}

impl ApprovalRegistry {
    pub fn new(max_pending: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            notify: Arc::new(Notify::new()),
            max_pending: Some(max_pending),
        }
    }

    pub fn from_settings() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            notify: Arc::new(Notify::new()),
            max_pending: None,
        }
    }

    fn capacity(&self) -> usize {
        self.max_pending
            .unwrap_or_else(|| crate::coordination_settings().max_pending_approvals)
    }

    pub fn register(&self, pending: PendingApproval, now: u64) -> Result<(), ApprovalError> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        if entries.contains_key(&pending.id)
            || entries.values().any(|entry| {
                entry.pending.parent_chat_id == pending.parent_chat_id
                    && entry.pending.child_work_id == pending.child_work_id
                    && entry.pending.run_generation == pending.run_generation
                    && entry.pending.tool_call_id == pending.tool_call_id
                    && entry.pending.expires_at > now
            })
        {
            return Err(ApprovalError::Duplicate);
        }
        if pending.expires_at <= now {
            return Err(ApprovalError::Expired);
        }
        let active = entries
            .values()
            .filter(|entry| entry.pending.expires_at > now)
            .count();
        if active >= self.capacity() {
            return Err(ApprovalError::Capacity);
        }
        entries.insert(
            pending.id.clone(),
            Entry {
                pending,
                decision: None,
                user_once: None,
                consumed: false,
            },
        );
        Ok(())
    }

    pub fn grant_user_once(
        &self,
        parent: &str,
        id: &str,
        generation: u64,
        wrapper_tool_call_id: &str,
        now: u64,
    ) -> Result<(), ApprovalError> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        let entry = entries.get_mut(id).ok_or(ApprovalError::NotFound)?;
        Self::check_user_once_entry(entry, parent, generation)?;
        if wrapper_tool_call_id.is_empty() {
            return Err(ApprovalError::BindingMismatch);
        }
        if let Some(grant) = &entry.user_once {
            if grant.pending != entry.pending || grant.wrapper_tool_call_id != wrapper_tool_call_id
            {
                return Err(ApprovalError::BindingMismatch);
            }
            return if grant.consumed {
                Err(ApprovalError::Consumed)
            } else {
                Ok(())
            };
        }
        entry.user_once = Some(UserOnceGrant {
            pending: entry.pending.clone(),
            wrapper_tool_call_id: wrapper_tool_call_id.to_owned(),
            consumed: false,
        });
        Ok(())
    }

    pub fn consume_user_once(
        &self,
        parent: &str,
        id: &str,
        generation: u64,
        wrapper_tool_call_id: &str,
        now: u64,
    ) -> Result<(), ApprovalError> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        let entry = entries.get_mut(id).ok_or(ApprovalError::NotFound)?;
        Self::check_user_once_entry(entry, parent, generation)?;
        let grant = entry.user_once.as_mut().ok_or(ApprovalError::NotApproved)?;
        if grant.pending != entry.pending || grant.wrapper_tool_call_id != wrapper_tool_call_id {
            return Err(ApprovalError::BindingMismatch);
        }
        if grant.consumed {
            return Err(ApprovalError::Consumed);
        }
        grant.consumed = true;
        Ok(())
    }

    fn check_user_once_entry(
        entry: &Entry,
        parent: &str,
        generation: u64,
    ) -> Result<(), ApprovalError> {
        if entry.pending.parent_chat_id != parent {
            return Err(ApprovalError::NotOwner);
        }
        if entry.pending.run_generation != generation {
            return Err(ApprovalError::GenerationMismatch);
        }
        match entry.decision {
            Some(ApprovalDecision::Expired) => Err(ApprovalError::Expired),
            Some(_) => Err(ApprovalError::AlreadyDecided),
            None => Ok(()),
        }
    }

    pub fn decide(
        &self,
        caller_parent: &str,
        expected_generation: u64,
        id: &str,
        approve: bool,
        now: u64,
    ) -> Result<ApprovalDecision, ApprovalError> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        let entry = entries.get_mut(id).ok_or(ApprovalError::NotFound)?;
        if entry.pending.parent_chat_id != caller_parent {
            return Err(ApprovalError::NotOwner);
        }
        if entry.pending.run_generation != expected_generation {
            return Err(ApprovalError::GenerationMismatch);
        }
        if entry.decision == Some(ApprovalDecision::Expired) {
            return Err(ApprovalError::Expired);
        }
        let wanted = if approve {
            ApprovalDecision::Approved
        } else {
            ApprovalDecision::Rejected
        };
        if let Some(existing) = entry.decision {
            return if existing == wanted {
                Ok(existing)
            } else {
                Err(ApprovalError::AlreadyDecided)
            };
        }
        entry.decision = Some(wanted);
        entry.user_once = None;
        self.notify.notify_waiters();
        Ok(wanted)
    }

    pub fn cancel(
        &self,
        caller_parent: &str,
        expected_generation: u64,
        id: &str,
        now: u64,
    ) -> Result<(), ApprovalError> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        let entry = entries.get_mut(id).ok_or(ApprovalError::NotFound)?;
        if entry.pending.parent_chat_id != caller_parent {
            return Err(ApprovalError::NotOwner);
        }
        if entry.pending.run_generation != expected_generation {
            return Err(ApprovalError::GenerationMismatch);
        }
        if entry.decision.is_some() {
            return Err(ApprovalError::AlreadyDecided);
        }
        entry.decision = Some(ApprovalDecision::Cancelled);
        entry.user_once = None;
        self.notify.notify_waiters();
        Ok(())
    }

    pub async fn wait(&self, id: &str, now: u64) -> Result<ApprovalDecision, ApprovalError> {
        let started = Instant::now();
        let pending = {
            let entries = self.entries.lock().unwrap();
            entries
                .get(id)
                .ok_or(ApprovalError::NotFound)?
                .pending
                .clone()
        };
        let expiry = pending.expires_at;
        let deadline = started + Duration::from_millis(expiry.saturating_sub(now));
        let mut timeout = Box::pin(sleep_until(deadline));
        loop {
            let notification = self.notify.notified();
            tokio::pin!(notification);
            notification.as_mut().enable();
            let elapsed = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let current = now.saturating_add(elapsed);
            let decision = {
                let mut entries = self.entries.lock().unwrap();
                self.expire_locked(&mut entries, current);
                match entries.get(id) {
                    Some(entry) if entry.pending == pending => entry.decision,
                    _ => Some(ApprovalDecision::Expired),
                }
            };
            if let Some(decision) = decision {
                return Ok(decision);
            }
            tokio::select! {
                _ = &mut timeout => {
                    let mut entries = self.entries.lock().unwrap();
                    self.expire_locked(&mut entries, expiry);
                    return Ok(entries.get(id).filter(|entry| entry.pending == pending).and_then(|entry| entry.decision).unwrap_or(ApprovalDecision::Expired));
                }
                _ = &mut notification => {}
            }
        }
    }

    pub fn consume(
        &self,
        id: &str,
        tool_call_id: &str,
        args: &Value,
        now: u64,
    ) -> Result<PendingApproval, ApprovalError> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        let entry = entries.get_mut(id).ok_or(ApprovalError::NotFound)?;
        if entry.decision != Some(ApprovalDecision::Approved) {
            return Err(match entry.decision {
                Some(ApprovalDecision::Expired) => ApprovalError::Expired,
                Some(ApprovalDecision::Cancelled) | Some(ApprovalDecision::Rejected) => {
                    ApprovalError::NotApproved
                }
                None => ApprovalError::NotApproved,
                Some(ApprovalDecision::Approved) => unreachable!(),
            });
        }
        if entry.pending.tool_call_id != tool_call_id || entry.pending.args != *args {
            return Err(ApprovalError::BindingMismatch);
        }
        if entry.consumed {
            return Err(ApprovalError::Consumed);
        }
        entry.consumed = true;
        Ok(entry.pending.clone())
    }

    pub fn get(
        &self,
        caller_parent: &str,
        id: &str,
        now: u64,
    ) -> Result<PendingApproval, ApprovalError> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        let entry = entries.get(id).ok_or(ApprovalError::NotFound)?;
        if entry.pending.parent_chat_id != caller_parent {
            return Err(ApprovalError::NotOwner);
        }
        Ok(entry.pending.clone())
    }

    pub fn list_for_parent(&self, caller_parent: &str, now: u64) -> Vec<PendingApproval> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        let mut pending: Vec<_> = entries
            .values()
            .filter(|entry| entry.pending.parent_chat_id == caller_parent)
            .map(|entry| entry.pending.clone())
            .collect();
        pending
            .sort_by(|left, right| (left.created_at, &left.id).cmp(&(right.created_at, &right.id)));
        pending
    }

    /// Every request that is still awaiting a decision, across all parents.
    /// The wait-for graph needs the undecided set, not one parent's view.
    pub fn undecided(&self, now: u64) -> Vec<PendingApproval> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        let mut pending: Vec<_> = entries
            .values()
            .filter(|entry| entry.decision.is_none())
            .map(|entry| entry.pending.clone())
            .collect();
        pending
            .sort_by(|left, right| (left.created_at, &left.id).cmp(&(right.created_at, &right.id)));
        pending
    }

    pub fn expire(&self, now: u64) {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        self.notify.notify_waiters();
    }
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
    pub fn wait_now(&self, id: &str, now: u64) -> Result<ApprovalDecision, ApprovalError> {
        let mut entries = self.entries.lock().unwrap();
        self.expire_locked(&mut entries, now);
        let entry = entries.get(id).ok_or(ApprovalError::NotFound)?;
        entry.decision.ok_or(ApprovalError::NotApproved)
    }

    fn expire_locked(&self, entries: &mut HashMap<String, Entry>, now: u64) {
        for entry in entries.values_mut() {
            if entry.pending.expires_at <= now {
                entry.decision = Some(ApprovalDecision::Expired);
                entry.user_once = None;
            }
        }
        let mut expired: Vec<_> = entries
            .iter()
            .filter(|(_, entry)| entry.decision == Some(ApprovalDecision::Expired))
            .map(|(id, entry)| (entry.pending.expires_at, id.clone()))
            .collect();
        let purge_count = expired.len().saturating_sub(self.capacity());
        if purge_count > 0 {
            expired.sort_unstable();
            for (_, id) in expired.into_iter().take(purge_count) {
                entries.remove(&id);
            }
            self.notify.notify_waiters();
        }
    }
}

#[cfg(test)]
mod user_grant_tests {
    use super::*;

    fn registry() -> ApprovalRegistry {
        let registry = ApprovalRegistry::new(2);
        registry
            .register(
                PendingApproval {
                    id: "request".into(),
                    child_work_id: "child".into(),
                    parent_chat_id: "parent".into(),
                    run_generation: 2,
                    tool_call_id: "command".into(),
                    tool_name: "shell".into(),
                    args: serde_json::json!({"command":"echo hello"}),
                    digest: Some("checkout".into()),
                    created_at: 0,
                    expires_at: 100,
                    execution: None,
                },
                0,
            )
            .unwrap();
        registry
    }

    #[test]
    fn grant_is_bound_to_user_confirmed_wrapper_and_consumed_once() {
        let registry = registry();
        assert_eq!(
            registry.consume_user_once("parent", "request", 2, "wrapper", 1),
            Err(ApprovalError::NotApproved)
        );
        registry
            .grant_user_once("parent", "request", 2, "wrapper", 1)
            .unwrap();
        assert_eq!(
            registry.consume_user_once("parent", "request", 2, "other", 2),
            Err(ApprovalError::BindingMismatch)
        );
        registry
            .consume_user_once("parent", "request", 2, "wrapper", 2)
            .unwrap();
        assert_eq!(
            registry.consume_user_once("parent", "request", 2, "wrapper", 3),
            Err(ApprovalError::Consumed)
        );
        assert_eq!(
            registry.grant_user_once("parent", "request", 2, "wrapper", 3),
            Err(ApprovalError::Consumed)
        );
    }

    #[test]
    fn a_tool_call_cannot_obtain_a_second_request_in_the_same_run() {
        let registry = registry();
        let mut duplicate = registry.get("parent", "request", 0).unwrap();
        duplicate.id = "another-request".into();
        assert_eq!(
            registry.register(duplicate.clone(), 1),
            Err(ApprovalError::Duplicate)
        );
        registry.decide("parent", 2, "request", true, 2).unwrap();
        registry
            .consume("request", "command", &duplicate.args, 3)
            .unwrap();
        assert_eq!(
            registry.register(duplicate.clone(), 4),
            Err(ApprovalError::Duplicate)
        );
        duplicate.run_generation = 3;
        registry.register(duplicate, 5).unwrap();
    }

    #[test]
    fn grants_reject_foreign_stale_cancelled_and_expired_requests() {
        let registry = registry();
        assert_eq!(
            registry.grant_user_once("other", "request", 2, "wrapper", 1),
            Err(ApprovalError::NotOwner)
        );
        assert_eq!(
            registry.grant_user_once("parent", "request", 1, "wrapper", 1),
            Err(ApprovalError::GenerationMismatch)
        );
        registry
            .grant_user_once("parent", "request", 2, "wrapper", 1)
            .unwrap();
        registry.cancel("parent", 2, "request", 2).unwrap();
        assert_eq!(
            registry.consume_user_once("parent", "request", 2, "wrapper", 3),
            Err(ApprovalError::AlreadyDecided)
        );
        assert_eq!(
            registry.consume_user_once("parent", "request", 2, "wrapper", 100),
            Err(ApprovalError::Expired)
        );
    }

    #[test]
    fn grant_binds_arguments_digest_and_id() {
        for mutate_digest in [false, true] {
            let registry = registry();
            registry
                .grant_user_once("parent", "request", 2, "wrapper", 1)
                .unwrap();
            assert_eq!(
                registry.grant_user_once("parent", "request", 2, "wrapper", 2),
                Ok(())
            );
            assert_eq!(
                registry.grant_user_once("parent", "request", 2, "other", 2),
                Err(ApprovalError::BindingMismatch)
            );
            assert_eq!(
                registry.consume_user_once("other", "request", 2, "wrapper", 2),
                Err(ApprovalError::NotOwner)
            );
            assert_eq!(
                registry.consume_user_once("parent", "request", 3, "wrapper", 2),
                Err(ApprovalError::GenerationMismatch)
            );
            assert_eq!(
                registry.consume_user_once("parent", "other", 2, "wrapper", 2),
                Err(ApprovalError::NotFound)
            );
            {
                let mut entries = registry.entries.lock().unwrap();
                let entry = entries.get_mut("request").unwrap();
                if mutate_digest {
                    entry.pending.digest = Some("different".into());
                } else {
                    entry.pending.args = serde_json::json!({"command": "different"});
                }
            }
            assert_eq!(
                registry.grant_user_once("parent", "request", 2, "wrapper", 3),
                Err(ApprovalError::BindingMismatch)
            );
            assert_eq!(
                registry.consume_user_once("parent", "request", 2, "wrapper", 3),
                Err(ApprovalError::BindingMismatch)
            );
        }
    }

    #[test]
    fn grant_does_not_decide_and_terminal_decisions_revoke_it() {
        for approve in [false, true] {
            let registry = registry();
            assert_eq!(
                registry.grant_user_once("parent", "request", 2, "", 1),
                Err(ApprovalError::BindingMismatch)
            );
            registry
                .grant_user_once("parent", "request", 2, "wrapper", 1)
                .unwrap();
            assert_eq!(
                registry.wait_now("request", 2),
                Err(ApprovalError::NotApproved)
            );
            registry.decide("parent", 2, "request", approve, 2).unwrap();
            assert_eq!(
                registry.consume_user_once("parent", "request", 2, "wrapper", 3),
                Err(ApprovalError::AlreadyDecided)
            );
            assert_eq!(
                registry.grant_user_once("parent", "request", 2, "wrapper", 3),
                Err(ApprovalError::AlreadyDecided)
            );
            assert!(registry
                .entries
                .lock()
                .unwrap()
                .get("request")
                .unwrap()
                .user_once
                .is_none());
        }
        let registry = registry();
        registry
            .grant_user_once("parent", "request", 2, "wrapper", 1)
            .unwrap();
        registry
            .consume_user_once("parent", "request", 2, "wrapper", 2)
            .unwrap();
        assert_eq!(
            registry.wait_now("request", 2),
            Err(ApprovalError::NotApproved)
        );
        registry.decide("parent", 2, "request", true, 3).unwrap();
        assert!(registry
            .consume(
                "request",
                "command",
                &serde_json::json!({"command":"echo hello"}),
                4
            )
            .is_ok());
    }

    #[test]
    fn expired_grants_cannot_be_consumed_or_restored_by_reused_ids() {
        let registry = registry();
        let original = registry.get("parent", "request", 0).unwrap();
        registry
            .grant_user_once("parent", "request", 2, "wrapper", 1)
            .unwrap();
        assert_eq!(
            registry.consume_user_once("parent", "request", 2, "wrapper", 100),
            Err(ApprovalError::Expired)
        );
        assert_eq!(
            registry.grant_user_once("parent", "request", 2, "wrapper", 100),
            Err(ApprovalError::Expired)
        );
        assert!(registry
            .entries
            .lock()
            .unwrap()
            .get("request")
            .unwrap()
            .user_once
            .is_none());
        for now in 100..200 {
            let mut pending = original.clone();
            pending.id = format!("request-{now}");
            pending.expires_at = now + 1;
            registry.register(pending, now).unwrap();
            assert!(registry.len() <= registry.capacity() * 2);
        }
        registry.expire(200);
        assert_eq!(registry.len(), registry.capacity());
        assert_eq!(
            registry.consume_user_once("parent", "request", 2, "wrapper", 200),
            Err(ApprovalError::NotFound)
        );
        let mut replacement = original;
        replacement.run_generation = 3;
        replacement.expires_at = 300;
        registry.register(replacement, 200).unwrap();
        assert_eq!(
            registry.consume_user_once("parent", "request", 2, "wrapper", 201),
            Err(ApprovalError::GenerationMismatch)
        );
        assert_eq!(
            registry.consume_user_once("parent", "request", 3, "wrapper", 201),
            Err(ApprovalError::NotApproved)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn waiter_observes_expiry_after_purge_even_when_id_is_reused() {
        use std::{future::Future, task::Poll};

        for reuse_id in [false, true] {
            let registry = registry();
            let original = registry.get("parent", "request", 0).unwrap();
            let waiter = registry.wait("request", 0);
            tokio::pin!(waiter);
            std::future::poll_fn(|cx| {
                assert!(waiter.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            for now in 100..103 {
                let mut pending = original.clone();
                pending.id = format!("request-{now}");
                pending.expires_at = now + 1;
                registry.register(pending, now).unwrap();
            }
            assert!(!registry.entries.lock().unwrap().contains_key("request"));
            if reuse_id {
                let mut pending = original;
                pending.expires_at = 200;
                registry.register(pending, 103).unwrap();
                registry.decide("parent", 2, "request", true, 104).unwrap();
            }
            assert_eq!(waiter.await, Ok(ApprovalDecision::Expired));
        }
    }
}
