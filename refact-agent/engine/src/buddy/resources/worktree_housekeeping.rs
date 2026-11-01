//! Explicit, exact worktree archiving. This is NOT deletion or disk-space reclamation.
// External writers are not frozen; immediate reference/native identity rechecks fail closed,
// while intact quarantine preserves bytes even if an external editor races a move.
use std::path::PathBuf;
#[cfg(test)]
use std::path::Path;
use std::sync::Arc;

use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    AsyncResourceAdapter, DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};
use crate::app_state::AppState;
use crate::buddy::outcome_runtime::OutcomeRuntime;
use crate::worktrees::{service::WorktreeService, types::WorktreeArchiveIdentity};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeHousekeepingOperation {
    Archive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeHousekeepingDraft {
    pub operation: WorktreeHousekeepingOperation,
    pub project: ProjectIdentity,
    pub worktree_id: String,
}

pub struct WorktreeHousekeepingAdapter {
    app: AppState,
    runtime: OutcomeRuntime,
    worktrees: WorktreeService,
    drafts: Arc<dyn DraftRegistry<WorktreeHousekeepingDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
    gate: tokio::sync::Mutex<()>,
}

impl WorktreeHousekeepingAdapter {
    pub fn new(
        app: AppState,
        runtime: OutcomeRuntime,
        worktrees: WorktreeService,
        drafts: Arc<dyn DraftRegistry<WorktreeHousekeepingDraft>>,
        materials: Arc<dyn RecoveryMaterialStore>,
    ) -> Result<Self, ResourceError> {
        if worktrees.source_workspace_root().to_string_lossy() != runtime.project().canonical_root {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        Ok(Self {
            app,
            runtime,
            worktrees,
            drafts,
            materials,
            gate: tokio::sync::Mutex::new(()),
        })
    }

    fn validate(&self, spec: &ActionSpec) -> Result<WorktreeHousekeepingDraft, ResourceError> {
        if spec.project != self.runtime.project() {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != self.adapter_id()
            || spec.kind != "archive"
            || spec.resource.resource_kind != "registered_worktree"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != spec.project || draft.worktree_id != spec.resource.resource_id {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        Ok(draft)
    }

    pub async fn version(&self, id: &str) -> Result<String, ResourceError> {
        Ok(self
            .worktrees
            .preview_worktree_archive(id)
            .await
            .map_err(ResourceError::ValidationFailed)?
            .version)
    }

    async fn prepare(&self, spec: &ActionSpec) -> Result<WorktreeArchiveIdentity, ResourceError> {
        let draft = self.validate(spec)?;
        let identity = self
            .worktrees
            .preview_worktree_archive(&draft.worktree_id)
            .await
            .map_err(ResourceError::ValidationFailed)?;
        if identity.version != spec.expected_version || spec.resource.version != identity.version {
            return Err(ResourceError::VersionMismatch);
        }
        let id = identity.record.meta.id.clone();
        let root = identity.record.meta.root.to_string_lossy().into_owned();
        let branch = identity.record.meta.branch.clone();
        if self
            .runtime
            .with_store(move |store| store.worktree_is_referenced(&id, &root, branch.as_deref()))
            .await
            .map_err(|e| ResourceError::Io(e.to_string()))?
        {
            return Err(protected());
        }
        self.native_references(&identity).await?;
        Ok(identity)
    }

    async fn native_references(
        &self,
        identity: &WorktreeArchiveIdentity,
    ) -> Result<(), ResourceError> {
        let sessions = self.app.chat.sessions.read().await;
        for session in sessions.values() {
            let session = session.try_lock().map_err(|_| protected())?;
            if session.thread.worktree.as_ref().is_some_and(|meta| {
                meta.id == identity.record.meta.id
                    || meta.root == identity.record.meta.root
                    || meta
                        .branch
                        .as_ref()
                        .is_some_and(|branch| identity.record.meta.branch.as_ref() == Some(branch))
            }) {
                return Err(protected());
            }
        }
        drop(sessions);
        // Native trajectory files may still hold references after the registry reference was lost.
        // Fail closed on malformed/oversized/unreadable history, including pinned and goal metadata.
        let mut dirs =
            crate::chat::trajectories::get_all_trajectories_dirs(self.app.gcx.clone()).await;
        for name in ["trajectories", "trajectory_archive", "tasks"] {
            dirs.push(
                self.worktrees
                    .source_workspace_root()
                    .join(".refact")
                    .join(name),
            );
        }
        let identity = identity.clone();
        tokio::task::spawn_blocking(move || scan_native_references(dirs, &identity))
            .await
            .map_err(|e| ResourceError::Io(e.to_string()))?
    }

    fn recovery_identity(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<WorktreeArchiveIdentity, ResourceError> {
        self.validate(spec)?;
        let (project, identity): (ProjectIdentity, WorktreeArchiveIdentity) =
            serde_json::from_slice(&self.materials.get(&recovery.previous_material_reference)?)
                .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
        if project != spec.project
            || identity.record.meta.id != spec.resource.resource_id
            || identity.version != spec.expected_version
            || archive_version(&identity) != recovery.expected_post_version
        {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        Ok(identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::operator_types::*;
    use crate::worktrees::types::{CreateWorktreeRequest, WorktreeReference};

    struct Draft(WorktreeHousekeepingDraft);
    impl DraftRegistry<WorktreeHousekeepingDraft> for Draft {
        fn get(&self, key: &str) -> Result<WorktreeHousekeepingDraft, ResourceError> {
            if key != "scratch-draft" {
                return Err(ResourceError::ParameterReferenceMissing);
            }
            Ok(self.0.clone())
        }
    }
    #[derive(Default)]
    struct Materials(std::sync::Mutex<Vec<Vec<u8>>>);
    impl RecoveryMaterialStore for Materials {
        fn put(&self, bytes: &[u8]) -> Result<String, ResourceError> {
            let mut all = self.0.lock().unwrap();
            all.push(bytes.to_vec());
            Ok((all.len() - 1).to_string())
        }
        fn get(&self, key: &str) -> Result<Vec<u8>, ResourceError> {
            self.0
                .lock()
                .unwrap()
                .get(
                    key.parse::<usize>()
                        .map_err(|_| ResourceError::MaterialUnavailable)?,
                )
                .cloned()
                .ok_or(ResourceError::MaterialUnavailable)
        }
    }
    fn git(root: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn adapter_scratch_archive_verify_recover_protects_and_never_overwrites() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        git(&root, &["config", "user.name", "Scratch"]);
        git(&root, &["config", "user.email", "scratch@example.invalid"]);
        std::fs::write(root.join("tracked"), "retained\n").unwrap();
        git(&root, &["add", "tracked"]);
        git(&root, &["commit", "-m", "initial"]);
        let project = ProjectIdentity {
            project_id: "scratch-archive".into(),
            canonical_root: dunce::canonicalize(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let owner = WorktreeService::new(scratch.path().join("cache"), root.clone()).unwrap();
        let created = owner
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/housekeeping".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let checkout = created.worktree.meta.root.clone();
        let id = created.worktree.meta.id.clone();
        let runtime = OutcomeRuntime::open(
            &root,
            project.clone(),
            WorkerIdentity {
                worker_id: "scratch".into(),
                engine_instance_id: "scratch-engine".into(),
                epoch: 1,
            },
        )
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            scratch.path().join("gcx-cache"),
            scratch.path().join("config"),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        let drafts = Arc::new(Draft(WorktreeHousekeepingDraft {
            operation: WorktreeHousekeepingOperation::Archive,
            project: project.clone(),
            worktree_id: id.clone(),
        }));
        let materials = Arc::new(Materials::default());
        let adapter = WorktreeHousekeepingAdapter::new(
            app.clone(),
            runtime.clone(),
            owner.clone(),
            drafts,
            materials.clone(),
        )
        .unwrap();
        let version = adapter.version(&id).await.unwrap();
        let mut spec = ActionSpec {
            schema_version: 1,
            action_id: "archive-scratch".into(),
            kind: "archive".into(),
            adapter: "worktree_housekeeping".into(),
            project,
            resource: ResourceRef {
                resource_kind: "registered_worktree".into(),
                resource_id: id.clone(),
                version: version.clone(),
            },
            expected_version: version,
            parameter_ref: Some("scratch-draft".into()),
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
        };
        let before = std::fs::read(owner.registry_path()).unwrap();
        assert!(adapter
            .preview(&spec)
            .await
            .unwrap()
            .diff
            .unwrap()
            .contains("NO deletion"));
        assert_eq!(before, std::fs::read(owner.registry_path()).unwrap());
        assert!(materials.0.lock().unwrap().is_empty());
        assert!(!scratch.path().join("cache/worktree-archive").exists());
        let mut stale = spec.clone();
        stale.expected_version = "stale".into();
        stale.resource.version = "stale".into();
        assert_eq!(
            adapter.apply(&stale).await.unwrap_err(),
            ResourceError::VersionMismatch
        );
        assert_eq!(before, std::fs::read(owner.registry_path()).unwrap());
        assert!(materials.0.lock().unwrap().is_empty());
        std::fs::write(checkout.join("dirty"), "never delete").unwrap();
        assert!(adapter.apply(&spec).await.is_err());
        assert_eq!(
            std::fs::read_to_string(checkout.join("dirty")).unwrap(),
            "never delete"
        );
        std::fs::remove_file(checkout.join("dirty")).unwrap();
        let reference = WorktreeReference {
            kind: "chat".into(),
            chat_id: Some("native-pin".into()),
            ..Default::default()
        };
        owner.add_reference(&id, reference.clone()).await.unwrap();
        assert!(adapter.apply(&spec).await.is_err());
        owner.remove_reference(&id, &reference).await.unwrap();
        spec.expected_version = adapter.version(&id).await.unwrap();
        spec.resource.version = spec.expected_version.clone();
        let native_dir = root.join(".refact/trajectories");
        std::fs::create_dir_all(&native_dir).unwrap();
        std::fs::write(
            native_dir.join("pinned.json"),
            serde_json::to_vec(&serde_json::json!({"pinned":true,"worktree":{"id":id}})).unwrap(),
        )
        .unwrap();
        assert!(adapter.apply(&spec).await.is_err());
        std::fs::remove_file(native_dir.join("pinned.json")).unwrap();
        let mut session = crate::chat::types::ChatSession::new("scratch-active".into());
        session.thread.worktree = Some(created.worktree.meta.clone());
        app.chat.sessions.write().await.insert(
            "scratch-active".into(),
            Arc::new(tokio::sync::Mutex::new(session)),
        );
        assert!(adapter.apply(&spec).await.is_err());
        app.chat.sessions.write().await.remove("scratch-active");
        // Actual adapter call, actual Git move, exact owner verification and actual recovery.
        let receipt = adapter.apply(&spec).await.unwrap();
        assert!(!checkout.exists());
        assert!(adapter
            .verify(&spec, &receipt.expected_post_version)
            .await
            .unwrap());
        assert!(adapter.apply(&spec).await.is_err());
        git(&root, &["rev-parse", "refs/heads/refact/chat/housekeeping"]);
        let recovery = RecoveryDescriptor {
            previous_material_reference: receipt.previous_material_reference,
            expected_post_version: receipt.expected_post_version,
        };
        std::fs::create_dir(&checkout).unwrap();
        std::fs::write(checkout.join("unrelated"), "preserve").unwrap();
        assert!(adapter.recover(&spec, &recovery).await.is_err());
        assert_eq!(
            std::fs::read_to_string(checkout.join("unrelated")).unwrap(),
            "preserve"
        );
        std::fs::remove_file(checkout.join("unrelated")).unwrap();
        std::fs::remove_dir(&checkout).unwrap();
        assert_eq!(
            adapter.recover(&spec, &recovery).await.unwrap(),
            ResourceApplyState::Applied
        );
        assert_eq!(
            std::fs::read_to_string(checkout.join("tracked")).unwrap(),
            "retained\n"
        );
        assert!(owner.get_worktree(&id).await.is_ok());
        assert!(adapter.recover(&spec, &recovery).await.is_err());
        let (_, identity): (ProjectIdentity, WorktreeArchiveIdentity) = serde_json::from_slice(
            &materials
                .get(&recovery.previous_material_reference)
                .unwrap(),
        )
        .unwrap();
        assert!(scratch
            .path()
            .join("cache/worktree-archive")
            .join(owner.project_hash())
            .join(identity.version)
            .join("identity.json")
            .is_file());
    }
}

fn protected() -> ResourceError {
    ResourceError::ValidationFailed(
        "worktree is active, pinned, or referenced by native/Buddy work or evidence".into(),
    )
}

fn archive_version(identity: &WorktreeArchiveIdentity) -> String {
    format!("archived:{}", identity.version)
}

fn references(value: &Value, identity: &WorktreeArchiveIdentity) -> bool {
    crate::buddy::operator_store::contains_worktree_reference(
        value,
        &identity.record.meta.id,
        &identity.record.meta.root.to_string_lossy(),
        identity.record.meta.branch.as_deref(),
    )
}

fn scan_native_references(
    dirs: Vec<PathBuf>,
    identity: &WorktreeArchiveIdentity,
) -> Result<(), ResourceError> {
    use std::io::Read;

    let mut pending = dirs;
    let mut visited = std::collections::HashSet::new();
    let mut files = 0usize;
    let mut bytes = 0u64;
    while let Some(dir) = pending.pop() {
        if !visited.insert(dir.clone()) {
            continue;
        }
        let meta = match std::fs::symlink_metadata(&dir) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if meta.file_type().is_symlink() {
            return Err(ResourceError::SymlinkNotAllowed);
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            files += 1;
            if files > 10_000 {
                return Err(protected());
            }
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(ResourceError::SymlinkNotAllowed);
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if !kind.is_file() {
                return Err(protected());
            }
            let remaining = (64_u64 * 1024 * 1024).saturating_sub(bytes);
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = options.open(entry.path())?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.len() > remaining {
                return Err(protected());
            }
            let mut contents = Vec::new();
            file.take(remaining + 1).read_to_end(&mut contents)?;
            bytes = bytes.saturating_add(contents.len() as u64);
            if bytes > 64 * 1024 * 1024 {
                return Err(protected());
            }
            let value: Value = serde_json::from_slice(&contents)
                .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
            if references(&value, identity) {
                return Err(protected());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod adapter_regression_tests {
    use super::*;
    use refact_buddy_core::operator_types::*;
    struct Draft(WorktreeHousekeepingDraft);
    impl DraftRegistry<WorktreeHousekeepingDraft> for Draft {
        fn get(&self, _: &str) -> Result<WorktreeHousekeepingDraft, ResourceError> {
            Ok(self.0.clone())
        }
    }
    #[derive(Default)]
    struct Materials(std::sync::Mutex<Vec<Vec<u8>>>);
    impl RecoveryMaterialStore for Materials {
        fn put(&self, bytes: &[u8]) -> Result<String, ResourceError> {
            let mut values = self.0.lock().unwrap();
            values.push(bytes.to_vec());
            Ok((values.len() - 1).to_string())
        }
        fn get(&self, key: &str) -> Result<Vec<u8>, ResourceError> {
            self.0
                .lock()
                .unwrap()
                .get(
                    key.parse::<usize>()
                        .map_err(|_| ResourceError::MaterialUnavailable)?,
                )
                .cloned()
                .ok_or(ResourceError::MaterialUnavailable)
        }
    }
    fn git(root: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    #[tokio::test]
    async fn actual_adapter_quarantine_apply_verify_recover_scratch() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.name", "Scratch"]);
        git(&root, &["config", "user.email", "scratch@example.invalid"]);
        std::fs::write(root.join("tracked"), "evidence").unwrap();
        git(&root, &["add", "tracked"]);
        git(&root, &["commit", "-m", "initial"]);
        let root = dunce::canonicalize(&root).unwrap();
        let owner = WorktreeService::new(temp.path().join("cache"), root.clone()).unwrap();
        let created = owner
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/archive-adapter".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let project = ProjectIdentity {
            project_id: "scratch".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        };
        let runtime = OutcomeRuntime::open(
            &root,
            project.clone(),
            WorkerIdentity {
                worker_id: "w".into(),
                engine_instance_id: "e".into(),
                epoch: 1,
            },
        )
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        let adapter = WorktreeHousekeepingAdapter::new(
            app.clone(),
            runtime.clone(),
            owner.clone(),
            Arc::new(Draft(WorktreeHousekeepingDraft {
                operation: WorktreeHousekeepingOperation::Archive,
                project: project.clone(),
                worktree_id: created.worktree.meta.id.clone(),
            })),
            Arc::new(Materials::default()),
        )
        .unwrap();
        let version = adapter.version(&created.worktree.meta.id).await.unwrap();
        let spec = ActionSpec {
            schema_version: 1,
            action_id: "archive".into(),
            kind: "archive".into(),
            adapter: "worktree_housekeeping".into(),
            project,
            resource: ResourceRef {
                resource_kind: "registered_worktree".into(),
                resource_id: created.worktree.meta.id.clone(),
                version: version.clone(),
            },
            expected_version: version,
            parameter_ref: Some("draft".into()),
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
        };
        let before = std::fs::read(owner.registry_path()).unwrap();
        assert!(adapter
            .preview(&spec)
            .await
            .unwrap()
            .diff
            .unwrap()
            .contains("NO deletion"));
        assert_eq!(before, std::fs::read(owner.registry_path()).unwrap());
        let mut stale = spec.clone();
        stale.expected_version = "stale".into();
        assert!(matches!(
            adapter.apply(&stale).await,
            Err(ResourceError::VersionMismatch)
        ));
        assert_eq!(before, std::fs::read(owner.registry_path()).unwrap());
        let native = root.join(".refact/trajectories");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(
            native.join("pinned.json"),
            serde_json::to_vec(
                &serde_json::json!({"pinned":true,"worktree":created.worktree.meta}),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(adapter.preview(&spec).await.is_err());
        std::fs::remove_file(native.join("pinned.json")).unwrap();
        for locator in [
            created
                .worktree
                .meta
                .root
                .join("tracked")
                .to_string_lossy()
                .into_owned(),
            format!("file://{}/tracked", created.worktree.meta.root.display()),
            format!(
                "Retained evidence: `{}/tracked`",
                created.worktree.meta.root.display()
            ),
        ] {
            std::fs::write(
                native.join("file-reference.json"),
                serde_json::to_vec(&serde_json::json!({"evidence": locator})).unwrap(),
            )
            .unwrap();
            assert!(adapter.preview(&spec).await.is_err());
            assert_eq!(before, std::fs::read(owner.registry_path()).unwrap());
        }
        std::fs::remove_file(native.join("file-reference.json")).unwrap();
        std::fs::write(
            native.join("sibling-reference.json"),
            serde_json::to_vec(&serde_json::json!({
                "evidence": format!("{}-sibling/tracked", created.worktree.meta.root.display())
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(adapter.preview(&spec).await.is_ok());
        std::fs::remove_file(native.join("sibling-reference.json")).unwrap();
        let session = Arc::new(tokio::sync::Mutex::new(
            crate::chat::types::ChatSession::new("active".into()),
        ));
        session.lock().await.thread.worktree = Some(created.worktree.meta.clone());
        app.chat
            .sessions
            .write()
            .await
            .insert("active".into(), session);
        assert!(adapter.preview(&spec).await.is_err());
        app.chat.sessions.write().await.remove("active");
        std::fs::write(created.worktree.meta.root.join("dirty"), "preserve").unwrap();
        assert!(adapter.apply(&spec).await.is_err());
        std::fs::remove_file(created.worktree.meta.root.join("dirty")).unwrap();
        let receipt = adapter.apply(&spec).await.unwrap();
        assert!(adapter
            .verify(&spec, &receipt.expected_post_version)
            .await
            .unwrap());
        assert!(adapter.apply(&spec).await.is_err());
        let recovery = RecoveryDescriptor {
            previous_material_reference: receipt.previous_material_reference,
            expected_post_version: receipt.expected_post_version,
        };
        std::fs::create_dir(&created.worktree.meta.root).unwrap();
        std::fs::write(created.worktree.meta.root.join("other"), "preserve").unwrap();
        assert!(adapter.recover(&spec, &recovery).await.is_err());
        assert_eq!(
            std::fs::read_to_string(created.worktree.meta.root.join("other")).unwrap(),
            "preserve"
        );
        std::fs::remove_file(created.worktree.meta.root.join("other")).unwrap();
        std::fs::remove_dir(&created.worktree.meta.root).unwrap();
        assert_eq!(
            adapter.recover(&spec, &recovery).await.unwrap(),
            ResourceApplyState::Applied
        );
        assert_eq!(
            std::fs::read_to_string(created.worktree.meta.root.join("tracked")).unwrap(),
            "evidence"
        );
        assert!(adapter.recover(&spec, &recovery).await.is_err());
        assert!(
            crate::worktrees::git::branch_exists(&root, "refact/chat/archive-adapter").unwrap()
        );
        let evidence = serde_json::json!({"schema_version":1,"project":spec.project,"worktree_id":created.worktree.meta.id});
        runtime
            .with_store(move |store| store.put_document("candidate", "protected", &evidence, None))
            .await
            .unwrap();
        assert!(
            adapter.preview(&spec).await.is_err(),
            "durable candidate evidence protects recovered checkout"
        );
    }
}

#[async_trait::async_trait]
impl AsyncResourceAdapter for WorktreeHousekeepingAdapter {
    fn adapter_id(&self) -> &str {
        "worktree_housekeeping"
    }
    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities {
            preview: true,
            apply: true,
            verify: true,
            recover: true,
        }
    }

    async fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError> {
        let identity = self.prepare(spec).await?;
        Ok(ResourcePreview { resource_id: identity.record.meta.id.clone(), expected_version: identity.version.clone(),
            proposed_version: archive_version(&identity), reload_needed: false,
            diff: Some(format!("Archive exactly registered checkout {} at {} (branch {:?}, HEAD {}). Move intact into owner quarantine and remove its active registry entry. Retain branch, committed/untracked/ignored files and recovery evidence. NO deletion, pruning of other worktrees, merge, or disk-space reclamation.", identity.record.meta.id, identity.record.meta.root.display(), identity.record.meta.branch, identity.head)) })
    }

    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let _gate = self.gate.lock().await;
        crate::chat::trajectory_archive::with_trajectory_mutation_lock(async {
            let identity = self.prepare(spec).await?;
            let sessions = self.app.chat.sessions.read().await;
            // Keep every loaded session stable until the owner finishes; fail closed on contention.
            let mut guards = Vec::new();
            for session in sessions.values() {
                let session = session.try_lock().map_err(|_| protected())?;
                if session.thread.worktree.as_ref().is_some_and(|meta| {
                    meta.id == identity.record.meta.id
                        || meta.root == identity.record.meta.root
                        || meta.branch == identity.record.meta.branch
                }) {
                    return Err(protected());
                }
                guards.push(session);
            }
            let previous_material_reference = self.materials.put(
                &serde_json::to_vec(&(spec.project.clone(), identity.clone()))
                    .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?,
            )?;
            let worktrees = self.worktrees.clone();
            let expected_post_version = archive_version(&identity);
            let id = identity.record.meta.id.clone();
            let root = identity.record.meta.root.to_string_lossy().into_owned();
            let branch = identity.record.meta.branch.clone();
            if self
                .runtime
                .with_store(move |store| {
                    store.worktree_is_referenced(&id, &root, branch.as_deref())
                })
                .await
                .map_err(|e| ResourceError::Io(e.to_string()))?
            {
                return Err(protected());
            }
            // Native owner rechecks exact registry and cleanliness under its mutation locks.
            // Never hold the Buddy SQLite connection/mutex across async native service work.
            worktrees
                .archive_worktree_checked(identity)
                .await
                .map_err(ResourceError::ValidationFailed)?;
            Ok(ResourceApplyReceipt {
                resource_id: spec.resource.resource_id.clone(),
                previous_material_reference,
                expected_post_version,
                state: ResourceApplyState::Applied,
            })
        })
        .await
    }

    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        self.validate(spec)?;
        if post == format!("archived:{}", spec.expected_version) {
            // Identity is durable native evidence; never infer archive success merely from a
            // missing checkout.
            return self
                .worktrees
                .verify_worktree_archive_version(&spec.resource.resource_id, &spec.expected_version)
                .await
                .map_err(ResourceError::ValidationFailed);
        }
        if post == spec.expected_version {
            // Recovery returns the active checkout to the exact reviewed native version. The
            // recovery service already required archived identity before it invoked recover;
            // this is the post-recovery check, not evidence that a pre-archive checkout applied.
            return Ok(self.version(&spec.resource.resource_id).await? == post);
        }
        Err(ResourceError::VersionMismatch)
    }

    async fn recovered_version(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<String, ResourceError> {
        let identity = self.recovery_identity(spec, recovery)?;
        Ok(identity.version)
    }

    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        let _gate = self.gate.lock().await;
        let identity = self.recovery_identity(spec, recovery)?;
        crate::chat::trajectory_archive::with_trajectory_mutation_lock(async {
            self.native_references(&identity).await?;
            let checked = identity.clone();
            if self
                .runtime
                .with_store(move |store| {
                    store.worktree_is_referenced(
                        &checked.record.meta.id,
                        &checked.record.meta.root.to_string_lossy(),
                        checked.record.meta.branch.as_deref(),
                    )
                })
                .await
                .map_err(|e| ResourceError::Io(e.to_string()))?
            {
                return Err(protected());
            }
            self.worktrees
                .recover_worktree_archive(identity)
                .await
                .map_err(ResourceError::ValidationFailed)?;
            Ok(ResourceApplyState::Applied)
        })
        .await
    }
}
