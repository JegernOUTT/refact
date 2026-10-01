use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerAuthority {
    Metadata,
    Mail,
    Control,
}

impl PeerAuthority {
    fn rank(self) -> u8 {
        match self {
            PeerAuthority::Metadata => 1,
            PeerAuthority::Mail => 2,
            PeerAuthority::Control => 3,
        }
    }

    /// The only place the ladder is read. `Metadata` never satisfies `Mail`
    /// and `Mail` never satisfies `Control`, so discovering a peer grants no
    /// messaging and holding mail grants no cancel, preempt or mutation.
    pub fn allows(self, required: PeerAuthority) -> bool {
        self.rank() >= required.rank()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerGrant {
    pub project: String,
    pub sender_root: String,
    pub recipient_root: String,
    pub authority: PeerAuthority,
    pub granted_by: String,
    pub expires_at_ms: u64,
}

#[derive(Clone, Default)]
pub struct PeerGrantRegistry {
    grants: Arc<Mutex<HashMap<(String, String, String), PeerGrant>>>,
}

impl PeerGrantRegistry {
    pub fn grant(&self, grant: PeerGrant, now_ms: u64) -> Result<(), String> {
        if grant.project.trim().is_empty()
            || grant.sender_root.trim().is_empty()
            || grant.recipient_root.trim().is_empty()
            || grant.granted_by.trim().is_empty()
            || grant.sender_root == grant.recipient_root
        {
            return Err("peer grant identities must be distinct and nonempty".to_string());
        }
        if grant.expires_at_ms <= now_ms {
            return Err("peer grant is already expired".to_string());
        }
        let key = (
            grant.project.clone(),
            grant.sender_root.clone(),
            grant.recipient_root.clone(),
        );
        let mut grants = self
            .grants
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        grants.retain(|_, existing| existing.expires_at_ms > now_ms);
        if let Some(existing) = grants.get(&key) {
            if existing.authority.rank() > grant.authority.rank() {
                return Err("peer grant cannot be silently narrowed or replaced".to_string());
            }
        } else if grants.len() >= refact_work_core::coordination_settings().peer_grant_max_count {
            return Err("peer grant registry is full".to_string());
        }
        grants.insert(key, grant);
        Ok(())
    }

    pub fn allows(
        &self,
        project: &str,
        sender_root: &str,
        recipient_root: &str,
        required: PeerAuthority,
        now_ms: u64,
    ) -> bool {
        let mut grants = self
            .grants
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        grants.retain(|_, grant| grant.expires_at_ms > now_ms);
        grants
            .get(&(
                project.to_string(),
                sender_root.to_string(),
                recipient_root.to_string(),
            ))
            .is_some_and(|grant| grant.authority.allows(required))
    }

    pub fn get(
        &self,
        project: &str,
        sender_root: &str,
        recipient_root: &str,
        now_ms: u64,
    ) -> Option<PeerGrant> {
        self.grants
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(
                project.to_string(),
                sender_root.to_string(),
                recipient_root.to_string(),
            ))
            .filter(|grant| grant.expires_at_ms > now_ms)
            .cloned()
    }

    pub fn revoke(&self, project: &str, sender_root: &str, recipient_root: &str) -> bool {
        self.grants
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&(
                project.to_string(),
                sender_root.to_string(),
                recipient_root.to_string(),
            ))
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(authority: PeerAuthority, expires_at_ms: u64) -> PeerGrant {
        PeerGrant {
            project: "project".to_string(),
            sender_root: "root-a".to_string(),
            recipient_root: "root-b".to_string(),
            authority,
            granted_by: "user".to_string(),
            expires_at_ms,
        }
    }

    #[test]
    fn metadata_mail_and_control_are_distinct_and_expire() {
        let registry = PeerGrantRegistry::default();
        registry
            .grant(grant(PeerAuthority::Metadata, 100), 1)
            .unwrap();
        assert!(registry.allows("project", "root-a", "root-b", PeerAuthority::Metadata, 2));
        assert!(!registry.allows("project", "root-a", "root-b", PeerAuthority::Mail, 2));
        registry.grant(grant(PeerAuthority::Mail, 100), 2).unwrap();
        assert!(registry.allows("project", "root-a", "root-b", PeerAuthority::Mail, 3));
        assert!(!registry.allows("project", "root-a", "root-b", PeerAuthority::Control, 3));
        assert!(!registry.allows("project", "root-b", "root-a", PeerAuthority::Metadata, 3));
        assert!(!registry.allows("project", "root-a", "root-b", PeerAuthority::Metadata, 100));
    }

    /// The whole ladder in one table: a held authority satisfies itself and
    /// everything below it, and nothing above it.
    #[test]
    fn the_authority_ladder_is_strict_in_both_directions() {
        use PeerAuthority::{Control, Mail, Metadata};
        for (held, required, expected) in [
            (Metadata, Metadata, true),
            (Metadata, Mail, false),
            (Metadata, Control, false),
            (Mail, Metadata, true),
            (Mail, Mail, true),
            (Mail, Control, false),
            (Control, Metadata, true),
            (Control, Mail, true),
            (Control, Control, true),
        ] {
            assert_eq!(held.allows(required), expected, "{held:?} -> {required:?}");
            let registry = PeerGrantRegistry::default();
            registry.grant(grant(held, 100), 1).unwrap();
            assert_eq!(
                registry.allows("project", "root-a", "root-b", required, 2),
                expected,
                "registry {held:?} -> {required:?}"
            );
        }
    }

    #[test]
    fn grants_are_bounded_and_expired_entries_free_room() {
        let registry = PeerGrantRegistry::default();
        let max = refact_work_core::coordination_settings().peer_grant_max_count;
        for index in 0..max {
            registry
                .grant(
                    PeerGrant {
                        recipient_root: format!("root-{index}"),
                        ..grant(PeerAuthority::Mail, 100)
                    },
                    1,
                )
                .unwrap();
        }
        assert_eq!(
            registry
                .grant(
                    PeerGrant {
                        recipient_root: "overflow".to_string(),
                        ..grant(PeerAuthority::Mail, 100)
                    },
                    1,
                )
                .unwrap_err(),
            "peer grant registry is full"
        );
        registry
            .grant(
                PeerGrant {
                    recipient_root: "overflow".to_string(),
                    ..grant(PeerAuthority::Mail, 200)
                },
                101,
            )
            .unwrap();
    }

    #[test]
    fn grants_are_project_scoped_and_revocable() {
        let registry = PeerGrantRegistry::default();
        registry
            .grant(grant(PeerAuthority::Control, 100), 1)
            .unwrap();
        assert!(!registry.allows("other", "root-a", "root-b", PeerAuthority::Metadata, 2));
        assert!(registry.revoke("project", "root-a", "root-b"));
        assert!(!registry.allows("project", "root-a", "root-b", PeerAuthority::Metadata, 2));
    }
}
