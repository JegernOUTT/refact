use std::path::Path;

use axum::http::HeaderMap;
use refact_buddy_core::operator_types::{
    NativeSourceIdentity, NativeSourceKind, ProjectIdentity, WorkerIdentity,
};
use sha2::{Digest, Sha256};

use crate::app_state::AppState;
use super::outcome_runtime::OutcomeRuntime;
use super::operator_store::OperatorStoreError;

#[derive(Debug)]
pub(crate) enum OperatorRuntimeOpenError {
    ProjectIdentity(String),
    Store(OperatorStoreError),
    Task(String),
}

impl std::fmt::Display for OperatorRuntimeOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProjectIdentity(error) | Self::Task(error) => f.write_str(error),
            Self::Store(error) => error.fmt(f),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct OperatorFeatures {
    pub storage: bool,
    pub actions: bool,
    pub supervision: bool,
    pub improvement: bool,
    pub editorial: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum OperatorCapability {
    Storage,
    Actions,
    Supervision,
    Improvement,
    Editorial,
}

impl OperatorFeatures {
    pub fn from_settings(settings: &refact_buddy_core::settings::BuddySettings) -> Self {
        if !settings.operator_enabled {
            return Self::default();
        }
        Self {
            storage: true,
            actions: true,
            supervision: true,
            improvement: true,
            editorial: true,
        }
    }

    pub fn allows(self, capability: OperatorCapability) -> bool {
        self.storage
            && match capability {
                OperatorCapability::Storage => true,
                OperatorCapability::Actions => self.actions,
                OperatorCapability::Supervision => self.supervision,
                OperatorCapability::Improvement => self.improvement,
                OperatorCapability::Editorial => self.editorial,
            }
    }
}

/// Identity for the project's operator store, hashed exactly like the daemon
/// registry so both processes address the same durable project.
pub fn project_identity(project_root: &Path) -> Result<ProjectIdentity, String> {
    let canonical = std::fs::canonicalize(project_root).map_err(|error| error.to_string())?;
    let canonical = dunce::simplified(&canonical).to_path_buf();
    if canonical != project_root {
        return Err(format!(
            "project root {project_root:?} is not its own canonical path {canonical:?}"
        ));
    }
    let canonical_root = canonical.to_string_lossy().into_owned();
    let mut hasher = Sha256::new();
    hasher.update(canonical_root.as_bytes());
    Ok(ProjectIdentity {
        project_id: hex::encode(hasher.finalize()),
        canonical_root,
    })
}

pub fn engine_worker_identity() -> WorkerIdentity {
    let epoch = chrono::Utc::now().timestamp_millis().max(0) as u64;
    WorkerIdentity {
        worker_id: "engine-buddy-actor".to_string(),
        engine_instance_id: format!("engine:{}:{}", std::process::id(), epoch),
        epoch,
    }
}

/// Open the durable operator store for the live service. Every failure is an
/// error the caller reports as `unavailable`; nothing here removes or rewrites data.
pub(crate) fn open_project_runtime(
    project_root: &Path,
) -> Result<OutcomeRuntime, OperatorRuntimeOpenError> {
    let project =
        project_identity(project_root).map_err(OperatorRuntimeOpenError::ProjectIdentity)?;
    OutcomeRuntime::open(project_root, project, engine_worker_identity())
        .map_err(OperatorRuntimeOpenError::Store)
}

pub(crate) async fn open_project_runtime_async(
    project_root: std::path::PathBuf,
) -> Result<OutcomeRuntime, OperatorRuntimeOpenError> {
    tokio::task::spawn_blocking(move || open_project_runtime(&project_root))
        .await
        .map_err(|_| {
            OperatorRuntimeOpenError::Task("operator store open task failed".to_string())
        })?
}

pub async fn runtime(
    app: &AppState,
    capability: OperatorCapability,
) -> Result<OutcomeRuntime, String> {
    let (runtime, root) = {
        let buddy = app.buddy.buddy.lock().await;
        let service = buddy.as_ref().ok_or("Buddy service unavailable")?;
        if !service.operator_features.allows(capability) {
            return Err(format!("Buddy operator {capability:?} rollout is disabled"));
        }
        (
            service
                .outcome_runtime
                .clone()
                .ok_or("Buddy operator store unavailable")?,
            service.project_root.clone(),
        )
    };
    let roots = crate::files_correction::get_unscoped_project_dirs(app.gcx.clone()).await;
    if roots.len() != 1
        || roots[0] != root
        || runtime.project().canonical_root != root.to_string_lossy()
    {
        return Err("Buddy operator project identity changed".into());
    }
    Ok(runtime)
}

pub fn authenticated_operator_request(
    app: &AppState,
    headers: &HeaderMap,
) -> Result<NativeSourceIdentity, String> {
    let expected = app
        .gcx
        .cmdline
        .daemon_auth_token
        .as_deref()
        .filter(|token| !token.is_empty())
        .ok_or("Authenticated operator transport is unavailable")?;
    let actual = crate::daemon::auth::bearer_token_from_headers(headers)
        .ok_or("Operator authorization required")?;
    if !crate::daemon::auth::token_matches(actual, expected) {
        return Err("Operator authorization required".into());
    }
    Ok(NativeSourceIdentity {
        source_kind: NativeSourceKind::External,
        source_id: "authenticated-user".into(),
        source_version: "operator-http-v1".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_flags_never_grant_automation() {
        let disabled = OperatorFeatures::default();
        for capability in [
            OperatorCapability::Storage,
            OperatorCapability::Actions,
            OperatorCapability::Supervision,
            OperatorCapability::Improvement,
            OperatorCapability::Editorial,
        ] {
            assert!(!disabled.allows(capability));
        }
        let storage_only = OperatorFeatures {
            storage: true,
            ..Default::default()
        };
        assert!(storage_only.allows(OperatorCapability::Storage));
        assert!(!storage_only.allows(OperatorCapability::Actions));
        assert!(!OperatorFeatures {
            actions: true,
            ..Default::default()
        }
        .allows(OperatorCapability::Actions));
    }

    #[test]
    fn default_settings_open_every_capability_and_opt_out_closes_them() {
        let mut settings = refact_buddy_core::settings::BuddySettings::default();
        let enabled = OperatorFeatures::from_settings(&settings);
        assert!(enabled.storage);
        assert!(enabled.actions);
        assert!(enabled.supervision);
        assert!(enabled.improvement);
        assert!(enabled.editorial);
        for capability in [
            OperatorCapability::Storage,
            OperatorCapability::Actions,
            OperatorCapability::Supervision,
            OperatorCapability::Improvement,
            OperatorCapability::Editorial,
        ] {
            assert!(enabled.allows(capability));
        }
        settings.operator_enabled = false;
        let disabled = OperatorFeatures::from_settings(&settings);
        assert!(!disabled.storage);
        assert!(!disabled.actions);
        assert!(!disabled.supervision);
        assert!(!disabled.improvement);
        assert!(!disabled.editorial);
    }

    #[tokio::test]
    async fn missing_or_wrong_transport_never_becomes_a_user_grant() {
        let root = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.path().join("cache"),
            root.path().join("config"),
        )
        .await;
        std::sync::Arc::get_mut(&mut gcx)
            .unwrap()
            .cmdline
            .daemon_auth_token = Some("fixture-secret".into());
        let app = AppState::from_gcx(gcx).await;
        let mut headers = HeaderMap::new();
        assert!(authenticated_operator_request(&app, &headers).is_err());
        headers.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(authenticated_operator_request(&app, &headers).is_err());
        headers.insert("authorization", "Bearer fixture-secret".parse().unwrap());
        let origin = authenticated_operator_request(&app, &headers).unwrap();
        assert_eq!(origin.source_id, "authenticated-user");
        assert!(!serde_json::to_string(&origin)
            .unwrap()
            .contains("fixture-secret"));
    }
}
