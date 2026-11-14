//! Durable cache only: no installation, activation, execution, or qualification authority.
//!
//! Linux is deliberately the only supported publication platform. The caller supplies an
//! existing canonical, private host-owned root; no user home/cache default is discovered.
//! Cooperative locks and pinned directory handles exclude other users, not hostile processes
//! with the same UID (which can change files or remove locks). Observed namespace changes fail
//! closed. Receipts are informational, untrusted on reload, and never reconstruct resolver or
//! qualification capabilities. This synchronous operation must not be held across an await.
//! If wrapped in spawn_blocking, cancelling the waiter leaves an UNKNOWN publication result.
use crate::public_release::VerifiedPublicCandidate;
use crate::runtime_manifest::{
    digest, native_target, require, Component, ContractResult, RuntimeManifest, MAX_MANIFEST_BYTES,
};
use crate::selection_store::SelectionStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

const RECEIPT_LIMIT: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    selection_revision: u64,
    selection_digest: String,
    manifest_digest: String,
    target: String,
    runtime_sha256: String,
    runtime_size: u64,
}

/// A rehashed cache observation, NOT permission to execute or evidence of qualification.
/// There is intentionally no conversion to VerifiedPublicCandidate or qualification types.
/// ```compile_fail
/// use refact_self_update::{runtime_stage::StagedObservation, public_release::VerifiedPublicCandidate};
/// fn authorize(observation: StagedObservation) -> VerifiedPublicCandidate { observation.into() }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedObservation(Receipt, crate::runtime_status::ManagedIdentity);
impl StagedObservation {
    /// Manifest-derived descriptive identity, not executable or qualification authority.
    pub fn identity(&self) -> &crate::runtime_status::ManagedIdentity {
        &self.1
    }
    pub fn selection_revision(&self) -> u64 {
        self.0.selection_revision
    }
    pub fn manifest_digest(&self) -> &str {
        &self.0.manifest_digest
    }
    pub fn target(&self) -> &str {
        &self.0.target
    }
    pub fn runtime_sha256(&self) -> &str {
        &self.0.runtime_sha256
    }
    pub fn runtime_size(&self) -> u64 {
        self.0.runtime_size
    }
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub struct StageStore {
    #[cfg(target_os = "linux")]
    root: linux::Directory,
}
impl StageStore {
    pub fn open(root: &Path) -> ContractResult<Self> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                root: linux::Directory::root(root)?,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = root;
            Err("durable runtime staging unsupported on this platform".into())
        }
    }

    /// Selection lock is acquired first and held through durable publication. A caller-supplied
    /// snapshot is not a fence. Both a strictly positive current revision and exact selection
    /// equality are required. Existing material is never repaired, replaced, or re-scoped.
    pub fn stage(
        &self,
        selections: &SelectionStore,
        expected_revision: u64,
        candidate: VerifiedPublicCandidate,
    ) -> ContractResult<StagedObservation> {
        #[cfg(target_os = "linux")]
        {
            self.stage_with_hook(selections, expected_revision, candidate, || Ok(()))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (selections, expected_revision, candidate);
            Err("durable runtime staging unsupported on this platform".into())
        }
    }

    // Private seam uses the exact production wrapper; tests pause after fsync, before rename.
    #[cfg(target_os = "linux")]
    fn stage_with_hook(
        &self,
        selections: &SelectionStore,
        expected_revision: u64,
        candidate: VerifiedPublicCandidate,
        prepublish: impl FnOnce() -> ContractResult<()>,
    ) -> ContractResult<StagedObservation> {
        selections.with_current(expected_revision, |snapshot| {
            require(
                candidate.resolved().selection() == snapshot.selection(),
                "candidate selection changed",
            )?;
            let artifact = candidate.artifact();
            let manifest = candidate.resolved().manifest();
            require(
                artifact.manifest_digest() == manifest.digest(),
                "candidate manifest mismatch",
            )?;
            let receipt = Receipt {
                schema_version: 1,
                selection_revision: snapshot.revision(),
                selection_digest: hash(
                    &serde_json::to_vec(snapshot.selection()).map_err(|e| e.to_string())?,
                ),
                manifest_digest: manifest.digest(),
                target: artifact.target().into(),
                runtime_sha256: hash(artifact.bytes()),
                runtime_size: artifact.bytes().len() as u64,
            };
            self.root.validate()?;
            crate::selection_store::with_file_lock(&self.root.path().join(".stage.lock"), || {
                self.publish(manifest, artifact.bytes(), &receipt, prepublish)
            })
        })
    }

    /// Rehashes exact bounded bytes. Receipt contents supply no trust, qualification, or authority.
    pub fn observe(
        &self,
        target: &str,
        manifest_digest: &str,
        selection_revision: u64,
    ) -> ContractResult<StagedObservation> {
        require(
            native_target(target) && digest(manifest_digest, 64) && selection_revision > 0,
            "invalid stage key",
        )?;
        #[cfg(target_os = "linux")]
        {
            self.root.validate()?;
            crate::selection_store::with_file_lock_existing(
                &self.root.path().join(".stage.lock"),
                || {
                    let version = self.root.child("v1", false)?;
                    let targets = version.child(target, false)?;
                    let content = targets.child(manifest_digest, false)?;
                    let entry = content.child(&selection_revision.to_string(), false)?;
                    let result = linux::observe(&entry, target, manifest_digest)?;
                    require(
                        result.selection_revision() == selection_revision,
                        "stage revision mismatch",
                    )?;
                    self.root.validate()?;
                    Ok(result)
                },
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err("durable runtime staging unsupported on this platform".into())
        }
    }

    #[cfg(target_os = "linux")]
    fn publish(
        &self,
        manifest: &RuntimeManifest,
        runtime: &[u8],
        receipt: &Receipt,
        prepublish: impl FnOnce() -> ContractResult<()>,
    ) -> ContractResult<StagedObservation> {
        require(
            native_target(&receipt.target) && digest(&receipt.manifest_digest, 64),
            "invalid stage key",
        )?;
        let version = self.root.child("v1", true)?;
        let targets = version.child(&receipt.target, true)?;
        let content = targets.child(&receipt.manifest_digest, true)?;
        let revision_key = receipt.selection_revision.to_string();
        match content.child(&revision_key, false) {
            Ok(entry) => {
                let observed = linux::observe(&entry, &receipt.target, &receipt.manifest_digest)?;
                require(
                    observed.0 == *receipt,
                    "existing stage revision or identity differs",
                )?;
                self.root.validate()?;
                return Ok(observed);
            }
            Err(e) => {
                // A corrupt/symlinked/non-directory destination must not be treated as absent.
                if std::fs::symlink_metadata(content.path().join(&revision_key)).is_ok() {
                    return Err(e);
                }
                if std::fs::symlink_metadata(content.path().join(&revision_key))
                    .err()
                    .is_none_or(|e| e.kind() != std::io::ErrorKind::NotFound)
                {
                    return Err(e);
                }
            }
        }
        let temporary = tempfile::Builder::new()
            .prefix(".stage-")
            .permissions(linux::permissions(0o700))
            .tempdir_in(content.path())
            .map_err(|e| e.to_string())?;
        linux::write(
            &temporary.path().join("manifest.json"),
            &manifest.canonical_bytes(),
            0o600,
        )?;
        linux::write(&temporary.path().join("runtime"), runtime, 0o700)?;
        let bytes = serde_json::to_vec(receipt).map_err(|e| e.to_string())?;
        require(
            bytes.len() <= RECEIPT_LIMIT,
            "stage receipt byte limit exceeded",
        )?;
        linux::write(&temporary.path().join("receipt.json"), &bytes, 0o600)?;
        let tempdir = linux::Directory::open(temporary.path())?;
        let checked = linux::observe(&tempdir, &receipt.target, &receipt.manifest_digest)?;
        require(checked.0 == *receipt, "temporary stage identity mismatch")?;
        tempdir.sync()?;
        content.sync()?;
        prepublish()?;
        self.root.validate()?;
        targets.validate()?;
        content.validate()?;
        linux::rename_no_replace(temporary.path(), &content.path().join(&revision_key))?;
        // TempDir still owns only its now-absent old random name, never the published directory.
        content.sync()?;
        self.root.validate()?;
        Ok(checked)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::runtime_artifact::MAX_RUNTIME_BYTES;
    use crate::selection_store::{check_file, safe_open};
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::PathBuf;

    pub(super) fn permissions(mode: u32) -> std::fs::Permissions {
        std::fs::Permissions::from_mode(mode)
    }
    pub(super) struct Directory {
        file: File,
        named: PathBuf,
    }
    impl Directory {
        pub(super) fn root(path: &Path) -> ContractResult<Self> {
            require(
                path.is_absolute() && path.canonicalize().map_err(|e| e.to_string())? == path,
                "existing canonical stage root required",
            )?;
            Self::open(path)
        }
        pub(super) fn open(path: &Path) -> ContractResult<Self> {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(path)
                .map_err(|e| e.to_string())?;
            let directory = Self {
                file,
                named: path.to_owned(),
            };
            directory.validate()?;
            Ok(directory)
        }
        pub(super) fn validate(&self) -> ContractResult<()> {
            let m = self.file.metadata().map_err(|e| e.to_string())?;
            let named = std::fs::symlink_metadata(&self.named).map_err(|e| e.to_string())?;
            require(
                m.is_dir()
                    && m.uid() == unsafe { libc::geteuid() }
                    && m.mode() & 0o7777 == 0o700
                    && named.is_dir()
                    && !named.file_type().is_symlink()
                    && (m.dev(), m.ino()) == (named.dev(), named.ino()),
                "stage directory owner/mode/identity rejected",
            )
        }
        pub(super) fn path(&self) -> PathBuf {
            PathBuf::from(format!("/proc/self/fd/{}", self.file.as_raw_fd()))
        }
        pub(super) fn sync(&self) -> ContractResult<()> {
            self.file.sync_all().map_err(|e| e.to_string())
        }
        pub(super) fn child(&self, name: &str, create: bool) -> ContractResult<Self> {
            self.validate()?;
            let path = self.path().join(name);
            if create {
                use std::os::unix::fs::DirBuilderExt;
                match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.to_string()),
                }
                // Also repair durability when an earlier interrupted writer created this
                // ancestor but had not yet synced its parent directory.
                self.sync()?;
            }
            Self::open(&path)
        }
    }
    pub(super) fn write(path: &Path, bytes: &[u8], mode: u32) -> ContractResult<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())
    }
    fn open(path: &Path, expected_mode: u32, limit: u64) -> ContractResult<File> {
        let file = safe_open(path, false)?;
        check_file(&file)?;
        let m = file.metadata().map_err(|e| e.to_string())?;
        require(
            m.mode() & 0o7777 == expected_mode && m.len() <= limit,
            "stage file mode/size rejected",
        )?;
        Ok(file)
    }
    fn unchanged(file: &File, before: &std::fs::Metadata, path: &Path) -> ContractResult<()> {
        check_file(file)?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        let named = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        require(
            !named.file_type().is_symlink()
                && (before.dev(), before.ino()) == (named.dev(), named.ino())
                && (
                    before.len(),
                    before.mtime(),
                    before.mtime_nsec(),
                    before.ctime(),
                    before.ctime_nsec(),
                ) == (
                    after.len(),
                    after.mtime(),
                    after.mtime_nsec(),
                    after.ctime(),
                    after.ctime_nsec(),
                ),
            "stage file changed while reading",
        )
    }
    fn read(path: &Path, limit: usize) -> ContractResult<Vec<u8>> {
        let file = open(path, 0o600, limit as u64)?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        (&file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        require(
            bytes.len() <= limit && bytes.len() as u64 == before.len(),
            "stage metadata size mismatch",
        )?;
        unchanged(&file, &before, path)?;
        Ok(bytes)
    }
    pub(super) fn observe(
        entry: &Directory,
        target: &str,
        manifest_digest: &str,
    ) -> ContractResult<StagedObservation> {
        entry.validate()?;
        let bytes = read(&entry.path().join("manifest.json"), MAX_MANIFEST_BYTES)?;
        let manifest = RuntimeManifest::from_json(&bytes)?;
        require(
            bytes == manifest.canonical_bytes() && manifest.digest() == manifest_digest,
            "stage canonical manifest mismatch",
        )?;
        let artifact = manifest
            .artifacts()
            .iter()
            .find(|a| a.component == Component::Runtime && a.target == target)
            .ok_or("stage target absent")?;
        let name = if target.contains("windows") {
            "refact.exe"
        } else {
            "refact"
        };
        require(
            artifact.extracted_files.len() == 1 && artifact.extracted_files[0].path == name,
            "stage runtime binding rejected",
        )?;
        let runtime = &artifact.extracted_files[0];
        require(
            runtime.size > 0 && runtime.size <= MAX_RUNTIME_BYTES,
            "stage runtime byte limit exceeded",
        )?;
        let path = entry.path().join("runtime");
        let file = open(&path, 0o700, runtime.size)?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        require(before.len() == runtime.size, "stage runtime size mismatch")?;
        let mut reader = (&file).take(runtime.size + 1);
        let mut hasher = Sha256::new();
        let mut count = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            count += n as u64;
            require(count <= runtime.size, "stage runtime size mismatch")?;
            hasher.update(&buffer[..n]);
        }
        require(
            count == runtime.size && hex::encode(hasher.finalize()) == runtime.sha256,
            "stage runtime hash/size mismatch",
        )?;
        unchanged(&file, &before, &path)?;
        let receipt: Receipt =
            serde_json::from_slice(&read(&entry.path().join("receipt.json"), RECEIPT_LIMIT)?)
                .map_err(|e| e.to_string())?;
        require(
            receipt.schema_version == 1
                && receipt.selection_revision > 0
                && digest(&receipt.selection_digest, 64)
                && receipt.manifest_digest == manifest_digest
                && receipt.target == target
                && receipt.runtime_sha256 == runtime.sha256
                && receipt.runtime_size == runtime.size,
            "stage receipt identity mismatch",
        )?;
        entry.validate()?;
        let identity = crate::runtime_status::ManagedIdentity::from_manifest(&manifest, target)?;
        Ok(StagedObservation(receipt, identity))
    }
    #[cfg(test)]
    #[test]
    fn prepublication_failure_removes_only_owned_temporary_directory() {
        let root = tempfile::Builder::new()
            .permissions(permissions(0o700))
            .tempdir()
            .unwrap();
        let store = StageStore::open(&root.path().canonicalize().unwrap()).unwrap();
        let target = "x86_64-unknown-linux-gnu";
        let manifest = test_manifest(target);
        let receipt = Receipt {
            schema_version: 1,
            selection_revision: 1,
            selection_digest: "b".repeat(64),
            manifest_digest: manifest.digest(),
            target: target.into(),
            runtime_sha256: hash(b"new"),
            runtime_size: 3,
        };
        let error = store
            .publish(&manifest, b"new", &receipt, || {
                Err("injected prepublish failure".into())
            })
            .unwrap_err();
        assert_eq!(error, "injected prepublish failure");
        let parent = root.path().join("v1").join(target).join(manifest.digest());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
        assert!(!parent.join("1").exists());
    }

    #[cfg(test)]
    fn test_manifest(target: &str) -> RuntimeManifest {
        let value = serde_json::json!({"schema_version":1,
            "repository":{"owner":"JegernOUTT","name":"refact","id":1254216370u64},
            "source_sha":"a".repeat(40),"workflow":{"path":".github/workflows/plugin_build.yml","id":1},"run":{"id":1,"attempt":1},
            "build":{"version":"1.2.3","channel":"preview"},
            "build_inputs":{"rust_toolchain":"1","node_version":"22","profile":"release","cargo_lock_sha256":"a".repeat(64),"javascript_locks_sha256":"a".repeat(64),"config_sha256":"a".repeat(64)},
            "component_versions":{"runtime":"1.2.3","gui":"1","vscode":"1","jetbrains":"1"},
            "compatibility":{"daemon_api":{"min":1,"max":1},"worker_api":{"min":1,"max":1},"host_bridge":{"min":1,"max":1}},
            "artifacts":[{"component":"runtime","target":target,"filename":"runtime.tar.gz","format":"tar_gz","size":1,"sha256":"a".repeat(64),"extracted_files":[{"path":"refact","size":3,"sha256":hash(b"new")}]}],"tests":"not_run"});
        RuntimeManifest::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    #[cfg(test)]
    #[tokio::test]
    async fn actual_stage_holds_selection_fence_through_publication() {
        use crate::public_release::*;
        use crate::runtime_artifact::ArtifactLimits;
        use crate::runtime_manifest::Channel;
        use serde_json::json;
        use std::sync::{mpsc, Mutex};
        use std::time::Duration;
        struct Fake(Mutex<std::collections::VecDeque<(String, Vec<u8>)>>);
        impl PublicTransport for Fake {
            fn get<'a>(&'a self, url: &'a str, _: bool, cap: usize) -> ReadFuture<'a> {
                Box::pin(async move {
                    let (suffix, body) = self.0.lock().unwrap().pop_front().unwrap();
                    assert!(url.ends_with(&suffix));
                    assert!(body.len() <= cap);
                    Ok(PublicResponse {
                        status: 200,
                        location: None,
                        body,
                    })
                })
            }
        }
        let target = "x86_64-unknown-linux-gnu";
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "refact", std::io::Cursor::new(b"new"))
            .unwrap();
        let archive = builder.into_inner().unwrap().finish().unwrap();
        let mut value = serde_json::to_value(test_manifest(target)).unwrap();
        value["artifacts"][0]["size"] = json!(archive.len());
        value["artifacts"][0]["sha256"] = json!(hash(&archive));
        let bytes = serde_json::to_vec(&value).unwrap();
        let repo = json!({"id":1254216370u64,"full_name":"JegernOUTT/refact"});
        let rows = vec![
            ("/refact", serde_json::to_vec(&repo).unwrap()),
            ("releases/tags/engine%2Fv1%2E2%2E3", serde_json::to_vec(&json!({"id":99,"tag_name":"engine/v1.2.3","draft":false,"prerelease":true,"assets":[
                {"id":1,"name":"runtime-manifest.json","size":bytes.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&bytes))},
                {"id":2,"name":"runtime.tar.gz","size":archive.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&archive))}]})).unwrap()),
            ("git/ref/tags/engine%2Fv1%2E2%2E3", serde_json::to_vec(&json!({"ref":"refs/tags/engine/v1.2.3","object":{"type":"commit","sha":"a".repeat(40)}})).unwrap()),
            ("releases/assets/1", bytes),
            ("actions/workflows/plugin%5Fbuild%2Eyml", serde_json::to_vec(&json!({"id":1,"path":".github/workflows/plugin_build.yml","state":"active"})).unwrap()),
            ("actions/runs/1/attempts/1", serde_json::to_vec(&json!({"id":1,"run_attempt":1,"workflow_id":1,"path":".github/workflows/plugin_build.yml","head_sha":"a".repeat(40),"event":"workflow_dispatch","status":"completed","conclusion":"success","repository":repo,"head_repository":repo})).unwrap()),
            ("releases/assets/2", archive),
        ];
        let fake = Fake(Mutex::new(
            rows.into_iter().map(|(p, b)| (p.into(), b)).collect(),
        ));
        let selection = parse_source_url(
            "https://github.com/JegernOUTT/refact/releases/tag/engine/v1.2.3",
            Channel::Preview,
        )
        .unwrap();
        let resolved = resolve_public_release(&fake, &selection, target, (1, 1, 1))
            .await
            .unwrap();
        let candidate = fetch_verified_candidate(
            &fake,
            resolved,
            &selection,
            target,
            (1, 1, 1),
            ArtifactLimits {
                archive_bytes: 1 << 20,
                runtime_bytes: 1024,
            },
        )
        .await
        .unwrap();
        let root = tempfile::Builder::new()
            .permissions(permissions(0o700))
            .tempdir()
            .unwrap();
        let state = tempfile::Builder::new()
            .permissions(permissions(0o700))
            .tempdir()
            .unwrap();
        let path = state.path().canonicalize().unwrap().join("selection.json");
        let selections = SelectionStore::new(path.clone()).unwrap();
        selections.compare_and_swap(0, selection.clone()).unwrap();
        let store = StageStore::open(&root.path().canonicalize().unwrap()).unwrap();
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            // A handshake, not a sleep: prove a real conflicting lock attempt was refused.
            let mut lockname = path.as_os_str().to_os_string();
            lockname.push(".lock");
            let mut lock = fd_lock::RwLock::new(
                crate::selection_store::safe_open(Path::new(&lockname), true).unwrap(),
            );
            // Wait until the publication hook confirms it is holding the production fence.
            attempt_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            let refused = match lock.try_write() {
                Err(e) => e.kind() == std::io::ErrorKind::WouldBlock,
                Ok(_) => false,
            };
            finish_tx.send(refused).unwrap();
            let result = SelectionStore::new(path)
                .unwrap()
                .compare_and_swap(1, selection);
            done_tx.send(result).unwrap();
        });
        let observation = store
            .stage_with_hook(&selections, 1, candidate, || {
                attempt_tx.send(()).map_err(|e| e.to_string())?;
                require(
                    finish_rx
                        .recv_timeout(Duration::from_secs(10))
                        .map_err(|e| e.to_string())?,
                    "CAS was not fenced before publication",
                )
            })
            .unwrap();
        // Bounded readiness/completion channels prevent an unbounded join on lock regressions.
        let updated = done_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        handle.join().unwrap();
        assert_eq!(updated.revision(), 2);
        assert_eq!(observation.selection_revision(), 1);
        assert_eq!(
            store
                .observe(target, observation.manifest_digest(), 1)
                .unwrap(),
            observation
        );
    }

    pub(super) fn rename_no_replace(from: &Path, to: &Path) -> ContractResult<()> {
        let from =
            std::ffi::CString::new(from.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let to = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(format!(
                "atomic no-overwrite stage publication failed: {}",
                std::io::Error::last_os_error()
            ))
        }
    }
}
