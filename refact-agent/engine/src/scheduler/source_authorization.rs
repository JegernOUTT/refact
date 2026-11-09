//! Trusted human-approval bridge. This module never enrolls credential values.
use std::{path::Path, sync::Arc};

use refact_apps_core::SourceDriver;
use refact_scheduler_core::sources::{AutomationService, CredentialContext};

use super::credentials::AutomationCredentialStore;

/// Call only from an exact, actual-human approval port, under the caller's
/// canonical definition/published-revision guard. Listing, ticks, demand and
/// restart must never mint private-store authorizations.
pub async fn authorize_source_bindings(
    root: &Path,
    sources: &Arc<AutomationService>,
    app: &str,
    source: &str,
    driver: &SourceDriver,
) -> Result<(), String> {
    let policy = sources.source_policy(app, source).await?;
    policy.validate()?;
    if policy.credential_bindings.is_empty() {
        return Ok(());
    }
    let url = match driver {
        SourceDriver::HttpPoll {
            url, ..
        }
        | SourceDriver::Sse {
            url,
        }
        | SourceDriver::Websocket {
            url,
        } => url,
        _ => return Err("credential bindings require a network source".into()),
    };
    let origin = reqwest::Url::parse(url)
        .map_err(|_| "invalid collector endpoint")?
        .origin()
        .ascii_serialization();
    let digest = sources.source_config_digest(app, source).await?;
    let context = CredentialContext {
        app_id: app,
        source_id: source,
        config_digest: &digest,
        origin: &origin,
    };
    let store = AutomationCredentialStore::open(root, None)?;
    let metadata = store.list_metadata()?;
    // Validate the entire requested set before granting any member. The opaque
    // identity revision covers tenant, principal and scope as well as origin/header.
    for binding in &policy.credential_bindings {
        let revision = binding
            .expected_identity_revision
            .as_deref()
            .filter(|r| !r.is_empty())
            .ok_or("credential binding requires expected identity revision")?;
        if !metadata.iter().any(|m| {
            m.identity.id == binding.reference
                && m.identity_revision == revision
                && m.identity.origin == origin
                && m.identity.header == binding.header.to_ascii_lowercase()
        }) {
            return Err("credential capability denied".into());
        }
    }
    for binding in &policy.credential_bindings {
        store.authorize_binding(
            &context,
            binding,
            binding.expected_identity_revision.as_deref().unwrap(),
        )?;
    }
    if sources.source_config_digest(app, source).await? != digest {
        for binding in &policy.credential_bindings {
            store.revoke_binding(&context, binding)?;
        }
        return Err("source configuration changed during credential approval".into());
    }
    Ok(())
}
