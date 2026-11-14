use super::*;
use refact_self_update::public_release::{parse_source_url, PublicResponse, ReadFuture};
use refact_self_update::runtime_manifest::Channel;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

fn fixture() -> (tempfile::TempDir, RuntimeCoordinator) {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let root = dir.path().canonicalize().unwrap();
    let core = RuntimeCoordinator::with_host_data(
        root.join("config/runtime-selection.json"),
        root.join("daemon/runtime-staging"),
        "x86_64-unknown-linux-gnu".into(),
        (1, 1, 1),
    )
    .unwrap();
    (dir, core)
}
fn selection() -> RuntimeSelection {
    parse_source_url(
        "https://github.com/JegernOUTT/refact/releases/tag/engine/v1.2.3",
        Channel::Preview,
    )
    .unwrap()
}
fn select(core: &RuntimeCoordinator) {
    core.select(SelectRequest {
        expected_revision: 0,
        selection: selection(),
    })
    .unwrap();
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn status_missing_is_read_only_and_explicitly_unknown() {
    let (dir, core) = fixture();
    let response = core.status(None);
    assert_eq!(response.status.selection_revision, 0);
    assert!(matches!(response.status.selected, Observation::Unknown));
    assert!(matches!(response.status.installed, Observation::Unknown));
    assert!(matches!(response.status.staged, Observation::Unknown));
    assert!(response
        .diagnostics
        .iter()
        .any(|d| d.contains("selection absent")));
    assert!(!response.activation_supported && !response.restart_dispatched);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert_eq!(compiled_epochs().unwrap(), (1, 1, 1));
}
#[cfg(unix)]
#[test]
fn selection_is_cas_and_corruption_is_not_absence() {
    let (_dir, core) = fixture();
    select(&core);
    assert!(core
        .select(SelectRequest {
            expected_revision: 0,
            selection: selection()
        })
        .is_err());
    assert_eq!(core.load_selection().unwrap().unwrap().revision(), 1);
    std::fs::write(&core.selection_path, b"{").unwrap();
    assert!(core.load_selection().is_err());
    assert!(core
        .status(None)
        .diagnostics
        .iter()
        .any(|d| d.contains("selection unreadable")));
    assert!(core
        .select(SelectRequest {
            expected_revision: 0,
            selection: selection()
        })
        .is_err());
    assert_eq!(std::fs::read(&core.selection_path).unwrap(), b"{");
}
#[test]
fn wire_rejects_targets_paths_and_approval_flags() {
    for extra in ["target", "stage_root", "approved", "activate", "restart"] {
        let mut request = json!({"expected_revision":1});
        request[extra] = json!(true);
        assert!(serde_json::from_value::<RevisionRequest>(request).is_err());
    }
}
#[cfg(unix)]
#[test]
fn unsafe_existing_config_and_symlink_ancestors_are_never_repaired() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let (dir, core) = fixture();
    let config = core.selection_path.parent().unwrap();
    std::fs::create_dir(config).unwrap();
    std::fs::set_permissions(config, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(core
        .select(SelectRequest {
            expected_revision: 0,
            selection: selection()
        })
        .is_err());
    assert_eq!(
        std::fs::metadata(config).unwrap().permissions().mode() & 0o777,
        0o777
    );
    std::fs::remove_dir(config).unwrap();
    symlink(dir.path().join("absent"), config).unwrap();
    assert!(core.load_selection().is_err());
    assert!(core
        .select(SelectRequest {
            expected_revision: 0,
            selection: selection()
        })
        .is_err());
    assert!(!dir.path().join("absent").exists());
}

struct Fake {
    rows: Mutex<VecDeque<(String, Vec<u8>)>>,
    on_archive: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl PublicTransport for Fake {
    fn get<'a>(&'a self, url: &'a str, _: bool, cap: usize) -> ReadFuture<'a> {
        Box::pin(async move {
            let (suffix, body) = self
                .rows
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected network request");
            assert!(url.ends_with(&suffix), "{url} did not end in {suffix}");
            assert!(body.len() <= cap);
            if suffix == "releases/assets/2" {
                if let Some(f) = &self.on_archive {
                    f();
                }
            }
            Ok(PublicResponse {
                status: 200,
                location: None,
                body,
            })
        })
    }
}
fn fake(include_archive: bool, on_archive: Option<Arc<dyn Fn() + Send + Sync>>) -> Fake {
    // A minimal regular-file ustar archive, constructed without a test-only tar dependency.
    let mut tar = vec![0u8; 2048];
    tar[..6].copy_from_slice(b"refact");
    tar[100..108].copy_from_slice(b"0000755\0");
    tar[108..116].copy_from_slice(b"0000000\0");
    tar[116..124].copy_from_slice(b"0000000\0");
    tar[124..136].copy_from_slice(b"00000000003\0");
    tar[136..148].copy_from_slice(b"00000000000\0");
    tar[148..156].fill(b' ');
    tar[156] = b'0';
    tar[257..263].copy_from_slice(b"ustar\0");
    tar[263..265].copy_from_slice(b"00");
    let sum: u32 = tar[..512].iter().map(|b| *b as u32).sum();
    tar[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    tar[512..515].copy_from_slice(b"new");
    use std::io::Write;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar).unwrap();
    let archive = gzip.finish().unwrap();
    let mut manifest: Value = serde_json::from_slice(include_bytes!(
        "../../crates/refact-self-update/tests/fixtures/runtime-manifest-v1.json"
    ))
    .unwrap();
    manifest["repository"] = json!({"owner":"JegernOUTT","name":"refact","id":1254216370u64});
    manifest["workflow"] = json!({"path":".github/workflows/plugin_build.yml","id":1});
    manifest["run"] = json!({"id":1,"attempt":1});
    manifest["source_sha"] = json!("a".repeat(40));
    manifest["build"]["version"] = json!("1.2.3");
    manifest["component_versions"]["runtime"] = json!("1.2.3");
    manifest["compatibility"] = json!({"daemon_api":{"min":1,"max":1},"worker_api":{"min":1,"max":1},"host_bridge":{"min":1,"max":1}});
    manifest["artifacts"] = json!([{"component":"runtime","target":"x86_64-unknown-linux-gnu","filename":"runtime.tar.gz","format":"tar_gz","size":archive.len(),"sha256":hash(&archive),"extracted_files":[{"path":"refact","size":3,"sha256":hash(b"new")}]}]);
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let repo = json!({"id":1254216370u64,"full_name":"JegernOUTT/refact"});
    let mut rows = vec![
        ("/refact", serde_json::to_vec(&repo).unwrap()),
        ("releases/tags/engine%2Fv1%2E2%2E3", serde_json::to_vec(&json!({"id":99,"tag_name":"engine/v1.2.3","draft":false,"prerelease":true,"assets":[
            {"id":1,"name":"runtime-manifest.json","size":bytes.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&bytes))},
            {"id":2,"name":"runtime.tar.gz","size":archive.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&archive))}]})).unwrap()),
        ("git/ref/tags/engine%2Fv1%2E2%2E3", serde_json::to_vec(&json!({"ref":"refs/tags/engine/v1.2.3","object":{"type":"commit","sha":"a".repeat(40)}})).unwrap()),
        ("releases/assets/1", bytes),
        ("actions/workflows/plugin%5Fbuild%2Eyml", serde_json::to_vec(&json!({"id":1,"path":".github/workflows/plugin_build.yml","state":"active"})).unwrap()),
        ("actions/runs/1/attempts/1", serde_json::to_vec(&json!({"id":1,"run_attempt":1,"workflow_id":1,"path":".github/workflows/plugin_build.yml","head_sha":"a".repeat(40),"event":"workflow_dispatch","status":"completed","conclusion":"success","repository":repo,"head_repository":repo})).unwrap()),
    ];
    if include_archive {
        rows.push(("releases/assets/2", archive));
    }
    Fake {
        rows: Mutex::new(rows.into_iter().map(|(p, b)| (p.into(), b)).collect()),
        on_archive,
    }
}
#[cfg(unix)]
#[tokio::test]
async fn resolve_is_read_only_and_plan_covers_runtime_as_one_unit() {
    let (_dir, core) = fixture();
    select(&core);
    let provider = fake(false, None);
    let plan = core
        .resolve(
            &provider,
            RevisionRequest {
                expected_revision: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        plan.together,
        ["runtime_cli", "daemon", "workers", "embedded_gui"]
    );
    assert_eq!(plan.ide_owned_not_updated, ["vscode", "jetbrains"]);
    assert_eq!(plan.qualification, "untested");
    assert!(!plan.activation_supported && !plan.restart_dispatched);
    assert!(!core.stage_root.exists());
    assert!(provider.rows.lock().unwrap().is_empty());
}
#[cfg(target_os = "linux")]
#[tokio::test]
async fn stage_rehash_status_and_reselected_release_keep_old_revisions() {
    let (_dir, core) = fixture();
    select(&core);
    let first = core
        .stage(
            &fake(true, None),
            RevisionRequest {
                expected_revision: 1,
            },
        )
        .await
        .unwrap();
    let key = StageKey {
        manifest_digest: first.staged.manifest_digest.clone(),
        selection_revision: 1,
    };
    let response = core.status(Some(key.clone()));
    assert!(matches!(response.status.staged, Observation::Known(_)));
    assert!(matches!(response.status.installed, Observation::Unknown));
    match response.status.running {
        Observation::Known(p) => {
            assert_eq!(p.pid, std::process::id());
            assert!(matches!(p.reported_versions, Observation::Unknown));
        }
        Observation::Unknown => panic!("host PID should be observed"),
    }
    core.select(SelectRequest {
        expected_revision: 1,
        selection: selection(),
    })
    .unwrap();
    assert!(matches!(
        core.status(Some(key)).status.staged,
        Observation::Unknown
    ));
    let second = core
        .stage(
            &fake(true, None),
            RevisionRequest {
                expected_revision: 2,
            },
        )
        .await
        .unwrap();
    assert_eq!(second.staged.manifest_digest, first.staged.manifest_digest);
    assert_eq!(second.staged.selection_revision, 2);
    assert!(StageStore::open(&core.stage_root)
        .unwrap()
        .observe(&core.target, &first.staged.manifest_digest, 1)
        .is_ok());
    // Read status may fail closed on an interrupted cache; it must not recreate missing locks.
    let lock = core.stage_root.join(".stage.lock");
    std::fs::remove_file(&lock).unwrap();
    let response = core.status(Some(StageKey {
        manifest_digest: second.staged.manifest_digest,
        selection_revision: 2,
    }));
    assert!(matches!(response.status.staged, Observation::Unknown));
    assert!(!lock.exists());
}
#[cfg(target_os = "linux")]
#[tokio::test]
async fn selection_change_during_download_rejects_before_stage_root_creation() {
    let (_dir, core) = fixture();
    select(&core);
    let changed = core.clone();
    let provider = fake(
        true,
        Some(Arc::new(move || {
            changed
                .select(SelectRequest {
                    expected_revision: 1,
                    selection: selection(),
                })
                .unwrap();
        })),
    );
    let error = core
        .stage(
            &provider,
            RevisionRequest {
                expected_revision: 1,
            },
        )
        .await
        .unwrap_err();
    assert!(error.contains("revision conflict"));
    assert!(!core.stage_root.exists());
}
#[tokio::test]
async fn missing_selection_fails_before_network_on_all_platforms() {
    let (_dir, core) = fixture();
    let provider = Fake {
        rows: Mutex::new(VecDeque::new()),
        on_archive: None,
    };
    assert!(core
        .resolve(
            &provider,
            RevisionRequest {
                expected_revision: 1
            }
        )
        .await
        .is_err());
    assert!(core
        .stage(
            &provider,
            RevisionRequest {
                expected_revision: 1
            }
        )
        .await
        .is_err());
    assert!(!core.selection_path.parent().unwrap().exists());
    assert!(!core.stage_root.exists());
}
#[cfg(unix)]
#[tokio::test]
async fn stale_and_local_selection_fail_before_network() {
    let (dir, core) = fixture();
    let provider = Fake {
        rows: Mutex::new(VecDeque::new()),
        on_archive: None,
    };
    let local = RuntimeSelection::from_json(
        &serde_json::to_vec(&json!({
            "kind": "local",
            "absolute_path": dir.path().canonicalize().unwrap().join("unmanaged/refact"),
        }))
        .unwrap(),
    )
    .unwrap();
    core.select(SelectRequest {
        expected_revision: 0,
        selection: local,
    })
    .unwrap();
    assert!(core
        .resolve(
            &provider,
            RevisionRequest {
                expected_revision: 0
            }
        )
        .await
        .is_err());
    assert!(core
        .stage(
            &provider,
            RevisionRequest {
                expected_revision: 1
            }
        )
        .await
        .unwrap_err()
        .contains("unmanaged"));
}

#[cfg(not(unix))]
#[tokio::test]
async fn unsupported_mutation_security_fails_closed_without_creating_state() {
    let (dir, core) = fixture();
    let error = core
        .select(SelectRequest {
            expected_revision: 0,
            selection: selection(),
        })
        .unwrap_err();
    assert!(error.contains("runtime mutation directory security unsupported"));
    assert!(!core.selection_path.parent().unwrap().exists());
    assert!(!core.stage_root.exists());
    assert!(matches!(
        core.status(None).status.selected,
        Observation::Unknown
    ));
    let provider = Fake {
        rows: Mutex::new(VecDeque::new()),
        on_archive: None,
    };
    assert!(core
        .resolve(
            &provider,
            RevisionRequest {
                expected_revision: 1
            }
        )
        .await
        .is_err());
    assert!(core
        .stage(
            &provider,
            RevisionRequest {
                expected_revision: 1
            }
        )
        .await
        .is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert!(matches!(
        core.status(None).status.selected,
        Observation::Unknown
    ));
}
