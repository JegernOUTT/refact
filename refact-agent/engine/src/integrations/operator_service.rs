//! Integration-owned local configuration preparation. Never calls settings_apply/load_integrations:
//! those operations may start processes, authenticate, connect or publish tools.
use crate::buddy::resources::{
    ResourceError,
    integrations::{McpDraft, McpEndpoint},
};
use crate::ext::operator_service::{exact_names, opaque_reference};

pub async fn write_config_atomically(path: &std::path::Path, content: &str) -> Result<(), String> {
    crate::ext::operator_service::write_file_atomic(path, content)
        .await
        .map_err(|e| e.to_string())
}
pub fn prepare_mcp(before: Option<&[u8]>, draft: &McpDraft) -> Result<Vec<u8>, ResourceError> {
    let invalid =
        || ResourceError::ValidationFailed("invalid MCP endpoint or scoped references".into());
    crate::ext::operator_service::validate_name(&draft.instance_id)
        .map_err(ResourceError::ValidationFailed)?;
    if !exact_names(&draft.tool_access_scope) {
        return Err(invalid());
    }
    let endpoint = match &draft.endpoint {
        McpEndpoint::Stdio {
            command,
            args,
            command_reference,
        } => {
            // An executable plus argv, never a shell command; actual executable identity is grant-bound.
            if command.is_empty()
                || command.chars().any(|c| c.is_control() || c.is_whitespace())
                || command.contains([';', '|', '&', '$', '`', '<', '>'])
                || !opaque_reference(command_reference)
                || args.iter().any(|a| a.contains('\0'))
            {
                return Err(invalid());
            }
            serde_json::json!({"command":shell_words::join(std::iter::once(command.as_str()).chain(args.iter().map(String::as_str)))})
        }
        McpEndpoint::Http {
            url,
            credential_references,
        } => {
            let parsed = url::Url::parse(url).map_err(|_| invalid())?;
            if parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || credential_references.iter().any(|r| !opaque_reference(r))
            {
                return Err(invalid());
            }
            // Auth references remain private grant requirements; never written as tokens/headers.
            serde_json::json!({"url":parsed.as_str()})
        }
    };
    crate::integrations::mcp::integr_mcp_stdio::unified_transport_choice(&endpoint)
        .map_err(|_| invalid())?;
    let mut config: serde_json::Value = match before {
        Some(b) => serde_yaml::from_slice(b).map_err(|_| invalid())?,
        None => serde_json::json!({}),
    };
    let map = config.as_object_mut().ok_or_else(invalid)?;
    // Refuse migration of live/credential-bearing instances: do not silently forward old credentials
    // to a new endpoint, or change a live transport behind the running owner's back.
    if before.is_some()
        && (map
            .get("available")
            .and_then(|a| a.get("on_your_laptop"))
            .and_then(|v| v.as_bool())
            != Some(false)
            || map
                .get("available")
                .and_then(|a| a.get("when_isolated"))
                .and_then(|v| v.as_bool())
                != Some(false))
    {
        return Err(ResourceError::ValidationFailed(
            "disable existing MCP via native owner before endpoint change".into(),
        ));
    }
    for key in [
        "env",
        "headers",
        "oauth_tokens",
        "auth",
        "api_key",
        "bearer_token",
    ] {
        if map
            .get(key)
            .is_some_and(|v| !v.is_null() && v != &serde_json::json!({}))
        {
            return Err(ResourceError::ValidationFailed(
                "credential migration needs native credential owner".into(),
            ));
        }
    }
    for key in ["command", "args", "url", "transport", "sse_url", "http_url"] {
        map.remove(key);
    }
    for (key, value) in endpoint.as_object().ok_or_else(invalid)? {
        map.insert(key.clone(), value.clone());
    }
    let available = map
        .entry("available")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(invalid)?;
    available.insert("on_your_laptop".into(), false.into());
    available.insert("when_isolated".into(), false.into());
    serde_yaml::to_string(&config)
        .map(|s| s.into_bytes())
        .map_err(|_| invalid())
}

