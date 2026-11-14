//! Hermetic: transports return in-memory fixtures and record every attempted URL.
use refact_self_update::public_release::*;
use refact_self_update::runtime_artifact::ArtifactLimits;
use refact_self_update::runtime_manifest::{Channel, Qualification};
use refact_self_update::runtime_selection::{RuntimeSelection, SelectionSpec};
use refact_self_update::public_qualification::TrustedQualificationPolicy;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, io::Cursor, sync::Mutex};
const BASE: &str = "https://api.github.com/repos/JegernOUTT/refact";
const TARGET: &str = "x86_64-unknown-linux-gnu";
const PATH: &str = ".github/workflows/plugin_build.yml";
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn repo() -> Value {
    json!({"id":1254216370u64,"full_name":"JegernOUTT/refact"})
}
fn selection() -> RuntimeSelection {
    parse_source_url(
        "https://github.com/JegernOUTT/refact/releases/tag/engine/v1.2.3",
        Channel::Preview,
    )
    .unwrap()
}
struct Fake {
    responses: Mutex<VecDeque<(String, bool, usize, PublicResponse)>>,
    calls: Mutex<Vec<String>>,
}
impl Fake {
    fn new() -> Self {
        Self {
            responses: Mutex::new(VecDeque::new()),
            calls: Mutex::new(vec![]),
        }
    }
    fn push(&self, path: &str, asset: bool, cap: usize, body: Vec<u8>) {
        let url = if path.is_empty() {
            BASE.to_owned()
        } else {
            format!("{BASE}/{path}")
        };
        self.responses.lock().unwrap().push_back((
            url,
            asset,
            cap,
            PublicResponse {
                status: 200,
                location: None,
                body,
            },
        ));
    }
    fn api(&self, path: &str, value: Value) {
        self.push(
            path,
            false,
            1024 * 1024,
            serde_json::to_vec(&value).unwrap(),
        );
    }
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}
impl PublicTransport for Fake {
    fn get<'a>(&'a self, url: &'a str, asset: bool, cap: usize) -> ReadFuture<'a> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(url.into());
            let (expected, expected_asset, expected_cap, response) = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected I/O / fallback");
            assert_eq!(url, expected);
            assert_eq!(asset, expected_asset);
            assert_eq!(cap, expected_cap);
            Ok(response)
        })
    }
}
fn manifest(archive: &[u8]) -> Value {
    json!({"schema_version":1,"repository":{"owner":"JegernOUTT","name":"refact","id":1254216370u64},
      "source_sha":"a".repeat(40),"workflow":{"path":PATH,"id":296686373},"run":{"id":123,"attempt":2},
      "build":{"version":"1.2.3","channel":"preview"},
      "build_inputs":{"rust_toolchain":"1.85.0","node_version":"22","profile":"release",
        "cargo_lock_sha256":"b".repeat(64),"javascript_locks_sha256":"c".repeat(64),"config_sha256":"d".repeat(64)},
      "component_versions":{"runtime":"1.2.3","gui":"1","vscode":"1","jetbrains":"1"},
      "compatibility":{"daemon_api":{"min":1,"max":1},"worker_api":{"min":1,"max":1},"host_bridge":{"min":1,"max":1}},
      "artifacts":[{"component":"runtime","target":TARGET,"filename":"runtime.tar.gz","format":"tar_gz",
        "size":archive.len(),"sha256":hash(archive),"extracted_files":[{"path":"refact","size":3,"sha256":hash(b"new")}]}],
      "tests":"not_run"})
}
fn archive() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut h = tar::Header::new_gnu();
    h.set_size(3);
    h.set_mode(0o755);
    h.set_cksum();
    builder
        .append_data(&mut h, "refact", Cursor::new(b"new"))
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}
fn run() -> Value {
    json!({"id":123,"run_attempt":2,"workflow_id":296686373,"path":PATH,
    "head_sha":"a".repeat(40),"event":"workflow_dispatch","status":"completed","conclusion":"success","repository":repo(),"head_repository":repo()})
}
fn fixture(
    m: Value,
    r: Value,
    mutate_release: impl FnOnce(&mut Value),
    mutate_tag: impl FnOnce(&mut Value),
    mutate_workflow: impl FnOnce(&mut Value),
) -> (Fake, Vec<u8>) {
    let archive = archive();
    let bytes = serde_json::to_vec(&m).unwrap();
    let f = Fake::new();
    f.api("", repo());
    let mut release = json!({"id":99,"tag_name":"engine/v1.2.3","draft":false,"prerelease":m["build"]["channel"] != "stable",
      "assets":[{"id":1,"name":"runtime-manifest.json","size":bytes.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&bytes))},
        {"id":2,"name":"runtime.tar.gz","size":archive.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&archive))}]});
    mutate_release(&mut release);
    f.api("releases/tags/engine%2Fv1%2E2%2E3", release);
    let mut tag =
        json!({"ref":"refs/tags/engine/v1.2.3","object":{"type":"commit","sha":"a".repeat(40)}});
    mutate_tag(&mut tag);
    f.api("git/ref/tags/engine%2Fv1%2E2%2E3", tag);
    f.push("releases/assets/1", true, 256 * 1024, bytes);
    let mut wf = json!({"id":m["workflow"]["id"],"path":m["workflow"]["path"],"state":"active"});
    mutate_workflow(&mut wf);
    let filename = m["workflow"]["path"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    let encoded =
        percent_encoding::utf8_percent_encode(filename, percent_encoding::NON_ALPHANUMERIC)
            .to_string();
    f.api(&format!("actions/workflows/{encoded}"), wf);
    f.api("actions/runs/123/attempts/2", r);
    (f, archive)
}
async fn resolve(f: &Fake) -> Result<ResolvedPublicRelease> {
    resolve_public_release(f, &selection(), TARGET, (1, 1, 1)).await
}

