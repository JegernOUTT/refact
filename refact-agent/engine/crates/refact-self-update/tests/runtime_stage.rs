//! Hermetic staging only: fake public transport, owned temp roots, never executes a runtime.
#![cfg(target_os = "linux")]
use refact_self_update::public_release::*;
use refact_self_update::runtime_artifact::ArtifactLimits;
use refact_self_update::runtime_manifest::Channel;
use refact_self_update::runtime_selection::RuntimeSelection;
use refact_self_update::runtime_stage::{StageStore, StagedObservation};
use refact_self_update::selection_store::SelectionStore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::Cursor;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, Mutex};
const TARGET: &str = "x86_64-unknown-linux-gnu";
const BASE: &str = "https://api.github.com/repos/JegernOUTT/refact";
fn hash(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}
fn selection() -> RuntimeSelection {
    parse_source_url(
        "https://github.com/JegernOUTT/refact/releases/tag/engine/v1.2.3",
        Channel::Preview,
    )
    .unwrap()
}
struct Fake(Mutex<VecDeque<(String, bool, Vec<u8>)>>);
impl PublicTransport for Fake {
    fn get<'a>(&'a self, url: &'a str, asset: bool, cap: usize) -> ReadFuture<'a> {
        Box::pin(async move {
            let (path, expected_asset, body) =
                self.0.lock().unwrap().pop_front().expect("unexpected I/O");
            assert_eq!(
                url,
                if path.is_empty() {
                    BASE.into()
                } else {
                    format!("{BASE}/{path}")
                }
            );
            assert_eq!(asset, expected_asset);
            assert!(body.len() <= cap);
            Ok(PublicResponse {
                status: 200,
                location: None,
                body,
            })
        })
    }
}
async fn candidate() -> VerifiedPublicCandidate {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut h = tar::Header::new_gnu();
    h.set_size(3);
    h.set_mode(0o755);
    h.set_cksum();
    builder
        .append_data(&mut h, "refact", Cursor::new(b"new"))
        .unwrap();
    let archive = builder.into_inner().unwrap().finish().unwrap();
    let manifest = serde_json::to_vec(&json!({"schema_version":1,
        "repository":{"owner":"JegernOUTT","name":"refact","id":1254216370u64},
        "source_sha":"a".repeat(40),"workflow":{"path":".github/workflows/plugin_build.yml","id":296686373},"run":{"id":123,"attempt":2},
        "build":{"version":"1.2.3","channel":"preview"},
        "build_inputs":{"rust_toolchain":"1.85.0","node_version":"22","profile":"release","cargo_lock_sha256":"b".repeat(64),"javascript_locks_sha256":"c".repeat(64),"config_sha256":"d".repeat(64)},
        "component_versions":{"runtime":"1.2.3","gui":"1","vscode":"1","jetbrains":"1"},
        "compatibility":{"daemon_api":{"min":1,"max":1},"worker_api":{"min":1,"max":1},"host_bridge":{"min":1,"max":1}},
        "artifacts":[{"component":"runtime","target":TARGET,"filename":"runtime.tar.gz","format":"tar_gz","size":archive.len(),"sha256":hash(&archive),"extracted_files":[{"path":"refact","size":3,"sha256":hash(b"new")}]}],"tests":"not_run"})).unwrap();
    let repo = json!({"id":1254216370u64,"full_name":"JegernOUTT/refact"});
    let mut rows = VecDeque::new();
    let mut api = |path: &str, v: Value| {
        rows.push_back((path.to_owned(), false, serde_json::to_vec(&v).unwrap()));
    };
    api("", repo.clone());
    api(
        "releases/tags/engine%2Fv1%2E2%2E3",
        json!({"id":99,"tag_name":"engine/v1.2.3","draft":false,"prerelease":true,
        "assets":[{"id":1,"name":"runtime-manifest.json","size":manifest.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&manifest))},
        {"id":2,"name":"runtime.tar.gz","size":archive.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&archive))}]}),
    );
    api(
        "git/ref/tags/engine%2Fv1%2E2%2E3",
        json!({"ref":"refs/tags/engine/v1.2.3","object":{"type":"commit","sha":"a".repeat(40)}}),
    );
    rows.push_back(("releases/assets/1".into(), true, manifest));
    rows.push_back((
        "actions/workflows/plugin%5Fbuild%2Eyml".into(),
        false,
        serde_json::to_vec(
            &json!({"id":296686373,"path":".github/workflows/plugin_build.yml","state":"active"}),
        )
        .unwrap(),
    ));
    rows.push_back(("actions/runs/123/attempts/2".into(), false, serde_json::to_vec(&json!({"id":123,"run_attempt":2,"workflow_id":296686373,"path":".github/workflows/plugin_build.yml","head_sha":"a".repeat(40),"event":"workflow_dispatch","status":"completed","conclusion":"success","repository":repo,"head_repository":repo})).unwrap()));
    rows.push_back(("releases/assets/2".into(), true, archive));
    let fake = Fake(Mutex::new(rows));
    let resolved = resolve_public_release(&fake, &selection(), TARGET, (1, 1, 1))
        .await
        .unwrap();
    let result = fetch_verified_candidate(
        &fake,
        resolved,
        &selection(),
        TARGET,
        (1, 1, 1),
        ArtifactLimits {
            archive_bytes: 1 << 20,
            runtime_bytes: 1024,
        },
    )
    .await
    .unwrap();
    assert!(fake.0.lock().unwrap().is_empty());
    result
}
fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
fn entry(root: &Path, observation: &StagedObservation) -> PathBuf {
    root.join("v1")
        .join(observation.target())
        .join(observation.manifest_digest())
        .join(observation.selection_revision().to_string())
}
fn stores() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    SelectionStore,
    StageStore,
) {
    let root = scratch();
    let selection_root = scratch();
    let selections = SelectionStore::new(
        selection_root
            .path()
            .canonicalize()
            .unwrap()
            .join("selection.json"),
    )
    .unwrap();
    selections.compare_and_swap(0, selection()).unwrap();
    let store = StageStore::open(&root.path().canonicalize().unwrap()).unwrap();
    (root, selection_root, selections, store)
}
#[test]
fn observing_empty_root_is_read_only() {
    let root = scratch();
    let store = StageStore::open(&root.path().canonicalize().unwrap()).unwrap();
    let before = cache_snapshot(root.path());
    assert!(store.observe(TARGET, &"a".repeat(64), 1).is_err());
    assert_eq!(cache_snapshot(root.path()), before);
}
#[tokio::test]
async fn durable_idempotent_rehash_preserves_selection_and_installed_sentinel() {
    let (root, selection_root, selections, store) = stores();
    let installed = selection_root.path().join("installed");
    std::fs::write(&installed, b"old").unwrap();
    let before = std::fs::read(selections.path()).unwrap();
    let observed = store.stage(&selections, 1, candidate().await).unwrap();
    assert_eq!(
        observed,
        store.stage(&selections, 1, candidate().await).unwrap()
    );
    drop(store);
    let reopened = StageStore::open(&root.path().canonicalize().unwrap()).unwrap();
    assert_eq!(
        observed,
        reopened
            .observe(
                TARGET,
                observed.manifest_digest(),
                observed.selection_revision()
            )
            .unwrap()
    );
    assert_eq!(observed.runtime_sha256(), hash(b"new"));
    assert_eq!(observed.runtime_size(), 3);
    assert_eq!(std::fs::read(&installed).unwrap(), b"old");
    assert_eq!(std::fs::read(selections.path()).unwrap(), before);
    let e = entry(root.path(), &observed);
    for (file, mode) in [
        ("runtime", 0o700),
        ("manifest.json", 0o600),
        ("receipt.json", 0o600),
    ] {
        let m = std::fs::metadata(e.join(file)).unwrap();
        assert_eq!(m.mode() & 0o777, mode);
        assert_eq!(m.nlink(), 1);
    }
    assert_eq!(std::fs::read_dir(e.parent().unwrap()).unwrap().count(), 1);
}
#[tokio::test]
async fn strict_revision_and_selection_fences_no_silent_rescope() {
    let (root, _selection_root, selections, store) = stores();
    assert!(store.stage(&selections, 0, candidate().await).is_err());
    assert!(store.stage(&selections, 2, candidate().await).is_err());
    assert!(!root.path().join("v1").exists());
    let observed = store.stage(&selections, 1, candidate().await).unwrap();
    let receipt = std::fs::read(entry(root.path(), &observed).join("receipt.json")).unwrap();
    selections.compare_and_swap(1, selection()).unwrap();
    assert!(store.stage(&selections, 1, candidate().await).is_err());
    let rebound = store.stage(&selections, 2, candidate().await).unwrap();
    assert_eq!(rebound.selection_revision(), 2);
    assert_eq!(
        std::fs::read_dir(entry(root.path(), &observed).parent().unwrap())
            .unwrap()
            .count(),
        2
    );
    assert_eq!(
        store
            .observe(
                TARGET,
                observed.manifest_digest(),
                observed.selection_revision()
            )
            .unwrap()
            .selection_revision(),
        1
    );
    assert_eq!(
        std::fs::read(entry(root.path(), &observed).join("receipt.json")).unwrap(),
        receipt
    );
    let other = parse_source_url("https://github.com/JegernOUTT/refact", Channel::Preview).unwrap();
    selections.compare_and_swap(2, other).unwrap();
    assert!(store.stage(&selections, 3, candidate().await).is_err());

    // An empty destination isolates selection equality from existing-receipt revision checks.
    let fresh_root = scratch();
    let fresh = StageStore::open(&fresh_root.path().canonicalize().unwrap()).unwrap();
    let selection_before = std::fs::read(selections.path()).unwrap();
    assert_eq!(
        fresh.stage(&selections, 3, candidate().await).unwrap_err(),
        "candidate selection changed"
    );
    assert_eq!(std::fs::read_dir(fresh_root.path()).unwrap().count(), 0);
    assert_eq!(std::fs::read(selections.path()).unwrap(), selection_before);
}
// Preserve names, inode/link identity, permissions, bytes, and symlink destinations without
// following links. This detects repair, replacement, unlinking, and leftover temporary entries.
fn cache_snapshot(root: &Path) -> Vec<(PathBuf, u64, u64, u32, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, u64, u64, u32, Vec<u8>)>) {
        let m = std::fs::symlink_metadata(path).unwrap();
        let payload = if m.file_type().is_symlink() {
            std::fs::read_link(path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if m.is_file() {
            std::fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        out.push((
            path.strip_prefix(root).unwrap().to_owned(),
            m.ino(),
            m.nlink(),
            m.mode(),
            payload,
        ));
        if m.is_dir() {
            let mut children: Vec<_> = std::fs::read_dir(path)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            children.sort();
            for child in children {
                visit(root, &child, out);
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out
}
#[tokio::test]
async fn tamper_links_corrupt_existing_are_rejected_without_repair() {
    for mode in [
        "runtime",
        "manifest",
        "receipt",
        "oversize_receipt",
        "hardlink",
        "symlink",
        "directory_symlink",
    ] {
        let (root, _selection_root, selections, store) = stores();
        let observed = store.stage(&selections, 1, candidate().await).unwrap();
        let e = entry(root.path(), &observed);
        match mode {
            "runtime" => std::fs::write(e.join("runtime"), b"bad").unwrap(),
            "manifest" => {
                let p = e.join("manifest.json");
                let mut b = std::fs::read(&p).unwrap();
                b.push(b' ');
                std::fs::write(p, b).unwrap();
            }
            "receipt" => std::fs::write(e.join("receipt.json"), b"{}").unwrap(),
            "oversize_receipt" => std::fs::write(e.join("receipt.json"), vec![b' '; 2049]).unwrap(),
            "hardlink" => std::fs::hard_link(e.join("runtime"), root.path().join("alias")).unwrap(),
            "symlink" => {
                std::fs::rename(e.join("runtime"), e.join("original")).unwrap();
                symlink("original", e.join("runtime")).unwrap();
            }
            _ => {
                let moved = e.with_file_name("moved");
                std::fs::rename(&e, &moved).unwrap();
                symlink("moved", &e).unwrap();
            }
        }
        let corrupted = cache_snapshot(root.path());
        assert!(
            store
                .observe(
                    TARGET,
                    observed.manifest_digest(),
                    observed.selection_revision()
                )
                .is_err(),
            "{mode}"
        );
        assert_eq!(
            cache_snapshot(root.path()),
            corrupted,
            "observe modified {mode}"
        );
        assert!(
            store.stage(&selections, 1, candidate().await).is_err(),
            "{mode}"
        );
        assert_eq!(
            cache_snapshot(root.path()),
            corrupted,
            "stage modified {mode}"
        );
        assert!(std::fs::symlink_metadata(&e).is_ok());
        if mode == "runtime" {
            assert_eq!(std::fs::read(e.join("runtime")).unwrap(), b"bad");
        }
    }
}
#[tokio::test]
async fn parallel_two_threads_publish_one_complete_entry() {
    let (root, _selection_root, selections, _store) = stores();
    let first = candidate().await;
    let second = candidate().await;
    // Distinct selection locks prevent selection fencing from serializing the publishers;
    // they contend on the shared stage lock itself, with identical selection and revision.
    let second_state = scratch();
    let second_selections = SelectionStore::new(
        second_state
            .path()
            .canonicalize()
            .unwrap()
            .join("selection.json"),
    )
    .unwrap();
    second_selections.compare_and_swap(0, selection()).unwrap();
    let first_path = selections.path().to_owned();
    let second_path = second_selections.path().to_owned();
    let root_path = root.path().canonicalize().unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [(first, first_path), (second, second_path)]
        .into_iter()
        .map(|(candidate, path)| {
            let root_path = root_path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = StageStore::open(&root_path).unwrap();
                let selections = SelectionStore::new(path).unwrap();
                barrier.wait();
                store.stage(&selections, 1, candidate).unwrap()
            })
        })
        .collect();
    let observations: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(observations[0], observations[1]);
    let e = entry(root.path(), &observations[0]);
    assert_eq!(std::fs::read_dir(e.parent().unwrap()).unwrap().count(), 1);
}
#[tokio::test]
async fn root_policy_pinned_identity_and_key_validation() {
    let (root, _selection_root, selections, initial_store) = stores();
    let path = root.path().canonicalize().unwrap();
    let observation = initial_store
        .stage(&selections, 1, candidate().await)
        .unwrap();
    assert_eq!(
        initial_store
            .observe(TARGET, observation.manifest_digest(), 1)
            .unwrap(),
        observation
    );
    drop(initial_store);
    assert!(StageStore::open(&path.join("absent")).is_err());
    let alias_root = scratch();
    let alias = alias_root.path().join("alias");
    symlink(&path, &alias).unwrap();
    assert!(StageStore::open(&alias).is_err());
    std::fs::remove_file(alias).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(StageStore::open(&path).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = StageStore::open(&path).unwrap();
    assert!(store.observe("../escape", &"a".repeat(64), 1).is_err());
    assert!(store.observe(TARGET, "../escape", 1).is_err());
    assert!(store
        .observe(TARGET, observation.manifest_digest(), 0)
        .is_err());
    assert_eq!(
        store
            .observe(TARGET, observation.manifest_digest(), 1)
            .unwrap(),
        observation
    );
    // Move a proven readable root, then replace its name.
    let outer = scratch();
    let relocated = outer.path().join("relocated");
    std::fs::rename(&path, &relocated).unwrap();
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(store
        .observe(TARGET, observation.manifest_digest(), 1)
        .is_err());
    assert!(store.stage(&selections, 1, candidate().await).is_err());
    assert_eq!(
        std::fs::read(
            relocated
                .join("v1")
                .join(TARGET)
                .join(observation.manifest_digest())
                .join("1")
                .join("runtime")
        )
        .unwrap(),
        b"new"
    );
}