/// Canonical operator configuration: never inherits native transport credentials or callbacks.
///
/// # Why credential references are rejected rather than resolved
///
/// This stays fail-closed deliberately. The codebase has no API that resolves an opaque
/// credential *reference* into a secret at connect time without persisting it, and inventing an
/// auth fallback here is out of scope. The two native mechanisms that do exist are both unusable
/// for this path:
///
/// 1. `setting_up_integrations::get_vars_for_replacements` — loads `secrets.yaml`/`variables.yaml`
///    into a `HashMap<String, String>`, applied by plain `$NAME` substitution over top-level
///    config string values inside `read_integrations_d`. It is keyed by variable name, not by an
///    opaque reference, and the operator path deliberately never calls the loader
///    (`settings_apply`/`load_integrations` may start processes, authenticate and publish tools).
///    Resolving through it would also require writing the literal secret into `prepared.after`
///    (the durable disk bytes) — exactly what `prepare_mcp` forbids.
/// 2. `mcp_auth::MCPAuthSettings` — carries literal `bearer_token` / `oauth2_client_secret` /
///    `oauth_tokens` persisted in the config YAML, or `AuthType::Oauth2Pkce` driven by
///    `AuthorizationManager`, which is an interactive browser flow and not reference-keyed.
///
/// Injecting a secret only in-memory (into `PreparedMcp::config`, never onto disk) is also not
/// sound today: `McpSessionOwner::effective_config` compares the live `launched_cfg` against the
/// on-disk config, and `McpRecovery::previous_effective` persists that effective configuration as
/// durable recovery material — so the secret would leak into the material store.
///
/// Missing API required before this can be implemented: a resolver
/// `fn resolve(reference: &str) -> Secret` backed by a real secret store, plus a
/// non-persisting header/auth injection point in
/// `integr_mcp_common::serve_streamable_http_transport` (which today takes owned
/// `HashMap<String, String>` headers and `MCPAuthSettings` by value), and an
/// `effective_config`/recovery representation that compares and stores references rather than
/// resolved values. Until then the reference list is carried to the native rights owner as part
/// of the grant requirement (see `operator_probe::prepare`), and never written or connected with.
pub(crate) fn prepare_runtime_config(
    before: Option<&[u8]>,
    draft: &McpDraft,
    enabled: bool,
) -> Result<Vec<u8>, ResourceError> {
    if matches!(&draft.endpoint, McpEndpoint::Http { credential_references, .. } if !credential_references.is_empty())
    {
        return Err(ResourceError::ValidationFailed(
            "credential-bearing probes require native credential resolution".into(),
        ));
    }

    let mut previous: serde_json::Value = before
        .map(serde_yaml::from_slice)
        .transpose()
        .map_err(|_| ResourceError::ValidationFailed("invalid MCP config".into()))?
        .unwrap_or_else(|| serde_json::json!({}));
    // Only explicitly restricted operator configurations may be changed while enabled.
    // The normal preparation path rejects every other live configuration.
    if let Some(marker) = previous.get("operator_tools") {
        let tools: Vec<String> = serde_json::from_value(marker.clone())
            .map_err(|_| ResourceError::ValidationFailed("invalid operator tool scope".into()))?;
        if !exact_names(&tools) {
            return Err(ResourceError::ValidationFailed(
                "invalid operator tool scope".into(),
            ));
        }
        previous["available"] = serde_json::json!({"on_your_laptop":false,"when_isolated":false});
    }
    let had_before = before.is_some();
    let previous_bytes = serde_yaml::to_string(&previous)
        .map_err(|_| ResourceError::ValidationFailed("invalid MCP config".into()))?;
    let disabled = prepare_mcp(had_before.then_some(previous_bytes.as_bytes()), draft)?;
    let mut config: serde_json::Value = serde_yaml::from_slice(&disabled)
        .map_err(|_| ResourceError::ValidationFailed("invalid MCP config".into()))?;
    config["operator_tools"] = serde_json::json!(draft.tool_access_scope);
    for key in [
        "auth_type",
        "bearer_token",
        "oauth2_client_id",
        "oauth2_client_secret",
        "oauth2_token_url",
        "oauth2_scopes",
        "oauth_tokens",
    ] {
        if config
            .get(key)
            .is_some_and(|v| !v.is_null() && v != "" && v != "none" && v != &serde_json::json!([]))
        {
            return Err(ResourceError::ValidationFailed(
                "credential migration needs native credential owner".into(),
            ));
        }
        config.as_object_mut().unwrap().remove(key);
    }

    config["sampling_policy"] = "deny".into();
    config["reconnect_enabled"] = false.into();
    config["auto_approve_tools"] = serde_json::json!([]);
    config["disabled_tools"] = serde_json::json!([]);
    config["init_timeout"] = "5".into();
    config["request_timeout"] = "5".into();
    config["available"] = serde_json::json!({"on_your_laptop":enabled,"when_isolated":false});
    serde_yaml::to_string(&config)
        .map(String::into_bytes)
        .map_err(|_| ResourceError::ValidationFailed("invalid MCP config".into()))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct McpRecovery {
    spec: refact_buddy_core::operator_types::ActionSpec,
    before: Option<Vec<u8>>,
    post: String,
    applied_config: serde_json::Value,
    previous_effective: Option<serde_json::Value>,
}

/// One owning service coordinates durable bytes, native session publication and compensation.
/// Unrestricted native sessions are not migratable through this API.
pub struct McpSessionOwner {
    gcx: std::sync::Arc<crate::global_context::GlobalContext>,
    materials: std::sync::Arc<dyn crate::buddy::resources::RecoveryMaterialStore>,
    mutation: tokio::sync::Mutex<()>,
}
impl McpSessionOwner {
    pub fn new(
        gcx: std::sync::Arc<crate::global_context::GlobalContext>,
        materials: std::sync::Arc<dyn crate::buddy::resources::RecoveryMaterialStore>,
    ) -> Self {
        Self {
            gcx,
            materials,
            mutation: Default::default(),
        }
    }

    pub(crate) async fn effective_config(
        session: &super::operator_probe::Session,
    ) -> Result<serde_json::Value, ResourceError> {
        use super::mcp::session_mcp::SessionMCP;
        let mut locked = session.lock().await;
        let native = locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .ok_or(ResourceError::UnsupportedResource)?;
        if native.connection_status != super::mcp::session_mcp::MCPConnectionStatus::Connected {
            return Err(ResourceError::ResourceMissing);
        }

        if native.launched_cfg.get("operator_tools").is_none()
            || native.startup_task_handles.is_some()
            || native.health_task_handle.is_some()
            || native.resource_index_task.is_some()
            || native.oauth_refresh_task_handle.is_some()
        {
            return Err(ResourceError::ValidationFailed(
                "native session must be retired by its own authority".into(),
            ));
        }
        let client = native
            .mcp_client
            .as_ref()
            .ok_or(ResourceError::ResourceMissing)?;
        if !client
            .lock()
            .await
            .as_ref()
            .is_some_and(|c| c.service().restricted)
        {
            return Err(ResourceError::UnsupportedResource);
        }
        Ok(native.launched_cfg.clone())
    }

    async fn activate(candidate: &mut super::operator_probe::Candidate) {
        use super::mcp::session_mcp::{SessionMCP, MCPConnectionStatus};
        let client = candidate.client.take().expect("connected candidate");
        let mut locked = candidate.session.lock().await;
        let native = locked.as_any_mut().downcast_mut::<SessionMCP>().unwrap();
        let scope: Vec<String> =
            serde_json::from_value(native.launched_cfg["operator_tools"].clone())
                .unwrap_or_default();
        native.mcp_tools = std::mem::take(&mut candidate.tools)
            .into_iter()
            .filter(|t| scope.iter().any(|n| n == t.name.as_ref()))
            .collect();
        native.server_info = client.peer_info().as_deref().cloned();
        native.mcp_client = Some(std::sync::Arc::new(tokio::sync::Mutex::new(Some(client))));
        native.connection_status = MCPConnectionStatus::Connected;
        native.last_successful_connection = Some(std::time::Instant::now());
    }

    async fn client_slot(
        session: &super::operator_probe::Session,
    ) -> Option<
        std::sync::Arc<tokio::sync::Mutex<Option<super::mcp::session_mcp::McpRunningService>>>,
    > {
        session
            .lock()
            .await
            .as_any_mut()
            .downcast_mut::<super::mcp::session_mcp::SessionMCP>()
            .and_then(|s| s.mcp_client.clone())
    }
    async fn retire(
        &self,
        session: super::operator_probe::Session,
        client: Option<super::mcp::session_mcp::McpRunningService>,
    ) {
        super::operator_probe::retire(&self.gcx, session, client).await;
    }
    pub async fn reload(
        &self,
        rights: &dyn crate::ext::operator_service::ResourceRights,
        spec: &refact_buddy_core::operator_types::ActionSpec,
        draft: &McpDraft,
    ) -> Result<crate::buddy::resources::ResourceApplyReceipt, ResourceError> {
        use crate::ext::operator_service as files;
        if spec.kind != "mcp_enabled_reload" {
            return Err(ResourceError::UnsupportedResource);
        }
        let _guard = self.mutation.lock().await;
        let prepared = super::operator_probe::authorize(rights, spec, draft)?;
        let previous = self
            .gcx
            .integration_sessions
            .lock()
            .await
            .get(&prepared.path)
            .cloned();
        let previous_effective = match &previous {
            Some(session) => Some(Self::effective_config(session).await?),
            None => None,
        };
        if let Some(effective) = &previous_effective {
            let disk: serde_json::Value = serde_yaml::from_slice(
                prepared
                    .before
                    .as_deref()
                    .ok_or(ResourceError::VersionMismatch)?,
            )
            .map_err(|_| ResourceError::VersionMismatch)?;
            if effective != &disk {
                return Err(ResourceError::VersionMismatch);
            }
        }

        let recovery = McpRecovery {
            spec: spec.clone(),
            before: prepared.before.clone(),
            post: prepared.post.clone(),
            applied_config: prepared.config.clone(),
            previous_effective,
        };
        let material =
            serde_json::to_vec(&recovery).map_err(|_| ResourceError::MaterialUnavailable)?;
        let reference = self.materials.put(&material)?;
        let mut candidate = super::operator_probe::connect(
            self.gcx.clone(),
            &prepared.path,
            prepared.config.clone(),
        )
        .await?;
        let granted: std::collections::HashSet<&str> =
            draft.tool_access_scope.iter().map(String::as_str).collect();
        let actual: std::collections::HashSet<&str> = candidate
            .tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect();
        if !granted.iter().all(|name| actual.contains(name)) {
            return Err(ResourceError::ValidationFailed(
                "connected MCP capabilities do not contain the complete granted tool allowlist"
                    .into(),
            ));
        }
        Self::activate(&mut candidate).await;
        let mut sessions = self.gcx.integration_sessions.lock().await;
        if !same_session(sessions.get(&prepared.path), previous.as_ref()) {
            return Err(ResourceError::VersionMismatch);
        }
        if let Some(previous) = &previous {
            if Self::effective_config(previous).await?
                != recovery.previous_effective.clone().unwrap()
            {
                return Err(ResourceError::VersionMismatch);
            }
        }
        let old_slot = match &previous {
            Some(s) => Self::client_slot(s).await,
            None => None,
        };
        let mut old_client = match &old_slot {
            Some(slot) => Some(slot.lock().await),
            None => None,
        };
        // Fresh exact rights/config check after the bounded handshake, immediately before commit.
        let current = super::operator_probe::authorize(rights, spec, draft)?;
        if current.after != prepared.after {
            return Err(ResourceError::VersionMismatch);
        }
        files::replace(
            &spec.project,
            &prepared.relative,
            &spec.expected_version,
            &Some(prepared.after),
        )?;
        // The prepared candidate is already initialized; commit has no await before publication.
        super::mcp::integr_mcp_common::tool_catalog_cache_remove(&prepared.path);
        sessions.insert(prepared.path, candidate.session.clone());
        candidate.disarm();
        let retired = old_client.as_mut().and_then(|slot| slot.take());
        drop(old_client);
        drop(sessions);
        if let Some(previous) = previous {
            self.retire(previous, retired).await;
        }
        self.gcx.tool_catalog_generations.advance_mcp();
        Ok(crate::buddy::resources::ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: reference,
            expected_post_version: prepared.post,
            state: crate::buddy::resources::ResourceApplyState::Applied,
        })
    }

    pub async fn verify(
        &self,
        spec: &refact_buddy_core::operator_types::ActionSpec,
        post: &str,
    ) -> Result<bool, ResourceError> {
        use crate::ext::operator_service as files;
        files::validate_name(&spec.resource.resource_id)
            .map_err(ResourceError::ValidationFailed)?;
        let relative = format!("integrations.d/mcp_{}.yaml", spec.resource.resource_id);
        let disk = files::snapshot(&spec.project, &relative)?;
        if files::version(&disk) != post {
            return Ok(false);
        }
        let config: Option<serde_json::Value> = disk
            .map(|bytes| serde_yaml::from_slice(&bytes))
            .transpose()
            .map_err(|_| ResourceError::ValidationFailed("invalid MCP config".into()))?;
        let path = files::checked_path(&spec.project, &relative)?
            .to_string_lossy()
            .into_owned();
        let session = self
            .gcx
            .integration_sessions
            .lock()
            .await
            .get(&path)
            .cloned();
        match session {
            Some(session) => Ok(Self::effective_config(&session)
                .await
                .is_ok_and(|current| Some(current) == config)),
            None => Ok(config.is_none_or(|config| {
                config
                    .pointer("/available/on_your_laptop")
                    .and_then(|v| v.as_bool())
                    == Some(false)
                    && config
                        .pointer("/available/when_isolated")
                        .and_then(|v| v.as_bool())
                        == Some(false)
            })),
        }
    }

    pub async fn recover(
        &self,
        spec: &refact_buddy_core::operator_types::ActionSpec,
        descriptor: &crate::buddy::resources::RecoveryDescriptor,
    ) -> Result<crate::buddy::resources::ResourceApplyState, ResourceError> {
        use crate::ext::operator_service as files;
        let _guard = self.mutation.lock().await;
        let recovery: McpRecovery = serde_json::from_slice(
            &self
                .materials
                .get(&descriptor.previous_material_reference)?,
        )
        .map_err(|_| ResourceError::MaterialUnavailable)?;
        if recovery.spec != *spec
            || recovery.post != descriptor.expected_post_version
            || files::version(&recovery.before) != spec.expected_version
        {
            return Err(ResourceError::MaterialUnavailable);
        }
        files::validate_name(&spec.resource.resource_id)
            .map_err(ResourceError::ValidationFailed)?;
        let relative = format!("integrations.d/mcp_{}.yaml", spec.resource.resource_id);
        let path = files::checked_path(&spec.project, &relative)?
            .to_string_lossy()
            .into_owned();
        let disk = files::snapshot(&spec.project, &relative)?;
        if files::version(&disk) != recovery.post {
            return Err(ResourceError::VersionMismatch);
        }
        let current_config: serde_json::Value =
            serde_yaml::from_slice(disk.as_deref().ok_or(ResourceError::VersionMismatch)?)
                .map_err(|_| ResourceError::VersionMismatch)?;
        if current_config != recovery.applied_config {
            return Err(ResourceError::VersionMismatch);
        }
        let active = self
            .gcx
            .integration_sessions
            .lock()
            .await
            .get(&path)
            .cloned();
        let Some(active) = active else {
            let sessions = self.gcx.integration_sessions.lock().await;
            if sessions.contains_key(&path) {
                return Err(ResourceError::VersionMismatch);
            }
            files::replace(&spec.project, &relative, &recovery.post, &recovery.before)?;
            super::mcp::integr_mcp_common::tool_catalog_cache_remove(&path);
            self.gcx.tool_catalog_generations.advance_mcp();
            return Ok(if recovery.previous_effective.is_some() {
                crate::buddy::resources::ResourceApplyState::DiskDoneReloadNeeded
            } else {
                crate::buddy::resources::ResourceApplyState::Applied
            });
        };
        if Self::effective_config(&active).await? != recovery.applied_config {
            return Err(ResourceError::VersionMismatch);
        }
        // Exact previous restricted effective configuration is private durable recovery material;
        // no mutable draft or current endpoint is reinterpreted and no new rights are minted.
        let mut previous = match recovery.previous_effective {
            Some(config) => {
                let before: serde_json::Value = serde_yaml::from_slice(
                    recovery
                        .before
                        .as_deref()
                        .ok_or(ResourceError::MaterialUnavailable)?,
                )
                .map_err(|_| ResourceError::MaterialUnavailable)?;
                if before != config || config.get("operator_tools").is_none() {
                    return Err(ResourceError::MaterialUnavailable);
                }
                let candidate =
                    super::operator_probe::connect(self.gcx.clone(), &path, config.clone()).await?;
                let scope: Vec<String> = serde_json::from_value(config["operator_tools"].clone())
                    .map_err(|_| ResourceError::MaterialUnavailable)?;
                if !scope.iter().all(|name| {
                    candidate
                        .tools
                        .iter()
                        .any(|tool| tool.name.as_ref() == name)
                }) {
                    return Err(ResourceError::ValidationFailed(
                        "previous MCP capabilities changed".into(),
                    ));
                }
                Some(candidate)
            }
            None => None,
        };
        if let Some(candidate) = previous.as_mut() {
            Self::activate(candidate).await;
        }
        let mut sessions = self.gcx.integration_sessions.lock().await;
        if !same_session(sessions.get(&path), Some(&active))
            || Self::effective_config(&active).await? != recovery.applied_config
        {
            return Err(ResourceError::VersionMismatch);
        }
        let old_slot = Self::client_slot(&active).await;
        let mut old_client = match &old_slot {
            Some(slot) => Some(slot.lock().await),
            None => None,
        };
        files::replace(&spec.project, &relative, &recovery.post, &recovery.before)?;
        if let Some(candidate) = previous.as_mut() {
            sessions.insert(path.clone(), candidate.session.clone());
            candidate.disarm();
        } else {
            sessions.remove(&path);
        }
        super::mcp::integr_mcp_common::tool_catalog_cache_remove(&path);
        let retired = old_client.as_mut().and_then(|slot| slot.take());
        drop(old_client);
        drop(sessions);
        self.retire(active, retired).await;
        self.gcx.tool_catalog_generations.advance_mcp();
        Ok(crate::buddy::resources::ResourceApplyState::Applied)
    }
}