#[test]
fn strict_sources() {
    for input in [
        "/tmp/refact",
        "file:///tmp/refact",
        "http://github.com/JegernOUTT/refact",
        "https://github.com:443/JegernOUTT/refact",
        "https://evil.example/JegernOUTT/refact",
        "https://github.com/other/refact",
        "https://user@github.com/JegernOUTT/refact",
        "https://github.com/JegernOUTT/refact?token=x",
        "https://github.com/JegernOUTT/refact#x",
        "https://github.com/JegernOUTT/refact/releases/tag/../x",
        "https://github.com/JegernOUTT/refact/releases/tag/%2e%2e/x",
        "https://github.com/JegernOUTT/refact/releases/tag/x%2fy",
        "https://github.com/JegernOUTT/refact/actions/runs/0",
    ] {
        assert!(
            parse_source_url(input, Channel::Preview).is_err(),
            "{input}"
        );
    }
    assert!(selection().is_managed());
    assert!(parse_source_url(
        "https://github.com/JegernOUTT/refact/actions/runs/123/attempts/2",
        Channel::Preview
    )
    .is_ok());
}
#[tokio::test]
async fn unsupported_sources_fail_before_io() {
    let f = Fake::new();
    for (url, expected) in [(
        "https://github.com/JegernOUTT/refact/actions/runs/123",
        PublicReleaseError::AuthenticationRequired,
    )] {
        let s = parse_source_url(url, Channel::Preview).unwrap();
        assert_eq!(
            resolve_public_release(&f, &s, TARGET, (1, 1, 1))
                .await
                .unwrap_err(),
            expected
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let local = RuntimeSelection::try_from(SelectionSpec::Local {
        absolute_path: dir.path().canonicalize().unwrap().to_str().unwrap().into(),
    })
    .unwrap();
    assert!(resolve_public_release(&f, &local, TARGET, (1, 1, 1))
        .await
        .is_err());
    let mut foreign: Value = serde_json::to_value(selection()).unwrap();
    foreign["repository"]["id"] = json!(1);
    let s = RuntimeSelection::from_json(&serde_json::to_vec(&foreign).unwrap()).unwrap();
    assert!(resolve_public_release(&f, &s, TARGET, (1, 1, 1))
        .await
        .is_err());
    assert_eq!(f.count(), 0);
}
#[tokio::test]
async fn immutable_preview_resolves_and_verifies_in_memory() {
    let (f, a) = fixture(manifest(&archive()), run(), |_| {}, |_| {}, |_| {});
    f.push("releases/assets/2", true, a.len(), a);
    let resolved = resolve(&f).await.unwrap();
    assert_eq!(resolved.qualification(), Qualification::Untested);
    assert_eq!(resolved.source_evidence().run.attempt, 2);
    assert_eq!(resolved.release_evidence().tag, "engine/v1.2.3");
    let verified = fetch_verified_candidate(
        &f,
        resolved,
        &selection(),
        TARGET,
        (1, 1, 1),
        ArtifactLimits {
            archive_bytes: 1024 * 1024,
            runtime_bytes: 1024,
        },
    )
    .await
    .unwrap();
    assert_eq!(verified.artifact().bytes(), b"new");
    assert_eq!(f.count(), 7);
}
#[tokio::test]
async fn adversarial_run_metadata_never_falls_back() {
    for (key, value) in [
        ("run_attempt", json!(1)),
        ("id", json!(124)),
        ("workflow_id", json!(1)),
        ("path", json!(".github/workflows/untrusted.yml")),
        ("event", json!("pull_request")),
        ("status", json!("in_progress")),
        ("conclusion", json!("failure")),
        ("head_repository", json!({"id":1,"full_name":"fork/refact"})),
        (
            "repository",
            json!({"id":1,"full_name":"JegernOUTT/refact"}),
        ),
        ("head_sha", json!("b".repeat(40))),
    ] {
        let mut r = run();
        r[key] = value;
        let (f, _) = fixture(manifest(&archive()), r, |_| {}, |_| {}, |_| {});
        let error = resolve(&f).await.unwrap_err();
        if key == "head_sha" {
            assert_eq!(error, PublicReleaseError::SelectedShaDispatchUnsupported);
        } else {
            assert!(matches!(error, PublicReleaseError::Rejected(_)), "{key}");
        }
        assert_eq!(f.count(), 6);
    }
}
#[tokio::test]
async fn release_tag_workflow_and_digest_rejections() {
    for key in ["draft", "tag_name", "assets"] {
        let (f, _) = fixture(
            manifest(&archive()),
            run(),
            |r| match key {
                "draft" => r[key] = json!(true),
                "tag_name" => r[key] = json!("other"),
                _ => r[key] = json!([]),
            },
            |_| {},
            |_| {},
        );
        assert!(resolve(&f).await.is_err(), "{key}");
    }
    let (f, _) = fixture(
        manifest(&archive()),
        run(),
        |_| {},
        |t| t["ref"] = json!("refs/tags/wrong"),
        |_| {},
    );
    assert!(matches!(
        resolve(&f).await.unwrap_err(),
        PublicReleaseError::Rejected(_)
    ));
    assert_eq!(f.count(), 3);
    let (f, _) = fixture(
        manifest(&archive()),
        run(),
        |_| {},
        |t| t["object"]["sha"] = json!("b".repeat(40)),
        |_| {},
    );
    assert!(resolve(&f).await.is_err());
    assert_eq!(f.count(), 4);
    let (f, _) = fixture(
        manifest(&archive()),
        run(),
        |_| {},
        |_| {},
        |w| w["id"] = json!(1),
    );
    assert!(resolve(&f).await.is_err());
    assert_eq!(f.count(), 6);
    let (f, _) = fixture(
        manifest(&archive()),
        run(),
        |r| r["assets"][0]["digest"] = json!(format!("sha256:{}", "0".repeat(64))),
        |_| {},
        |_| {},
    );
    assert!(resolve(&f).await.is_err());
    assert_eq!(f.count(), 4);
    let (f, _) = fixture(
        manifest(&archive()),
        run(),
        |r| r["assets"][1]["size"] = json!(1),
        |_| {},
        |_| {},
    );
    assert!(resolve(&f).await.is_err());
    assert_eq!(f.count(), 6);
}
#[tokio::test]
async fn stable_without_qualification_reference_is_rejected() {
    let mut m = manifest(&archive());
    m["build"]["channel"] = json!("stable");
    m["tests"] = json!("passed");
    m["workflow"]["path"] = json!(".github/workflows/plugin_release.yml");
    m["workflow"]["id"] = json!(304528468);
    let mut r = run();
    r["path"] = m["workflow"]["path"].clone();
    r["workflow_id"] = m["workflow"]["id"].clone();
    let (f, _) = fixture(m, r, |_| {}, |_| {}, |_| {});
    assert_eq!(
        resolve(&f).await.unwrap_err(),
        PublicReleaseError::Rejected("qualification reference required".into())
    );
    assert_eq!(f.count(), 6);
}
#[tokio::test]
async fn archive_hash_length_and_context_are_bound() {
    for mode in ["hash", "short", "oversize", "context", "budget"] {
        let (f, mut a) = fixture(manifest(&archive()), run(), |_| {}, |_| {}, |_| {});
        let resolved = resolve(&f).await.unwrap();
        let size = a.len();
        match mode {
            "hash" => a[0] ^= 1,
            "short" => {
                a.pop();
            }
            "oversize" => a.push(0),
            _ => {}
        }
        if !matches!(mode, "context" | "budget") {
            f.push("releases/assets/2", true, size, a);
        }
        let protocols = if mode == "context" {
            (2, 1, 1)
        } else {
            (1, 1, 1)
        };
        let limits = ArtifactLimits {
            archive_bytes: if mode == "budget" { 1 } else { 1024 * 1024 },
            runtime_bytes: 1024,
        };
        assert!(
            fetch_verified_candidate(&f, resolved, &selection(), TARGET, protocols, limits)
                .await
                .is_err(),
            "{mode}"
        );
        assert_eq!(
            f.count(),
            if matches!(mode, "context" | "budget") {
                6
            } else {
                7
            }
        );
    }
}
#[tokio::test]
async fn api_body_bound_and_redirect_no_fallback() {
    let f = Fake::new();
    f.push("", false, 1024 * 1024, vec![b' '; 1024 * 1024 + 1]);
    assert!(resolve(&f).await.is_err());
    assert_eq!(f.count(), 1);
    let f = Fake::new();
    f.responses.lock().unwrap().push_back((
        BASE.into(),
        false,
        1024 * 1024,
        PublicResponse {
            status: 302,
            location: Some("https://api.github.com/elsewhere".into()),
            body: vec![],
        },
    ));
    assert!(resolve(&f).await.is_err());
    assert_eq!(f.count(), 1);
}

#[tokio::test]
async fn asset_redirects_are_storage_only_and_bounded() {
    for location in [
        "https://evil.example/a",
        "http://objects.githubusercontent.com/a",
        "https://objects.githubusercontent.com:443/a",
        "HTTPS://objects.githubusercontent.com:443/a",
        "HTTPS://objects.githubusercontent.com/a",
        "https://user@objects.githubusercontent.com/a",
        "https://127.0.0.1/a",
        "https://objects.githubusercontent.com.evil.example/a",
        "/relative",
    ] {
        let (f, _) = fixture(manifest(&archive()), run(), |_| {}, |_| {}, |_| {});
        {
            let mut q = f.responses.lock().unwrap();
            q[3].3 = PublicResponse {
                status: 302,
                location: Some(location.into()),
                body: vec![],
            };
        }
        assert!(resolve(&f).await.is_err(), "{location}");
        assert_eq!(f.count(), 4);
    }
    for hops in [1, 3, 4] {
        let (f, _) = fixture(manifest(&archive()), run(), |_| {}, |_| {}, |_| {});
        {
            let mut q = f.responses.lock().unwrap();
            let original = q.remove(3).unwrap();
            let mut url = original.0.clone();
            for hop in 0..hops {
                let next = format!(
                    "https://release-assets.githubusercontent.com/asset/{hop}?signature=public"
                );
                q.insert(
                    3 + hop,
                    (
                        url,
                        true,
                        256 * 1024,
                        PublicResponse {
                            status: 302,
                            location: Some(next.clone()),
                            body: vec![],
                        },
                    ),
                );
                url = next;
            }
            q.insert(3 + hops, (url, true, 256 * 1024, original.3));
        }
        if hops <= 3 {
            assert!(resolve(&f).await.is_ok());
            assert_eq!(f.count(), 6 + hops);
        } else {
            assert!(resolve(&f).await.is_err());
            assert_eq!(f.count(), 7);
        }
    }
}

#[tokio::test]
async fn manifest_policy_and_protocols_fail_closed() {
    for field in ["qualification", "passed", "version", "protocol"] {
        let mut m = manifest(&archive());
        match field {
            "qualification" => m["qualification_ref"] = json!("unverified-report"),
            "passed" => m["tests"] = json!("passed"),
            "version" => m["build"]["version"] = json!("9.9.9"),
            _ => {}
        }
        let (f, _) = fixture(m, run(), |_| {}, |_| {}, |_| {});
        let protocols = if field == "protocol" {
            (2, 1, 1)
        } else {
            (1, 1, 1)
        };
        assert!(
            resolve_public_release(&f, &selection(), TARGET, protocols)
                .await
                .is_err(),
            "{field}"
        );
    }
    let mut m = manifest(&archive());
    m["build"]["channel"] = json!("stable");
    m["tests"] = json!("passed");
    m["workflow"]["path"] = json!(".github/workflows/plugin_release.yml");
    m["workflow"]["id"] = json!(304528468);
    let mut r = run();
    r["path"] = m["workflow"]["path"].clone();
    r["workflow_id"] = m["workflow"]["id"].clone();
    let (f, _) = fixture(m, r, |_| {}, |_| {}, |_| {});
    assert!(matches!(
        resolve_public_release(&f, &selection(), TARGET, (2, 1, 1))
            .await
            .unwrap_err(),
        PublicReleaseError::Rejected(_)
    ));
    assert_eq!(f.count(), 6);
}

#[tokio::test]
async fn candidate_selection_and_target_changes_do_not_fetch() {
    for mode in ["selection", "target"] {
        let (f, _) = fixture(manifest(&archive()), run(), |_| {}, |_| {}, |_| {});
        let resolved = resolve(&f).await.unwrap();
        let mut current: Value = serde_json::to_value(selection()).unwrap();
        if mode == "selection" {
            current["expected_sha"] = json!("a".repeat(40));
        }
        let current = RuntimeSelection::from_json(&serde_json::to_vec(&current).unwrap()).unwrap();
        let target = if mode == "target" {
            "aarch64-unknown-linux-gnu"
        } else {
            TARGET
        };
        let error = fetch_verified_candidate(
            &f,
            resolved,
            &current,
            target,
            (1, 1, 1),
            ArtifactLimits {
                archive_bytes: 1024 * 1024,
                runtime_bytes: 1024,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            PublicReleaseError::Rejected("candidate context changed".into())
        );
        assert_eq!(f.count(), 6);
    }
}

#[tokio::test]
async fn pinned_sha_and_manifest_digest_are_enforced() {
    for field in ["expected_sha", "manifest_digest"] {
        let (f, _) = fixture(manifest(&archive()), run(), |_| {}, |_| {}, |_| {});
        let mut pinned: Value = serde_json::to_value(selection()).unwrap();
        pinned[field] = json!("0".repeat(if field == "expected_sha" { 40 } else { 64 }));
        let pinned = RuntimeSelection::from_json(&serde_json::to_vec(&pinned).unwrap()).unwrap();
        let error = resolve_public_release(&f, &pinned, TARGET, (1, 1, 1))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PublicReleaseError::Rejected("pinned selection identity mismatch".into())
        );
        assert_eq!(f.count(), 6);
    }
}

#[tokio::test]
async fn release_asset_metadata_is_not_optional() {
    for index in [0, 1] {
        for mode in [
            "missing_digest",
            "wrong_digest",
            "duplicate",
            "zero_id",
            "zero_size",
            "unuploaded",
        ] {
            let (f, _) = fixture(
                manifest(&archive()),
                run(),
                |r| match mode {
                    "missing_digest" => {
                        r["assets"][index].as_object_mut().unwrap().remove("digest");
                    }
                    "wrong_digest" => {
                        r["assets"][index]["digest"] = json!(format!("sha256:{}", "0".repeat(64)))
                    }
                    "duplicate" => {
                        let duplicate = r["assets"][index].clone();
                        r["assets"].as_array_mut().unwrap().push(duplicate);
                    }
                    "zero_id" => r["assets"][index]["id"] = json!(0),
                    "zero_size" => r["assets"][index]["size"] = json!(0),
                    _ => r["assets"][index]["state"] = json!("new"),
                },
                |_| {},
                |_| {},
            );
            assert!(
                matches!(
                    resolve(&f).await.unwrap_err(),
                    PublicReleaseError::Rejected(_)
                ),
                "{index}:{mode}"
            );
            assert_eq!(
                f.count(),
                if index == 1 {
                    6
                } else if matches!(mode, "missing_digest" | "wrong_digest") {
                    4
                } else {
                    3
                }
            );
        }
    }
}

#[tokio::test]
async fn manifest_repository_and_workflow_are_independently_bound() {
    for mode in ["repository", "workflow_id", "workflow_path"] {
        let mut m = manifest(&archive());
        match mode {
            "repository" => m["repository"]["id"] = json!(1),
            "workflow_id" => m["workflow"]["id"] = json!(1),
            _ => m["workflow"]["path"] = json!(".github/workflows/unapproved.yml"),
        }
        let (f, _) = fixture(m, run(), |_| {}, |_| {}, |w| w["id"] = json!(296686373));
        assert!(
            matches!(
                resolve(&f).await.unwrap_err(),
                PublicReleaseError::Rejected(_)
            ),
            "{mode}"
        );
        assert_eq!(f.count(), if mode == "workflow_path" { 4 } else { 6 });
    }
}

fn inventory() -> Vec<u8> {
    serde_json::to_vec(&json!({"schema":1,"required_jobs":["engine"],"entries":[]})).unwrap()
}
fn policy() -> TrustedQualificationPolicy {
    TrustedQualificationPolicy::from_inventory_bytes(&inventory()).unwrap()
}
fn stable_fixture(mode: &str) -> Fake {
    let mut m = manifest(&archive());
    m["build"]["channel"] = json!("stable");
    m["tests"] = json!("passed");
    m["workflow"]["path"] = json!(".github/workflows/plugin_release.yml");
    m["workflow"]["id"] = json!(304528468);
    let bi: refact_self_update::runtime_manifest::BuildInputs =
        serde_json::from_value(m["build_inputs"].clone()).unwrap();
    let report = json!({"schema":1,"schema_version":1,"kind":"refact-tests-qualification","repository_id":"1254216370","run_id":"123","run_attempt":"2",
        "source_sha":"a".repeat(40),"build_inputs":m["build_inputs"],"build_inputs_digest":bi.digest(),
        "inputs":{"rust":bi.rust_toolchain,"node":bi.node_version,"java":"17","build_profile":bi.profile,"test_profile":"ci-test"},
        "inventory_sha256":hash(&inventory()),"results":[{"id":"engine","status":"PASS"}],"scope":"fixture","status":"PASS","tests":"passed",
        "coverage_complete":true,"excluded_coverage":[],"test_profile":"ci-test"});
    let report = serde_json::to_vec(&report).unwrap();
    m["qualification_ref"] = json!(format!(
        "github-artifact:1254216370/123/2/4242/{}",
        hash(&report)
    ));
    let mut r = run();
    r["path"] = m["workflow"]["path"].clone();
    r["workflow_id"] = m["workflow"]["id"].clone();
    let (f, _) = fixture(
        m,
        r,
        |release| {
            release["assets"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":3,"name":"tests-qualification.json",
        "size":report.len(),"state":"uploaded","digest":format!("sha256:{}",hash(&report))}))
        },
        |_| {},
        |_| {},
    );
    let mut returned = report;
    if mode == "tamper" {
        returned[0] ^= 1;
    }
    f.push("releases/assets/3", true, 1024 * 1024, returned);
    f.push(
        &format!(
            "contents/tools/dev/ci_test_inventory.json?ref={}",
            "a".repeat(40)
        ),
        false,
        1024 * 1024,
        if mode == "foreign_inventory" {
            b"{}".to_vec()
        } else {
            inventory()
        },
    );
    f
}
#[tokio::test]
async fn stable_public_report_and_source_inventory_are_bound() {
    for mode in ["valid", "tamper", "foreign_inventory", "shrink"] {
        let f = stable_fixture(mode);
        let p = if mode == "shrink" {
            TrustedQualificationPolicy::from_inventory_bytes(
                br#"{"schema":1,"required_jobs":["engine","gui"],"entries":[]}"#,
            )
            .unwrap()
        } else {
            policy()
        };
        let result =
            resolve_public_release_with_policy(&f, &selection(), TARGET, (1, 1, 1), &p).await;
        if mode == "valid" {
            let resolved = result.unwrap();
            assert_eq!(resolved.qualification(), Qualification::Tested);
            assert_eq!(
                resolved
                    .public_qualification()
                    .unwrap()
                    .reference()
                    .artifact_id,
                4242
            );
            assert_eq!(f.count(), 8);
        } else {
            assert!(result.is_err(), "{mode}");
            assert_eq!(f.count(), if mode == "tamper" { 7 } else { 8 });
        }
    }
}
#[tokio::test]
async fn annotated_tags_are_bounded_and_identity_checked() {
    for mode in ["valid", "cycle", "wrong_object", "depth", "invalid_sha"] {
        let (f, _) = fixture(
            manifest(&archive()),
            run(),
            |_| {},
            |t| {
                t["object"]["type"] = json!("tag");
                t["object"]["sha"] = json!("b".repeat(40));
            },
            |_| {},
        );
        let helper = Fake::new();
        for index in 0..if mode == "depth" { 4 } else { 1 } {
            let sha = if index == 0 {
                "b".repeat(40)
            } else {
                format!("{:040x}", index)
            };
            let next = if mode == "cycle" {
                "b".repeat(40)
            } else if mode == "invalid_sha" {
                "bad".into()
            } else if mode == "depth" {
                format!("{:040x}", index + 1)
            } else {
                "a".repeat(40)
            };
            helper.api(&format!("git/tags/{sha}"),json!({"sha":if mode=="wrong_object" {"c".repeat(40)} else {sha.clone()},
                "object":{"type":if matches!(mode,"cycle"|"depth") {"tag"} else {"commit"},"sha":next}}));
        }
        {
            let mut q = f.responses.lock().unwrap();
            for (index, row) in helper
                .responses
                .into_inner()
                .unwrap()
                .into_iter()
                .enumerate()
            {
                q.insert(3 + index, row);
            }
        }
        let result = resolve(&f).await;
        if mode == "valid" {
            assert!(result.is_ok());
            assert_eq!(f.count(), 7);
        } else {
            assert!(
                matches!(result.unwrap_err(), PublicReleaseError::Rejected(_)),
                "{mode}"
            );
        }
    }
}
fn channel_selection() -> RuntimeSelection {
    parse_source_url("https://github.com/JegernOUTT/refact", Channel::Preview).unwrap()
}
fn publication(id: u64, tag: &str, time: &str) -> Value {
    json!({"id":id,"tag_name":tag,"draft":false,"prerelease":true,"published_at":time})
}
fn prepend_listing(f: &Fake, rows: Value) {
    let prefix = Fake::new();
    prefix.api("", repo());
    prefix.api("releases?per_page=100&page=1", rows);
    let mut q = f.responses.lock().unwrap();
    for row in prefix.responses.into_inner().unwrap().into_iter().rev() {
        q.push_front(row);
    }
}
#[tokio::test]
async fn channels_use_publication_not_version_and_preserve_selection() {
    let (f, a) = fixture(manifest(&archive()), run(), |_| {}, |_| {}, |_| {});
    prepend_listing(
        &f,
        json!([
            publication(1, "engine/v999.0.0", "2025-01-01T00:00:00Z"),
            publication(99, "engine/v1.2.3", "2026-01-01T00:00:00Z")
        ]),
    );
    f.push("releases/assets/2", true, a.len(), a);
    let selected = channel_selection();
    let resolved = resolve_public_release_with_policy(&f, &selected, TARGET, (1, 1, 1), &policy())
        .await
        .unwrap();
    assert_eq!(resolved.selection(), &selected);
    assert_eq!(resolved.release_evidence().tag, "engine/v1.2.3");
    assert!(fetch_verified_candidate(
        &f,
        resolved,
        &selected,
        TARGET,
        (1, 1, 1),
        ArtifactLimits {
            archive_bytes: 1024 * 1024,
            runtime_bytes: 1024
        }
    )
    .await
    .is_ok());
    assert_eq!(f.count(), 9);
}
#[tokio::test]
async fn channels_incomplete_overlap_bad_dates_and_corruption_do_not_fallback() {
    for mode in ["incomplete", "overlap", "date", "corrupt", "not_found"] {
        let f = Fake::new();
        f.api("", repo());
        if matches!(mode, "incomplete" | "overlap") {
            for page in 1..=if mode == "overlap" { 2 } else { 3 } {
                let rows: Vec<Value> = (0..100)
                    .map(|i| {
                        publication(
                            if mode == "overlap" {
                                i + 1
                            } else {
                                (page - 1) * 100 + i + 1
                            },
                            &format!("engine/v{page}.{i}.0"),
                            "2026-01-01T00:00:00Z",
                        )
                    })
                    .collect();
                f.api(&format!("releases?per_page=100&page={page}"), json!(rows));
            }
        } else {
            f.api(
                "releases?per_page=100&page=1",
                if mode == "not_found" {
                    json!([])
                } else {
                    json!([publication(
                        99,
                        "engine/v1.2.3",
                        if mode == "date" {
                            "2025-02-29T00:00:00Z"
                        } else {
                            "2026-01-01T00:00:00Z"
                        }
                    )])
                },
            );
            if mode == "corrupt" {
                f.api("", repo());
                f.push(
                    "releases/tags/engine%2Fv1%2E2%2E3",
                    false,
                    1024 * 1024,
                    b"bad".to_vec(),
                );
            }
        }
        let error = resolve_public_release_with_policy(
            &f,
            &channel_selection(),
            TARGET,
            (1, 1, 1),
            &policy(),
        )
        .await
        .unwrap_err();
        match mode {
            "incomplete" | "overlap" => {
                assert_eq!(error, PublicReleaseError::ChannelResolutionIncomplete)
            }
            "not_found" => assert_eq!(error, PublicReleaseError::ChannelReleaseNotFound),
            _ => assert!(matches!(error, PublicReleaseError::Rejected(_))),
        }
        assert_eq!(
            f.count(),
            match mode {
                "incomplete" | "corrupt" => 4,
                "overlap" => 3,
                _ => 2,
            }
        );
    }
}

