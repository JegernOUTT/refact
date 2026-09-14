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
        if let Some(existing) = grants.get(&key) {
            if existing.expires_at_ms > now_ms && existing.authority.rank() > grant.authority.rank()
            {
                return Err("peer grant cannot be silently narrowed or replaced".to_string());
            }
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
            .is_some_and(|grant| grant.authority.rank() >= required.rank())
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
