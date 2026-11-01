use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    AsyncResourceAdapter, DraftRegistry, RecoveryDescriptor, RecoveryMaterialStore,
    ResourceApplyReceipt, ResourceApplyState, ResourceCapabilities, ResourceError, ResourcePreview,
};
use crate::app_state::AppState;
use crate::buddy::memory_lifecycle::{
    apply_memory_lifecycle_op, MemoryLifecycleOp, MemoryOpStatus, MemoryOpType,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryDraft {
    pub project: ProjectIdentity,
    pub resource_id: String,
    pub operation: MemoryLifecycleOp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MemoryPreimage {
    project: ProjectIdentity,
    resource_id: String,
    files: BTreeMap<String, Vec<u8>>,
}

pub struct MemoryAdapter {
    app: AppState,
    project: ProjectIdentity,
    drafts: Arc<dyn DraftRegistry<MemoryDraft>>,
    materials: Arc<dyn RecoveryMaterialStore>,
    gate: tokio::sync::Mutex<()>,
}

impl MemoryAdapter {
    pub fn new(
        app: AppState,
        project: ProjectIdentity,
        drafts: Arc<dyn DraftRegistry<MemoryDraft>>,
        materials: Arc<dyn RecoveryMaterialStore>,
    ) -> Self {
        Self {
            app,
            project,
            drafts,
            materials,
            gate: tokio::sync::Mutex::new(()),
        }
    }

    async fn root(&self, spec: &ActionSpec) -> Result<PathBuf, ResourceError> {
        if spec.project != self.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if spec.adapter != self.adapter_id()
            || spec.resource.resource_kind != "memory"
            || spec.kind != "memory_lifecycle"
        {
            return Err(ResourceError::UnsupportedResource);
        }
        let roots = crate::files_correction::get_project_dirs(self.app.gcx.clone()).await;
        let root = tokio::fs::canonicalize(&self.project.canonical_root).await?;
        let root = dunce::simplified(&root).to_path_buf();
        if roots.len() != 1
            || dunce::simplified(&tokio::fs::canonicalize(&roots[0]).await?) != root
            || root.to_string_lossy() != self.project.canonical_root
        {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        Ok(root.join(crate::file_filter::KNOWLEDGE_FOLDER_NAME))
    }

    async fn prepare(
        &self,
        spec: &ActionSpec,
    ) -> Result<(MemoryDraft, MemoryPreimage), ResourceError> {
        let root = self.root(spec).await?;
        let mut draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        // Validate the exact normalized payload the native owner will consume.
        draft.operation = draft.operation.normalized();
        if draft.project != self.project || draft.resource_id != spec.resource.resource_id {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        if !matches!(
            draft.operation.op_type,
            MemoryOpType::CreateMemory | MemoryOpType::Archive | MemoryOpType::MergeArchive
        ) {
            return Err(ResourceError::UnsupportedResource);
        }
        if !matches!(
            draft.operation.status,
            MemoryOpStatus::Pending | MemoryOpStatus::Approved
        ) {
            return Err(ResourceError::ValidationFailed(
                "memory operation is already finalized".into(),
            ));
        }
        let files = snapshot(&root).await?;
        let preimage = MemoryPreimage {
            project: self.project.clone(),
            resource_id: draft.resource_id.clone(),
            files,
        };
        if fingerprint(&preimage)? != spec.expected_version
            || spec.resource.version != spec.expected_version
        {
            return Err(ResourceError::VersionMismatch);
        }
        for target in draft
            .operation
            .target_paths
            .iter()
            .chain(draft.operation.payload.superseded_paths.iter())
        {
            let path = Path::new(target);
            if !path.is_absolute() || !path.starts_with(&root) {
                return Err(ResourceError::PathEscapesProject);
            }
            let canonical = tokio::fs::canonicalize(path).await?;
            let canonical = dunce::simplified(&canonical);
            if canonical != path || !canonical.starts_with(&root) {
                return Err(ResourceError::SymlinkNotAllowed);
            }
            let relative = canonical
                .strip_prefix(&root)
                .map_err(|_| ResourceError::PathEscapesProject)?
                .to_string_lossy()
                .into_owned();
            let bytes = preimage
                .files
                .get(&relative)
                .ok_or(ResourceError::ResourceMissing)?;
            if draft.operation.op_type == MemoryOpType::MergeArchive
                && draft.operation.target_paths.first() == Some(target)
            {
                let canonical = draft.operation.payload.canonical.as_ref().ok_or_else(|| {
                    ResourceError::ValidationFailed("canonical memory missing".into())
                })?;
                preserve_anchors(bytes, &canonical.content)?;
                let text = std::str::from_utf8(bytes).map_err(|_| {
                    ResourceError::ValidationFailed("invalid memory encoding".into())
                })?;
                let (metadata, _) =
                    crate::knowledge_graph::kg_structs::KnowledgeFrontmatter::parse(text);
                if metadata
                    .links
                    .iter()
                    .any(|link| !canonical.links.contains(link))
                    || metadata
                        .filenames
                        .iter()
                        .any(|link| !canonical.filenames.contains(link))
                    || metadata
                        .related_files
                        .iter()
                        .any(|link| !canonical.related_files.contains(link))
                {
                    return Err(ResourceError::ValidationFailed(
                        "merge must preserve existing links and file references".into(),
                    ));
                }
                if draft.operation.payload.source_id.is_some()
                    || draft.operation.payload.source_content_hash.is_some()
                {
                    return Err(ResourceError::ValidationFailed(
                        "merge cannot replace canonical provenance".into(),
                    ));
                }
            }
        }
        if draft.operation.op_type == MemoryOpType::MergeArchive
            && draft.operation.target_paths.is_empty()
        {
            return Err(ResourceError::ValidationFailed(
                "merge requires an existing canonical memory".into(),
            ));
        }
        if draft.operation.op_type == MemoryOpType::Archive
            && draft.operation.target_paths.is_empty()
        {
            return Err(ResourceError::ValidationFailed(
                "archive requires exact targets".into(),
            ));
        }
        if draft.operation.op_type == MemoryOpType::CreateMemory
            && (!draft.operation.target_paths.is_empty()
                || !draft.operation.payload.superseded_paths.is_empty())
        {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        if draft.operation.op_type == MemoryOpType::MergeArchive
            && (draft.operation.target_paths.len() != 1
                || draft.operation.payload.superseded_paths.is_empty()
                || draft
                    .operation
                    .payload
                    .superseded_paths
                    .iter()
                    .any(|p| draft.operation.target_paths.contains(p)))
        {
            return Err(ResourceError::ValidationFailed(
                "merge requires one canonical and distinct explicit superseded identities".into(),
            ));
        }
        if draft.operation.op_type == MemoryOpType::Archive
            && (draft.operation.payload.canonical.is_some()
                || draft.operation.payload.superseded_by.is_some()
                || !draft.operation.payload.superseded_paths.is_empty())
        {
            return Err(ResourceError::ValidationFailed(
                "archive cannot silently act as a merge".into(),
            ));
        }
        Ok((draft, preimage))
    }

    pub async fn version(&self, resource_id: &str) -> Result<String, ResourceError> {
        let root =
            Path::new(&self.project.canonical_root).join(crate::file_filter::KNOWLEDGE_FOLDER_NAME);
        fingerprint(&MemoryPreimage {
            project: self.project.clone(),
            resource_id: resource_id.to_owned(),
            files: snapshot(&root).await?,
        })
    }
}

pub(crate) fn fingerprint<T: Serialize>(value: &T) -> Result<String, ResourceError> {
    let bytes =
        serde_json::to_vec(value).map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

async fn snapshot(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, ResourceError> {
    let mut files = BTreeMap::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let metadata = match tokio::fs::symlink_metadata(&directory).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && directory == root => {
                continue
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(ResourceError::SymlinkNotAllowed);
        }
        let mut entries = tokio::fs::read_dir(&directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let kind = entry.file_type().await?;
            if kind.is_symlink() {
                return Err(ResourceError::SymlinkNotAllowed);
            }
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "md" || ext == "mdx")
            {
                let path = entry.path();
                let name = path
                    .strip_prefix(root)
                    .map_err(|_| ResourceError::PathEscapesProject)?
                    .to_string_lossy()
                    .into_owned();
                files.insert(name, tokio::fs::read(path).await?);
            }
        }
    }
    Ok(files)
}

fn preserve_anchors(before: &[u8], after: &str) -> Result<(), ResourceError> {
    let before = std::str::from_utf8(before)
        .map_err(|_| ResourceError::ValidationFailed("invalid memory encoding".into()))?;
    for line in before.lines().filter(|line| {
        line.starts_with('#')
            || line.contains("<a ")
            || line.contains("id=\"")
            || line.starts_with("Source:")
            || line.starts_with("[")
            || line.contains("](")
            || line.contains("][")
            || line.contains("<http")
    }) {
        if !after.lines().any(|new| new == line) {
            return Err(ResourceError::ValidationFailed(
                "merge would remove an existing anchor or provenance reference".into(),
            ));
        }
    }
    Ok(())
}

#[async_trait::async_trait]
impl AsyncResourceAdapter for MemoryAdapter {
    fn adapter_id(&self) -> &str {
        "memory"
    }
    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities {
            preview: true,
            apply: true,
            verify: true,
            recover: false,
        }
    }

    async fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError> {
        let (draft, preimage) = self.prepare(spec).await?;
        Ok(ResourcePreview {
            resource_id: spec.resource.resource_id.clone(),
            expected_version: spec.expected_version.clone(),
            proposed_version: fingerprint(&(preimage, &draft.operation))?,
            reload_needed: false,
            diff: Some(
                serde_json::to_string(&draft.operation)
                    .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?,
            ),
        })
    }

    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        crate::memories::with_memory_mutation_lock(async {
            let _guard = self.gate.lock().await;
            let (draft, preimage) = self.prepare(spec).await?;
            let previous_material_reference = self.materials.put(
                &serde_json::to_vec(&preimage)
                    .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?,
            )?;
            let outcome = apply_memory_lifecycle_op(self.app.clone(), &draft.operation)
                .await
                .map_err(ResourceError::Io)?;
            let expected_post_version = self.version(&spec.resource.resource_id).await?;
            Ok(ResourceApplyReceipt {
                resource_id: spec.resource.resource_id.clone(),
                previous_material_reference,
                expected_post_version: expected_post_version.clone(),
                state: if outcome.status == MemoryOpStatus::Applied
                    || expected_post_version != spec.expected_version
                {
                    ResourceApplyState::Applied
                } else {
                    ResourceApplyState::NotApplied
                },
            })
        })
        .await
    }

    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        self.root(spec).await?;
        Ok(self.version(&spec.resource.resource_id).await? == post)
    }

    async fn recover(
        &self,
        _: &ActionSpec,
        _: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        Err(ResourceError::UnsupportedResource)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Draft(MemoryDraft);
    impl DraftRegistry<MemoryDraft> for Draft {
        fn get(&self, reference: &str) -> Result<MemoryDraft, ResourceError> {
            if reference == "draft" {
                Ok(self.0.clone())
            } else {
                Err(ResourceError::MaterialUnavailable)
            }
        }
    }
    #[derive(Default)]
    struct Materials(std::sync::Mutex<Vec<u8>>);
    impl RecoveryMaterialStore for Materials {
        fn put(&self, bytes: &[u8]) -> Result<String, ResourceError> {
            *self.0.lock().unwrap() = bytes.to_vec();
            Ok("opaque".into())
        }
        fn get(&self, _: &str) -> Result<Vec<u8>, ResourceError> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    #[tokio::test]
    async fn canonical_archive_preserves_unknown_metadata_and_body_and_rejects_stale_retry() {
        use refact_buddy_core::operator_types::{
            ActionAvailability, ActionCapabilities, AuthorityClass, ConfirmationClass,
        };
        use crate::buddy::memory_lifecycle::MemorySource;
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let knowledge = root.join(crate::file_filter::KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge).await.unwrap();
        let path = knowledge.join("memory.md");
        tokio::fs::write(&path, "---\nid: memory\ntitle: Memory\nstatus: active\nfuture_key: retained\nsource_id: trajectory:memo:0-0:root:hash\n---\n\n# Stable anchor\n\nBody\n").await.unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![root.clone()];
        let project = ProjectIdentity {
            project_id: "p".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        };
        let mut operation = MemoryLifecycleOp::pending(
            "archive",
            MemorySource::MemoryGarden,
            MemoryOpType::Archive,
            vec![path.to_string_lossy().into_owned()],
            "archive approved",
            0.9,
            "2026-05-02T00:00:00Z",
        );
        operation.status = MemoryOpStatus::Approved;
        let adapter = MemoryAdapter::new(
            app,
            project.clone(),
            Arc::new(Draft(MemoryDraft {
                project: project.clone(),
                resource_id: "memory".into(),
                operation,
            })),
            Arc::new(Materials::default()),
        );
        let version = adapter.version("memory").await.unwrap();
        let spec = ActionSpec {
            schema_version: 1,
            action_id: "archive".into(),
            kind: "memory_lifecycle".into(),
            adapter: "memory".into(),
            project,
            resource: refact_buddy_core::operator_types::ResourceRef {
                resource_kind: "memory".into(),
                resource_id: "memory".into(),
                version: version.clone(),
            },
            expected_version: version,
            parameter_ref: Some("draft".into()),
            availability: ActionAvailability::Available,
            confirmation: ConfirmationClass::None,
            authority_class: AuthorityClass::Mutating,
            capabilities: ActionCapabilities {
                can_preview: true,
                can_execute: true,
                can_verify: true,
                can_recover: false,
                can_cancel: false,
            },
        };
        adapter.preview(&spec).await.unwrap();
        let receipt = adapter.apply(&spec).await.unwrap();
        assert_eq!(receipt.state, ResourceApplyState::Applied);
        assert!(adapter
            .verify(&spec, &receipt.expected_post_version)
            .await
            .unwrap());
        let text = tokio::fs::read_to_string(path).await.unwrap();
        assert!(text.contains("future_key: retained"));
        assert!(text.contains("# Stable anchor"));
        assert!(text.contains("trajectory:memo:0-0:root:hash"));
        assert!(matches!(
            adapter.apply(&spec).await,
            Err(ResourceError::VersionMismatch)
        ));
    }

    #[test]
    fn merge_cannot_drop_anchors_or_source_references() {
        let before = b"# Stable heading\n<a id=\"stable\"></a>\nSource: trajectory `t`\nbody";
        assert!(preserve_anchors(before, "replacement").is_err());
        assert!(preserve_anchors(
            b"See [original](trajectory:chat-a#message-3)",
            "See replacement"
        )
        .is_err());
        assert!(preserve_anchors(
            before,
            "# Stable heading\n<a id=\"stable\"></a>\nSource: trajectory `t`\nnew body"
        )
        .is_ok());
    }
}