fn version_fixture(version: &str, channel: &str, id: u64) -> Fake {
    let mut m = manifest(&archive());
    m["build"]["version"] = json!(version);
    m["build"]["channel"] = json!(channel);
    let (f, _) = fixture(m, run(), |_| {}, |_| {}, |_| {});
    let tag = format!("engine/v{version}");
    let encoded =
        percent_encoding::utf8_percent_encode(&tag, percent_encoding::NON_ALPHANUMERIC).to_string();
    {
        let mut q = f.responses.lock().unwrap();
        q[1].0 = format!("{BASE}/releases/tags/{encoded}");
        let mut release: Value = serde_json::from_slice(&q[1].3.body).unwrap();
        release["id"] = json!(id);
        release["tag_name"] = json!(tag);
        q[1].3.body = serde_json::to_vec(&release).unwrap();
        q[2].0 = format!("{BASE}/git/ref/tags/{encoded}");
        let mut reference: Value = serde_json::from_slice(&q[2].3.body).unwrap();
        reference["ref"] = json!(format!("refs/tags/{tag}"));
        q[2].3.body = serde_json::to_vec(&reference).unwrap();
    }
    f
}
#[tokio::test]
async fn channel_opposite_skip_cap_and_workflow_binding() {
    for mode in ["skip", "cap", "workflow", "tie"] {
        let f = Fake::new();
        f.api("", repo());
        let count = if mode == "cap" {
            11
        } else if matches!(mode, "skip" | "tie") {
            2
        } else {
            1
        };
        let mut rows = Vec::new();
        for index in 0..count {
            rows.push(publication(
                99 + index,
                &format!("engine/v1.2.{index}"),
                "2026-01-01T00:00:00Z",
            ));
        }
        // Equal publication times choose greater numeric ID, regardless of input order.
        f.api("releases?per_page=100&page=1", json!(rows));
        let inspected = if mode == "cap" {
            10
        } else if mode == "tie" {
            1
        } else {
            count
        };
        for index in (count - inspected..count).rev() {
            let subject = version_fixture(
                &format!("1.2.{index}"),
                if mode == "cap" || (mode == "skip" && index == 1) {
                    "nightly"
                } else {
                    "preview"
                },
                99 + index,
            );
            f.responses
                .lock()
                .unwrap()
                .extend(subject.responses.into_inner().unwrap());
        }
        let mut selected: Value = serde_json::to_value(channel_selection()).unwrap();
        if mode == "workflow" {
            selected["workflow"] = json!({"path":PATH,"id":1});
        }
        let selected =
            RuntimeSelection::from_json(&serde_json::to_vec(&selected).unwrap()).unwrap();
        let result =
            resolve_public_release_with_policy(&f, &selected, TARGET, (1, 1, 1), &policy()).await;
        match mode {
            "cap" => assert_eq!(
                result.unwrap_err(),
                PublicReleaseError::ChannelResolutionIncomplete
            ),
            "workflow" => assert!(matches!(
                result.unwrap_err(),
                PublicReleaseError::Rejected(_)
            )),
            _ => assert_eq!(
                result.unwrap().release_evidence().tag,
                if mode == "tie" {
                    "engine/v1.2.1"
                } else {
                    "engine/v1.2.0"
                }
            ),
        }
        assert_eq!(f.count(), 2 + inspected as usize * 6);
    }
}
#[tokio::test]
async fn stable_report_metadata_rejected_without_fallback() {
    for mode in [
        "missing",
        "wrong_digest",
        "wrong_size",
        "duplicate",
        "foreign_ref",
    ] {
        let f = stable_fixture("valid");
        {
            let mut q = f.responses.lock().unwrap();
            let mut release: Value = serde_json::from_slice(&q[1].3.body).unwrap();
            match mode {
                "missing" => {
                    release["assets"].as_array_mut().unwrap().pop();
                }
                "wrong_digest" => {
                    release["assets"][2]["digest"] = json!(format!("sha256:{}", "0".repeat(64)))
                }
                "wrong_size" => release["assets"][2]["size"] = json!(1),
                "duplicate" => {
                    let duplicate = release["assets"][2].clone();
                    release["assets"].as_array_mut().unwrap().push(duplicate);
                }
                _ => {
                    let mut m: Value = serde_json::from_slice(&q[3].3.body).unwrap();
                    let old = m["qualification_ref"].as_str().unwrap();
                    m["qualification_ref"] = json!(old.replacen("1254216370", "1", 1));
                    q[3].3.body = serde_json::to_vec(&m).unwrap();
                    release["assets"][0]["size"] = json!(q[3].3.body.len());
                    release["assets"][0]["digest"] =
                        json!(format!("sha256:{}", hash(&q[3].3.body)));
                }
            }
            q[1].3.body = serde_json::to_vec(&release).unwrap();
        }
        assert!(
            matches!(
                resolve_public_release_with_policy(&f, &selection(), TARGET, (1, 1, 1), &policy())
                    .await
                    .unwrap_err(),
                PublicReleaseError::Rejected(_)
            ),
            "{mode}"
        );
        assert_eq!(f.count(), if mode == "wrong_size" { 7 } else { 6 });
    }
}