fn same_session(
    a: Option<&super::operator_probe::Session>,
    b: Option<&super::operator_probe::Session>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Materials(std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>);
    impl crate::buddy::resources::RecoveryMaterialStore for Materials {
        fn put(&self, bytes: &[u8]) -> Result<String, ResourceError> {
            let id = uuid::Uuid::new_v4().to_string();
            self.0.lock().unwrap().insert(id.clone(), bytes.to_vec());
            Ok(id)
        }
        fn get(&self, id: &str) -> Result<Vec<u8>, ResourceError> {
            self.0
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .ok_or(ResourceError::MaterialUnavailable)
        }
    }
    struct Rights {
        spec: refact_buddy_core::operator_types::ActionSpec,
        post: String,
        requirement: serde_json::Value,
    }
    impl crate::ext::operator_service::ResourceRights for Rights {
        fn authorize(
            &self,
            spec: &refact_buddy_core::operator_types::ActionSpec,
            post: &str,
            requirement: &serde_json::Value,
        ) -> Result<(), ResourceError> {
            if spec == &self.spec && post == self.post && requirement == &self.requirement {
                Ok(())
            } else {
                Err(ResourceError::VersionMismatch)
            }
        }
    }
    fn action(
        project: refact_buddy_core::operator_types::ProjectIdentity,
        kind: &str,
        version: &str,
    ) -> refact_buddy_core::operator_types::ActionSpec {
        use refact_buddy_core::operator_types::*;
        ActionSpec {
            schema_version: 1,
            action_id: uuid::Uuid::new_v4().to_string(),
            adapter: "integrations".into(),
            kind: kind.into(),
            project,
            resource: ResourceRef {
                resource_kind: "mcp".into(),
                resource_id: "fixture".into(),
                version: version.into(),
            },
            expected_version: version.into(),
            parameter_ref: Some("fixture-draft".into()),
            availability: ActionAvailability::Available,
            confirmation: ConfirmationClass::UserConfirmationRequired,
            authority_class: AuthorityClass::Mutating,
            capabilities: ActionCapabilities {
                can_preview: true,
                can_execute: true,
                can_verify: true,
                can_recover: true,
                can_cancel: false,
            },
        }
    }
    fn grant(spec: &refact_buddy_core::operator_types::ActionSpec, draft: &McpDraft) -> Rights {
        let prepared = crate::integrations::operator_probe::prepare(spec, draft).unwrap();
        Rights {
            spec: spec.clone(),
            post: prepared.post,
            requirement: prepared.requirement,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn operator_mcp_revoked_after_handshake_never_publishes_or_writes() {
        use crate::ext::operator_service::{self as files, ResourceRights};
        struct Revoke {
            calls: std::sync::atomic::AtomicUsize,
        }
        impl ResourceRights for Revoke {
            fn authorize(
                &self,
                _: &refact_buddy_core::operator_types::ActionSpec,
                _: &str,
                _: &serde_json::Value,
            ) -> Result<(), ResourceError> {
                if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    Ok(())
                } else {
                    Err(ResourceError::VersionMismatch)
                }
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: temp
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let program = r#"import sys,json
for l in sys.stdin:
 q=json.loads(l)
 if q.get('method')=='initialize': r={'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
 elif q.get('method')=='tools/list': r={'tools':[{'name':'read','inputSchema':{'type':'object'}}]}
 else: continue
 print(json.dumps({'jsonrpc':'2.0','id':q['id'],'result':r}),flush=True)
"#;
        let draft = McpDraft {
            project: project.clone(),
            instance_id: "fixture".into(),
            endpoint: McpEndpoint::Stdio {
                command: "python3".into(),
                args: vec!["-u".into(), "-c".into(), program.into()],
                command_reference: "fixture".into(),
            },
            tool_access_scope: vec!["read".into()],
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let owner = McpSessionOwner::new(gcx.clone(), std::sync::Arc::new(Materials::default()));
        let spec = action(project.clone(), "mcp_enabled_reload", "absent");
        assert_eq!(
            owner
                .reload(
                    &Revoke {
                        calls: Default::default()
                    },
                    &spec,
                    &draft
                )
                .await,
            Err(ResourceError::VersionMismatch)
        );
        assert!(gcx.integration_sessions.lock().await.is_empty());
        assert!(files::snapshot(&project, "integrations.d/mcp_fixture.yaml")
            .unwrap()
            .is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn operator_mcp_real_stdio_zero_connection_probe_reload_and_durable_revert() {
        use crate::ext::operator_service as files;
        use crate::buddy::resources::RecoveryDescriptor;
        use crate::integrations::mcp::{integr_mcp_common, session_mcp::SessionMCP};
        let temp = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: dunce::canonicalize(temp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let marker = temp.path().join("connections");
        let script = temp.path().join("fixture.py");
        std::fs::write(&script, r#"import json,sys
with open(sys.argv[1], 'a') as f: f.write('connected\n')
for line in sys.stdin:
    q=json.loads(line)
    m=q.get('method')
    if m=='initialize':
        assert q['params']['capabilities']=={}
        r={'protocolVersion':'2024-11-05','capabilities':{'tools':{},'resources':{},'prompts':{}},'serverInfo':{'name':'fixture','version':'1'}}
    elif m=='tools/list':
        r={'tools':[{'name':n,'inputSchema':{'type':'object'}} for n in ['read','write','forbidden']]}
    elif m=='notifications/initialized': continue
    elif m=='notifications/cancelled': continue
    else: raise AssertionError(m)
    print(json.dumps({'jsonrpc':'2.0','id':q['id'],'result':r}),flush=True)
"#).unwrap();
        let mut draft = McpDraft {
            project: project.clone(),
            instance_id: "fixture".into(),
            endpoint: McpEndpoint::Stdio {
                command: "python3".into(),
                args: vec![
                    "-u".into(),
                    script.to_string_lossy().into_owned(),
                    marker.to_string_lossy().into_owned(),
                ],
                command_reference: "local-fixture".into(),
            },
            tool_access_scope: vec!["read".into()],
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let materials = std::sync::Arc::new(Materials::default());
        let owner = McpSessionOwner::new(gcx.clone(), materials.clone());
        let probe = action(project.clone(), "mcp_capability_probe", "absent");
        assert!(crate::integrations::operator_probe::probe_mcp(
            gcx.clone(),
            &files::DenyResourceRights,
            &probe,
            &draft
        )
        .await
        .is_err());
        let stale = action(project.clone(), "mcp_capability_probe", "stale");
        assert!(crate::integrations::operator_probe::probe_mcp(
            gcx.clone(),
            &files::DenyResourceRights,
            &stale,
            &draft
        )
        .await
        .is_err());
        assert!(
            !marker.exists(),
            "unauthorized or stale actions must not create a transport"
        );
        assert_eq!(
            crate::integrations::operator_probe::probe_mcp(
                gcx.clone(),
                &grant(&probe, &draft),
                &probe,
                &draft
            )
            .await
            .unwrap(),
            vec!["read"]
        );
        assert!(gcx.integration_sessions.lock().await.is_empty());
        assert_eq!(
            files::snapshot(&project, "integrations.d/mcp_fixture.yaml").unwrap(),
            None
        );
        let first = action(project.clone(), "mcp_enabled_reload", "absent");
        let receipt = owner
            .reload(&grant(&first, &draft), &first, &draft)
            .await
            .unwrap();
        assert!(owner
            .verify(&first, &receipt.expected_post_version)
            .await
            .unwrap());
        let path = temp
            .path()
            .join(".refact/integrations.d/mcp_fixture.yaml")
            .to_string_lossy()
            .into_owned();
        let native = gcx
            .integration_sessions
            .lock()
            .await
            .get(&path)
            .unwrap()
            .clone();
        assert_eq!(
            native
                .lock()
                .await
                .as_any_mut()
                .downcast_mut::<SessionMCP>()
                .unwrap()
                .mcp_tools
                .iter()
                .map(|t| t.name.to_string())
                .collect::<Vec<_>>(),
            vec!["read"]
        );
        let tools = integr_mcp_common::mcp_integr_tools(
            Some(std::sync::Arc::downgrade(&gcx)),
            &path,
            &Default::default(),
            &Default::default(),
        )
        .await;
        assert_eq!(
            tools.len(),
            1,
            "native tool publication must use exact filtered session catalog"
        );
        let enabled = files::snapshot(&project, "integrations.d/mcp_fixture.yaml").unwrap();
        let config: serde_json::Value = serde_yaml::from_slice(enabled.as_ref().unwrap()).unwrap();
        assert_eq!(config["available"]["on_your_laptop"], true);
        assert_eq!(config["operator_tools"], serde_json::json!(["read"]));
        draft.tool_access_scope = vec!["write".into()];
        let second = action(
            project.clone(),
            "mcp_enabled_reload",
            &receipt.expected_post_version,
        );
        let second_receipt = owner
            .reload(&grant(&second, &draft), &second, &draft)
            .await
            .unwrap();
        assert!(
            native
                .lock()
                .await
                .as_any_mut()
                .downcast_mut::<SessionMCP>()
                .unwrap()
                .mcp_client
                .as_ref()
                .unwrap()
                .lock()
                .await
                .is_none(),
            "old published handles must be retired"
        );
        assert!(owner
            .verify(&second, &second_receipt.expected_post_version)
            .await
            .unwrap());
        drop(owner);
        let recovered_owner = McpSessionOwner::new(gcx.clone(), materials);
        recovered_owner
            .recover(
                &second,
                &RecoveryDescriptor {
                    previous_material_reference: second_receipt.previous_material_reference,
                    expected_post_version: second_receipt.expected_post_version,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            files::snapshot(&project, "integrations.d/mcp_fixture.yaml").unwrap(),
            enabled
        );
        assert!(recovered_owner
            .verify(&first, &receipt.expected_post_version)
            .await
            .unwrap());
        let restored = gcx
            .integration_sessions
            .lock()
            .await
            .get(&path)
            .unwrap()
            .clone();
        assert_eq!(
            restored
                .lock()
                .await
                .as_any_mut()
                .downcast_mut::<SessionMCP>()
                .unwrap()
                .mcp_tools[0]
                .name,
            "read"
        );
        recovered_owner
            .recover(
                &first,
                &RecoveryDescriptor {
                    previous_material_reference: receipt.previous_material_reference,
                    expected_post_version: receipt.expected_post_version,
                },
            )
            .await
            .unwrap();
        assert!(gcx.integration_sessions.lock().await.get(&path).is_none());
        assert!(files::snapshot(&project, "integrations.d/mcp_fixture.yaml")
            .unwrap()
            .is_none());
        assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 4);

        let materials = std::sync::Arc::new(Materials::default());
        let owner = McpSessionOwner::new(gcx.clone(), materials.clone());
        draft.tool_access_scope = vec!["read".into()];
        let first = action(project.clone(), "mcp_enabled_reload", "absent");
        let first_receipt = owner
            .reload(&grant(&first, &draft), &first, &draft)
            .await
            .unwrap();
        let retained = files::snapshot(&project, "integrations.d/mcp_fixture.yaml").unwrap();
        draft.tool_access_scope = vec!["write".into()];
        let second = action(
            project.clone(),
            "mcp_enabled_reload",
            &first_receipt.expected_post_version,
        );
        let second_receipt = owner
            .reload(&grant(&second, &draft), &second, &draft)
            .await
            .unwrap();
        let active = gcx.integration_sessions.lock().await.remove(&path).unwrap();
        crate::integrations::operator_probe::stop(&active).await;
        drop(owner);
        let restarted_gcx = crate::global_context::tests::make_test_gcx().await;
        let restarted = McpSessionOwner::new(restarted_gcx.clone(), materials);
        let descriptor = RecoveryDescriptor {
            previous_material_reference: second_receipt.previous_material_reference,
            expected_post_version: second_receipt.expected_post_version,
        };
        let before_recovery = files::snapshot(&project, "integrations.d/mcp_fixture.yaml").unwrap();
        let mut stale = descriptor.clone();
        stale.expected_post_version = "stale".into();
        assert!(restarted.recover(&second, &stale).await.is_err());
        assert_eq!(
            files::snapshot(&project, "integrations.d/mcp_fixture.yaml").unwrap(),
            before_recovery
        );
        assert_eq!(
            restarted.recover(&second, &descriptor).await.unwrap(),
            crate::buddy::resources::ResourceApplyState::DiskDoneReloadNeeded
        );
        assert_eq!(
            files::snapshot(&project, "integrations.d/mcp_fixture.yaml").unwrap(),
            retained
        );
        assert!(!restarted
            .verify(&first, &first_receipt.expected_post_version)
            .await
            .unwrap());
        assert!(restarted_gcx.integration_sessions.lock().await.is_empty());
        assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 6);
        assert_eq!(
            restarted
                .recover(
                    &first,
                    &RecoveryDescriptor {
                        previous_material_reference: first_receipt.previous_material_reference,
                        expected_post_version: first_receipt.expected_post_version,
                    }
                )
                .await
                .unwrap(),
            crate::buddy::resources::ResourceApplyState::Applied
        );
        assert!(restarted.verify(&first, "absent").await.unwrap());
        assert!(restarted_gcx.integration_sessions.lock().await.is_empty());
        assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 6);
    }

    #[test]
    fn runtime_config_is_enabled_restricted_and_disabled_probe_stays_inert() {
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: "/fixture".into(),
        };
        let draft = McpDraft {
            project,
            instance_id: "fixture".into(),
            endpoint: McpEndpoint::Http {
                url: "https://fixture.invalid/mcp".into(),
                credential_references: vec![],
            },
            tool_access_scope: vec!["read".into()],
        };
        let probe: serde_json::Value =
            serde_yaml::from_slice(&prepare_runtime_config(None, &draft, false).unwrap()).unwrap();
        assert_eq!(probe["operator_tools"], serde_json::json!(["read"]));
        assert_eq!(probe["available"]["on_your_laptop"], false);
        assert_eq!(probe["available"]["when_isolated"], false);
        assert_eq!(probe["sampling_policy"], "deny");
        assert_eq!(probe["reconnect_enabled"], false);

        let enabled: serde_json::Value =
            serde_yaml::from_slice(&prepare_runtime_config(None, &draft, true).unwrap()).unwrap();
        assert_eq!(enabled["operator_tools"], serde_json::json!(["read"]));
        assert_eq!(enabled["available"]["on_your_laptop"], true);
        assert_eq!(enabled["available"]["when_isolated"], false);
    }

    #[test]
    fn runtime_config_rejects_unmanaged_live_and_unresolved_credentials() {
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: "/fixture".into(),
        };
        let stdio = McpDraft {
            project: project.clone(),
            instance_id: "fixture".into(),
            endpoint: McpEndpoint::Stdio {
                command: "fixture".into(),
                args: vec![],
                command_reference: "fixture-command".into(),
            },
            tool_access_scope: vec!["read".into()],
        };
        let error = prepare_runtime_config(
            Some(b"command: old\navailable:\n  on_your_laptop: true\n  when_isolated: false\n"),
            &stdio,
            true,
        )
        .unwrap_err();
        assert!(
            matches!(error, ResourceError::ValidationFailed(reason) if reason.contains("disable existing MCP"))
        );

        let http = McpDraft {
            project,
            instance_id: "fixture".into(),
            endpoint: McpEndpoint::Http {
                url: "https://fixture.invalid/mcp".into(),
                credential_references: vec!["private-token".into()],
            },
            tool_access_scope: vec!["read".into()],
        };
        assert!(matches!(
            prepare_runtime_config(None, &http, true),
            Err(ResourceError::ValidationFailed(reason)) if reason.contains("credential")
        ));
    }

    #[test]
    fn stdio_native_command_roundtrips_exact_argv() {
        let argv = vec![
            "fixture".to_string(),
            "two words".into(),
            "'quoted'".into(),
            "".into(),
            "$literal;".into(),
        ];
        let draft = McpDraft {
            project: refact_buddy_core::operator_types::ProjectIdentity {
                project_id: "fixture".into(),
                canonical_root: "/fixture".into(),
            },
            instance_id: "fixture".into(),
            endpoint: McpEndpoint::Stdio {
                command: argv[0].clone(),
                args: argv[1..].to_vec(),
                command_reference: "fixture-command".into(),
            },
            tool_access_scope: vec!["read".into()],
        };
        let value: serde_json::Value =
            serde_yaml::from_slice(&prepare_mcp(None, &draft).unwrap()).unwrap();
        assert_eq!(
            shell_words::split(value["command"].as_str().unwrap()).unwrap(),
            argv
        );
        assert!(value.get("args").is_none());
    }

    #[test]
    fn mcp_owner_disk_effect_verification_and_recovery_are_isolated() {
        use crate::ext::operator_service::{replace, snapshot, version};
        let temp = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: dunce::canonicalize(temp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let draft = McpDraft {
            project: project.clone(),
            instance_id: "fixture".into(),
            endpoint: McpEndpoint::Stdio {
                command: "fixture-never-executed".into(),
                args: vec!["--fixture".into()],
                command_reference: "registered-fixture".into(),
            },
            tool_access_scope: vec!["read".into()],
        };
        let next = Some(prepare_mcp(None, &draft).unwrap());
        let target = "integrations.d/mcp_fixture.yaml";
        assert_eq!(snapshot(&project, target).unwrap(), None);
        replace(&project, target, "absent", &next).unwrap();
        assert_eq!(snapshot(&project, target).unwrap(), next);
        let config: serde_json::Value = serde_yaml::from_slice(next.as_ref().unwrap()).unwrap();
        assert_eq!(config["available"]["on_your_laptop"], false);
        assert_eq!(config["available"]["when_isolated"], false);
        assert_eq!(
            replace(&project, target, "foreign-version", &None),
            Err(ResourceError::VersionMismatch)
        );
        replace(&project, target, &version(&next), &None).unwrap();
        assert_eq!(snapshot(&project, target).unwrap(), None);
    }
    #[test]
    fn local_prepare_preserves_unknown_and_never_enables_or_embeds_credentials() {
        let draft = McpDraft {
            project: refact_buddy_core::operator_types::ProjectIdentity {
                project_id: "fixture".into(),
                canonical_root: "/fixture".into(),
            },
            instance_id: "fixture".into(),
            endpoint: McpEndpoint::Http {
                url: "https://fixture.invalid/mcp".into(),
                credential_references: vec!["opaque-ref".into()],
            },
            tool_access_scope: vec!["read".into()],
        };
        let old = b"available:\n  on_your_laptop: false\n  when_isolated: false\nunknown:\n  retained: true\n";
        let bytes = prepare_mcp(Some(old), &draft).unwrap();
        let parsed: serde_json::Value = serde_yaml::from_slice(&bytes).unwrap();
        assert_eq!(parsed["unknown"]["retained"], true);
        assert_eq!(parsed["available"]["on_your_laptop"], false);
        assert!(!String::from_utf8(bytes).unwrap().contains("opaque-ref"));
        let temp = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: dunce::canonicalize(temp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let original = Some(old.to_vec());
        let after = Some(prepare_mcp(Some(old), &draft).unwrap());
        let path = "integrations.d/mcp_fixture.yaml";
        let owner = crate::ext::operator_service::replace;
        owner(&project, path, "absent", &original).unwrap();
        owner(
            &project,
            path,
            &crate::ext::operator_service::version(&original),
            &after,
        )
        .unwrap();
        assert_eq!(
            crate::ext::operator_service::snapshot(&project, path).unwrap(),
            after
        );
        assert_eq!(
            owner(&project, path, "wrong-version", &original),
            Err(ResourceError::VersionMismatch)
        );
        owner(
            &project,
            path,
            &crate::ext::operator_service::version(&after),
            &original,
        )
        .unwrap();
        assert_eq!(
            crate::ext::operator_service::snapshot(&project, path).unwrap(),
            original
        );
        assert!(prepare_mcp(Some(b"url: https://old.invalid\n"), &draft).is_err());
    }
}
