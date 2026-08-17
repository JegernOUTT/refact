use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::WebAuthn;
use serde::Serialize;

use refact_integrations::browser_models::{
    BrowserAuthenticatorProtocol, BrowserAuthenticatorTransport, BrowserWebAuthnCredential,
};

const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MaskedCredential {
    pub credential_id: String,
    pub is_resident_credential: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rp_id: Option<String>,
    pub private_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_handle: Option<String>,
    pub sign_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub large_blob: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_eligibility: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_state: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_display_name: Option<String>,
}

#[derive(Default)]
struct WebAuthnSession {
    enabled: bool,
    authenticator_ids: BTreeSet<String>,
}

#[derive(Default)]
pub struct WebAuthnManager {
    sessions: Mutex<HashMap<String, WebAuthnSession>>,
}

impl WebAuthnManager {
    pub fn add_virtual_authenticator(
        &self,
        tab: &Tab,
        protocol: BrowserAuthenticatorProtocol,
        transport: BrowserAuthenticatorTransport,
        has_resident_key: bool,
        has_user_verification: bool,
        is_user_verified: bool,
    ) -> Result<String, String> {
        self.ensure_enabled(tab)?;
        let result = tab
            .call_method(WebAuthn::AddVirtualAuthenticator {
                options: virtual_authenticator_options(
                    protocol,
                    transport,
                    has_resident_key,
                    has_user_verification,
                    is_user_verified,
                ),
            })
            .map_err(|error| format!("Failed to add virtual authenticator: {error}"))?;
        self.sessions
            .lock()
            .map_err(|error| error.to_string())?
            .entry(tab.get_target_id().to_string())
            .or_default()
            .authenticator_ids
            .insert(result.authenticator_id.clone());
        Ok(result.authenticator_id)
    }

    pub fn remove_virtual_authenticator(&self, tab: &Tab, id: &str) -> Result<(), String> {
        self.ensure_enabled(tab)?;
        tab.call_method(WebAuthn::RemoveVirtualAuthenticator {
            authenticator_id: id.to_string(),
        })
        .map_err(|error| format!("Failed to remove virtual authenticator: {error}"))?;
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(session) = sessions.get_mut(tab.get_target_id()) {
                session.authenticator_ids.remove(id);
            }
        }
        Ok(())
    }

    pub fn list_credentials(&self, tab: &Tab, id: &str) -> Result<Vec<MaskedCredential>, String> {
        self.ensure_enabled(tab)?;
        tab.call_method(WebAuthn::GetCredentials {
            authenticator_id: id.to_string(),
        })
        .map(|result| {
            result
                .credentials
                .iter()
                .map(mask_credential)
                .collect::<Vec<_>>()
        })
        .map_err(|error| format!("Failed to list virtual authenticator credentials: {error}"))
    }

    pub fn add_credential(
        &self,
        tab: &Tab,
        id: &str,
        credential: &BrowserWebAuthnCredential,
    ) -> Result<(), String> {
        self.ensure_enabled(tab)?;
        tab.call_method(WebAuthn::AddCredential {
            authenticator_id: id.to_string(),
            credential: credential_payload(credential),
        })
        .map(|_| ())
        .map_err(|error| format!("Failed to add virtual authenticator credential: {error}"))
    }

    pub fn clear_credentials(&self, tab: &Tab, id: &str) -> Result<(), String> {
        self.ensure_enabled(tab)?;
        tab.call_method(WebAuthn::ClearCredentials {
            authenticator_id: id.to_string(),
        })
        .map(|_| ())
        .map_err(|error| format!("Failed to clear virtual authenticator credentials: {error}"))
    }

    pub fn set_user_verified(&self, tab: &Tab, id: &str, verified: bool) -> Result<(), String> {
        self.ensure_enabled(tab)?;
        tab.call_method(WebAuthn::SetUserVerified {
            authenticator_id: id.to_string(),
            is_user_verified: verified,
        })
        .map(|_| ())
        .map_err(|error| format!("Failed to set virtual authenticator verification: {error}"))
    }

    pub fn cleanup(&self, tabs: &[std::sync::Arc<Tab>]) {
        let sessions = self
            .sessions
            .lock()
            .map(|mut sessions| std::mem::take(&mut *sessions))
            .unwrap_or_default();
        for (target_id, session) in sessions {
            let Some(tab) = tabs
                .iter()
                .find(|tab| tab.get_target_id().as_str() == target_id.as_str())
            else {
                continue;
            };
            for authenticator_id in session.authenticator_ids {
                let _ = tab.call_method(WebAuthn::RemoveVirtualAuthenticator { authenticator_id });
            }
            if session.enabled {
                let _ = tab.call_method(WebAuthn::Disable(None));
            }
        }
    }

    fn ensure_enabled(&self, tab: &Tab) -> Result<(), String> {
        let target_id = tab.get_target_id().to_string();
        let already_enabled = self
            .sessions
            .lock()
            .map_err(|error| error.to_string())?
            .get(&target_id)
            .is_some_and(|session| session.enabled);
        if already_enabled {
            return Ok(());
        }
        tab.call_method(WebAuthn::Enable {
            enable_ui: Some(false),
        })
        .map_err(|error| format!("Failed to enable WebAuthn: {error}"))?;
        self.sessions
            .lock()
            .map_err(|error| error.to_string())?
            .entry(target_id)
            .or_default()
            .enabled = true;
        Ok(())
    }
}

pub fn virtual_authenticator_options(
    protocol: BrowserAuthenticatorProtocol,
    transport: BrowserAuthenticatorTransport,
    has_resident_key: bool,
    has_user_verification: bool,
    is_user_verified: bool,
) -> WebAuthn::VirtualAuthenticatorOptions {
    WebAuthn::VirtualAuthenticatorOptions {
        protocol: match protocol {
            BrowserAuthenticatorProtocol::U2f => WebAuthn::AuthenticatorProtocol::U2F,
            BrowserAuthenticatorProtocol::Ctap2 => WebAuthn::AuthenticatorProtocol::Ctap2,
        },
        ctap_2_version: None,
        transport: match transport {
            BrowserAuthenticatorTransport::Usb => WebAuthn::AuthenticatorTransport::Usb,
            BrowserAuthenticatorTransport::Nfc => WebAuthn::AuthenticatorTransport::Nfc,
            BrowserAuthenticatorTransport::Ble => WebAuthn::AuthenticatorTransport::Ble,
            BrowserAuthenticatorTransport::Cable => WebAuthn::AuthenticatorTransport::Cable,
            BrowserAuthenticatorTransport::Internal => WebAuthn::AuthenticatorTransport::Internal,
        },
        has_resident_key: Some(has_resident_key),
        has_user_verification: Some(has_user_verification),
        has_large_blob: None,
        has_cred_blob: None,
        has_min_pin_length: None,
        has_prf: None,
        automatic_presence_simulation: Some(true),
        is_user_verified: Some(is_user_verified),
        default_backup_eligibility: None,
        default_backup_state: None,
    }
}

pub fn credential_payload(credential: &BrowserWebAuthnCredential) -> WebAuthn::Credential {
    WebAuthn::Credential {
        credential_id: credential.credential_id.clone(),
        is_resident_credential: credential.is_resident_credential,
        rp_id: credential.rp_id.clone(),
        private_key: credential.private_key.clone(),
        user_handle: credential.user_handle.clone(),
        sign_count: credential.sign_count,
        large_blob: credential.large_blob.clone(),
        backup_eligibility: credential.backup_eligibility,
        backup_state: credential.backup_state,
        user_name: credential.user_name.clone(),
        user_display_name: credential.user_display_name.clone(),
    }
}

pub fn mask_credential(credential: &WebAuthn::Credential) -> MaskedCredential {
    MaskedCredential {
        credential_id: REDACTED.to_string(),
        is_resident_credential: credential.is_resident_credential,
        rp_id: credential.rp_id.clone(),
        private_key: REDACTED.to_string(),
        user_handle: credential
            .user_handle
            .as_ref()
            .map(|_| REDACTED.to_string()),
        sign_count: credential.sign_count,
        large_blob: credential.large_blob.as_ref().map(|_| REDACTED.to_string()),
        backup_eligibility: credential.backup_eligibility,
        backup_state: credential.backup_state,
        user_name: credential.user_name.as_ref().map(|_| REDACTED.to_string()),
        user_display_name: credential
            .user_display_name
            .as_ref()
            .map(|_| REDACTED.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential() -> BrowserWebAuthnCredential {
        BrowserWebAuthnCredential {
            credential_id: "secret-id".to_string(),
            is_resident_credential: true,
            rp_id: Some("example.com".to_string()),
            private_key: "secret-private-key".to_string(),
            user_handle: Some("secret-user-handle".to_string()),
            sign_count: 7,
            large_blob: Some("secret-large-blob".to_string()),
            backup_eligibility: Some(true),
            backup_state: Some(false),
            user_name: Some("secret-name".to_string()),
            user_display_name: Some("Secret Display Name".to_string()),
        }
    }

    #[test]
    fn authenticator_options_map_to_cdp_payload() {
        let options = virtual_authenticator_options(
            BrowserAuthenticatorProtocol::Ctap2,
            BrowserAuthenticatorTransport::Internal,
            true,
            true,
            false,
        );
        assert_eq!(options.protocol, WebAuthn::AuthenticatorProtocol::Ctap2);
        assert_eq!(
            options.transport,
            WebAuthn::AuthenticatorTransport::Internal
        );
        assert_eq!(options.has_resident_key, Some(true));
        assert_eq!(options.has_user_verification, Some(true));
        assert_eq!(options.is_user_verified, Some(false));
        assert_eq!(options.automatic_presence_simulation, Some(true));
    }

    #[test]
    fn authenticator_options_serialize_required_passkey_capabilities() {
        let options = virtual_authenticator_options(
            BrowserAuthenticatorProtocol::Ctap2,
            BrowserAuthenticatorTransport::Internal,
            true,
            true,
            true,
        );
        assert_eq!(
            serde_json::to_value(options).unwrap(),
            serde_json::json!({
                "protocol": "ctap2",
                "transport": "internal",
                "hasResidentKey": true,
                "hasUserVerification": true,
                "automaticPresenceSimulation": true,
                "isUserVerified": true,
            })
        );
    }

    #[test]
    fn add_virtual_authenticator_request_carries_options_only() {
        let request = WebAuthn::AddVirtualAuthenticator {
            options: virtual_authenticator_options(
                BrowserAuthenticatorProtocol::Ctap2,
                BrowserAuthenticatorTransport::Internal,
                true,
                true,
                true,
            ),
        };

        let payload = serde_json::to_value(&request).unwrap();
        assert_eq!(
            payload.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["options"]
        );
        assert!(payload.get("authenticatorId").is_none());
        assert!(payload.get("id").is_none());
    }

    #[test]
    fn credential_payload_preserves_secret_material_only_for_cdp() {
        let source = credential();
        let payload = credential_payload(&source);
        assert_eq!(payload.credential_id, source.credential_id);
        assert_eq!(payload.private_key, source.private_key);
        assert_eq!(payload.user_handle, source.user_handle);
        assert_eq!(payload.sign_count, source.sign_count);
    }

    #[test]
    fn credential_reports_mask_every_secret_string() {
        let raw = credential_payload(&credential());
        let masked = mask_credential(&raw);
        let json = serde_json::to_string(&masked).unwrap();
        for secret in [
            "secret-id",
            "secret-private-key",
            "secret-user-handle",
            "secret-large-blob",
            "secret-name",
            "Secret Display Name",
        ] {
            assert!(!json.contains(secret));
        }
        assert_eq!(masked.credential_id, REDACTED);
        assert_eq!(masked.private_key, REDACTED);
        assert_eq!(masked.rp_id.as_deref(), Some("example.com"));
        assert_eq!(masked.sign_count, 7);
    }
}
