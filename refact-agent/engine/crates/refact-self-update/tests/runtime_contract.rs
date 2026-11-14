use refact_self_update::runtime_manifest::*;
use refact_self_update::runtime_selection::*;
use refact_self_update::runtime_status::*;
use refact_self_update::selection_store::*;
use serde_json::{json, Value};
use std::sync::{Arc, Barrier};

const FIXTURE: &[u8] = include_bytes!("fixtures/runtime-manifest-v1.json");
#[test]
fn portable_catalog_paths_and_target_case_rules() {
    for name in [
        "CON",
        "nul.txt",
        "COM1.zip",
        "trailing.",
        "trailing ",
        "bad?.zip",
        "unicodé.zip",
    ] {
        let mut v = value();
        v["artifacts"][0]["filename"] = json!(name);
        assert!(
            serde_json::from_value::<RuntimeManifest>(v).is_err(),
            "{name}"
        );
    }
    let mut v = value();
    v["artifacts"][1]["filename"] = json!("REFACT-LINUX.TAR.GZ");
    assert!(serde_json::from_value::<RuntimeManifest>(v).is_err());
    for (target, format, accepted) in [
        ("x86_64-unknown-linux-gnu", "tar_gz", true),
        ("x86_64-apple-darwin", "tar_gz", false),
        ("x86_64-pc-windows-msvc", "zip", false),
    ] {
        let mut v = value();
        v["artifacts"][0]["target"] = json!(target);
        v["artifacts"][0]["format"] = json!(format);
        let mut duplicate = v["artifacts"][0]["extracted_files"][0].clone();
        duplicate["path"] = json!("bin/Refact");
        v["artifacts"][0]["extracted_files"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert_eq!(
            serde_json::from_value::<RuntimeManifest>(v).is_ok(),
            accepted
        );
    }
    #[cfg(windows)]
    for path in [r"C:\a\.\b", r"C:\a\..\b"] {
        assert!(RuntimeSelection::try_from(SelectionSpec::Local {
            absolute_path: path.into()
        })
        .is_err());
    }
}
#[test]
fn repeated_cas_has_one_revision_per_success() {
    let dir = scratch();
    let path = dir.path().canonicalize().unwrap().join("selection");
    let barrier = Arc::new(Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let p = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = SelectionStore::new(p).unwrap();
                let mut successes = 0;
                barrier.wait();
                for _ in 0..20 {
                    let revision = store.load().unwrap().map_or(0, |s| s.revision());
                    match store.compare_and_swap(revision, local()) {
                        Ok(_) => successes += 1,
                        Err(error) => assert_eq!(error, "selection revision conflict"),
                    }
                }
                successes
            })
        })
        .collect();
    let successes: u64 = threads.into_iter().map(|t| t.join().unwrap()).sum();
    assert_eq!(
        SelectionStore::new(path)
            .unwrap()
            .load()
            .unwrap()
            .unwrap()
            .revision(),
        successes
    );
}
#[test]
fn exact_selection_binding_never_falls_back() {
    let m = manifest(&value());
    let t = trusted(&m);
    let e = SourceEvidence {
        repository: m.repository().clone(),
        workflow: m.workflow().clone(),
        run: m.run().clone(),
        source_sha: m.source_sha().into(),
        status: "completed".into(),
        conclusion: "success".into(),
    };
    let run = json!({"kind":"pinned","repository":m.repository(),"source":{"kind":"run","id":m.run().id,"attempt":m.run().attempt},"expected_sha":m.source_sha(),"manifest_digest":m.digest()});
    let selection =
        |v: &Value| RuntimeSelection::from_json(&serde_json::to_vec(v).unwrap()).unwrap();
    validate_selection_binding(&selection(&run), &m, &t, &e, None).unwrap();
    for (p, v) in [
        ("/repository/id", json!(999)),
        ("/source/id", json!(999)),
        ("/source/attempt", json!(999)),
        ("/expected_sha", json!("0".repeat(40))),
        ("/manifest_digest", json!("0".repeat(64))),
    ] {
        let mut wrong = run.clone();
        *wrong.pointer_mut(p).unwrap() = v;
        assert!(
            validate_selection_binding(&selection(&wrong), &m, &t, &e, None).is_err(),
            "{p}"
        );
    }
    let channel = json!({"kind":"channel","repository":m.repository(),"channel":"preview","workflow":m.workflow()});
    validate_selection_binding(&selection(&channel), &m, &t, &e, None).unwrap();
    for (p, v) in [
        ("/repository/id", json!(999)),
        ("/channel", json!("nightly")),
        ("/workflow/id", json!(999)),
    ] {
        let mut wrong = channel.clone();
        *wrong.pointer_mut(p).unwrap() = v;
        assert!(validate_selection_binding(&selection(&wrong), &m, &t, &e, None).is_err());
    }
    let mut pin = run;
    pin["source"] = json!({"kind":"release","tag":"runtime/v8.7.0"});
    let resolved = ResolvedReleaseEvidence {
        repository_id: m.repository().id,
        tag: "runtime/v8.7.0".into(),
        source_sha: m.source_sha().into(),
        manifest_digest: m.digest(),
    };
    assert!(validate_selection_binding(&selection(&pin), &m, &t, &e, None).is_err());
    validate_selection_binding(&selection(&pin), &m, &t, &e, Some(&resolved)).unwrap();
    for n in 0..4 {
        let mut wrong = resolved.clone();
        match n {
            0 => wrong.repository_id += 1,
            1 => wrong.tag = "other".into(),
            2 => wrong.source_sha = "0".repeat(40),
            _ => wrong.manifest_digest = "0".repeat(64),
        };
        assert!(validate_selection_binding(&selection(&pin), &m, &t, &e, Some(&wrong)).is_err());
    }
    assert!(validate_selection_binding(&local(), &m, &t, &e, None).is_err());
    let mut policy = trusted(&m);
    policy.workflow.id += 1;
    assert!(
        validate_selection_binding(&selection(&pin), &m, &policy, &e, Some(&resolved)).is_err()
    );
    let mut wrong = e;
    wrong.conclusion = "failure".into();
    assert!(validate_selection_binding(&selection(&channel), &m, &t, &wrong, None).is_err());
}
#[test]
fn later_preview_qualification_preserves_manifest_identity() {
    let m = manifest(&value());
    let before = m.digest();
    let t = trusted(&m);
    let q = QualificationEvidence {
        repository: m.repository().clone(),
        workflow: m.workflow().clone(),
        run: Run {
            id: 999,
            attempt: 1,
        },
        source_sha: m.source_sha().into(),
        build_inputs_digest: m.build_inputs().digest(),
        status: "completed".into(),
        conclusion: "success".into(),
        tests: TestResult::Passed,
    };
    assert_eq!(
        validate_qualification(&m, Channel::Preview, &t, None).unwrap(),
        Qualification::Untested
    );
    assert_eq!(
        validate_qualification(&m, Channel::Preview, &t, Some(&q)).unwrap(),
        Qualification::Tested
    );
    assert_eq!(m.digest(), before);
    let mut same_run = q.clone();
    same_run.run = m.run().clone();
    assert_eq!(
        validate_qualification(&m, Channel::Preview, &t, Some(&same_run)).unwrap(),
        Qualification::Tested
    );
    for n in 0..9 {
        let mut wrong = q.clone();
        match n {
            0 => wrong.repository.id += 1,
            1 => wrong.workflow.id += 1,
            2 => wrong.source_sha = "0".repeat(40),
            3 => wrong.build_inputs_digest = "0".repeat(64),
            4 => wrong.status = "queued".into(),
            5 => wrong.conclusion = "failure".into(),
            6 => wrong.tests = TestResult::Failed,
            7 => wrong.run.id = 0,
            _ => wrong.run.attempt = 0,
        };
        assert!(validate_qualification(&m, Channel::Preview, &t, Some(&wrong)).is_err());
    }
    let mut v = value();
    v["tests"] = json!("failed");
    assert!(validate_qualification(&manifest(&v), Channel::Preview, &t, Some(&q)).is_err());
    v["tests"] = json!("passed");
    assert!(validate_qualification(&manifest(&v), Channel::Preview, &t, None).is_err());
    v["tests"] = json!("not_run");
    v["build"]["channel"] = json!("stable");
    assert!(validate_qualification(&manifest(&v), Channel::Stable, &t, Some(&q)).is_err());
}
fn scratch() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}
fn value() -> Value {
    serde_json::from_slice(FIXTURE).unwrap()
}
fn manifest(v: &Value) -> RuntimeManifest {
    RuntimeManifest::from_json(&serde_json::to_vec(v).unwrap()).unwrap()
}
fn local() -> RuntimeSelection {
    #[cfg(windows)]
    return RuntimeSelection::from_json(
        br#"{"kind":"local","absolute_path":"C:\\not-scanned\\refact"}"#,
    )
    .unwrap();
    #[cfg(not(windows))]
    RuntimeSelection::from_json(br#"{"kind":"local","absolute_path":"/not-scanned/refact"}"#)
        .unwrap()
}
fn trusted(m: &RuntimeManifest) -> TrustedSource {
    TrustedSource {
        repository: m.repository().clone(),
        workflow: m.workflow().clone(),
    }
}

#[test]
fn fixture_canonical_identity_and_protocols() {
    let m = RuntimeManifest::from_json(FIXTURE).unwrap();
    let mut v = value();
    v["artifacts"].as_array_mut().unwrap().reverse();
    assert_eq!(m.digest(), manifest(&v).digest());
    assert_eq!(
        m.digest(),
        "5ff571f38849b95bb0d1aa626ebe150a64c72a8ddcf750ea2f93e28bdd7bb9f2"
    );
    assert_eq!(m, RuntimeManifest::from_json(&m.canonical_bytes()).unwrap());
    assert_eq!(m.component_versions().jetbrains, "8.7.0.138-EAP");
    assert!(m.check_protocols(1, 2, 3).is_ok());
    assert!(m.check_protocols(1, 3, 3).is_err());
    assert_eq!(
        validate_qualification(&m, Channel::Preview, &trusted(&m), None).unwrap(),
        Qualification::Untested
    );
    assert!(ManagedIdentity::from_manifest(&m, "universal").is_err());
    println!("fixture canonical SHA256 {}", m.digest());
}
#[test]
fn rejects_malformed_unknown_oversized_paths_and_duplicates() {
    for (pointer, replacement) in [
        ("/schema_version", json!(2)),
        ("/unexpected", json!(true)),
        ("/repository/extra", json!(1)),
        ("/source_sha", json!("abc")),
        ("/build/version", json!("latest")),
        ("/artifacts/0/target", json!("universal")),
        ("/artifacts/0/size", json!(0)),
        ("/artifacts/0/sha256", json!("A".repeat(64))),
        ("/compatibility/daemon_api/min", json!(10)),
    ] {
        let mut v = value();
        if pointer == "/unexpected" {
            v["unexpected"] = replacement;
        } else if pointer == "/repository/extra" {
            v["repository"]["extra"] = replacement;
        } else {
            *v.pointer_mut(pointer).unwrap() = replacement;
        }
        assert!(
            RuntimeManifest::from_json(&serde_json::to_vec(&v).unwrap()).is_err(),
            "{pointer}"
        );
    }
    for p in [
        "../refact",
        "/refact",
        "a\\b",
        "C:refact",
        "a/./b",
        "a//b",
        "a\0b",
    ] {
        let mut v = value();
        v["artifacts"][0]["extracted_files"][0]["path"] = json!(p);
        assert!(
            RuntimeManifest::from_json(&serde_json::to_vec(&v).unwrap()).is_err(),
            "{p}"
        );
    }
    let mut v = value();
    let a = v["artifacts"][0].clone();
    v["artifacts"].as_array_mut().unwrap().push(a);
    assert!(RuntimeManifest::from_json(&serde_json::to_vec(&v).unwrap()).is_err());
    v["artifacts"][4]["filename"] = json!("other.tar.gz");
    assert!(RuntimeManifest::from_json(&serde_json::to_vec(&v).unwrap()).is_err());
    assert!(RuntimeManifest::from_json(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
    assert!(RuntimeManifest::from_json(b"{").is_err());
    let mut v = value();
    v["artifacts"] = json!(vec![v["artifacts"][0].clone(); MAX_ARTIFACTS + 1]);
    assert!(serde_json::from_value::<RuntimeManifest>(v).is_err());
    let mut v = value();
    v["qualification_ref"] = json!("x".repeat(513));
    assert!(serde_json::from_value::<RuntimeManifest>(v).is_err());
}
#[test]
fn evidence_is_separate_and_exact() {
    let m = manifest(&value());
    let t = trusted(&m);
    let evidence = SourceEvidence {
        repository: m.repository().clone(),
        workflow: m.workflow().clone(),
        run: m.run().clone(),
        source_sha: m.source_sha().into(),
        status: "completed".into(),
        conclusion: "success".into(),
    };
    validate_source_evidence(&m, &t, &evidence).unwrap();
    for n in 0..7 {
        let mut e = evidence.clone();
        match n {
            0 => e.repository.id += 1,
            1 => e.workflow.id += 1,
            2 => e.run.id += 1,
            3 => e.run.attempt += 1,
            4 => e.source_sha = "0".repeat(40),
            5 => e.status = "in_progress".into(),
            _ => e.conclusion = "failure".into(),
        };
        assert!(validate_source_evidence(&m, &t, &e).is_err());
    }
    let mut policy = trusted(&m);
    policy.repository.id += 1;
    assert!(validate_source_evidence(&m, &policy, &evidence).is_err());
    assert_eq!(
        validate_qualification(&m, Channel::Preview, &t, None).unwrap(),
        Qualification::Untested
    );
    let mut v = value();
    v["build"]["channel"] = json!("stable");
    assert!(validate_qualification(&manifest(&v), Channel::Stable, &t, None).is_err());
    v["tests"] = json!("passed");
    let stable = manifest(&v);
    assert!(validate_qualification(&stable, Channel::Stable, &t, None).is_err());
    let mut q = QualificationEvidence {
        repository: stable.repository().clone(),
        workflow: stable.workflow().clone(),
        run: Run {
            id: 900,
            attempt: 1,
        },
        source_sha: stable.source_sha().into(),
        build_inputs_digest: stable.build_inputs().digest(),
        status: "completed".into(),
        conclusion: "success".into(),
        tests: TestResult::Passed,
    };
    validate_qualification(&stable, Channel::Stable, &t, Some(&q)).unwrap();
    q.build_inputs_digest = "0".repeat(64);
    assert!(validate_qualification(&stable, Channel::Stable, &t, Some(&q)).is_err());
    v["build"]["channel"] = json!("preview");
    v["tests"] = json!("failed");
    assert!(validate_qualification(&manifest(&v), Channel::Preview, &t, None).is_err());
}
#[test]
fn strict_selections_and_independent_unknown_observations() {
    assert!(local().require_managed_overwrite().is_err());
    for s in [
        json!({"kind":"local","absolute_path":"relative"}),
        json!({"kind":"local","absolute_path":"/a/../b"}),
        json!({"kind":"local","absolute_path":"/a","overwrite":true}),
        json!({"kind":"channel","repository":{"owner":"x","name":"y","id":1},"channel":"latest"}),
        json!({"kind":"pinned","repository":{"owner":"x","name":"y","id":1},"source":{"kind":"run","id":1,"attempt":0}}),
    ] {
        assert!(RuntimeSelection::from_json(&serde_json::to_vec(&s).unwrap()).is_err());
    }
    let identity =
        ManagedIdentity::from_manifest(&manifest(&value()), "x86_64-unknown-linux-gnu").unwrap();
    assert!(
        InstalledRuntime::new(&local(), "/not-scanned/refact".into(), identity.clone()).is_err()
    );
    let status = RuntimeStatus {
        selected: Observation::Known(local()),
        staged: Observation::Known(identity),
        ..Default::default()
    };
    assert_eq!(status.installed, Observation::Unknown);
    assert_eq!(status.running, Observation::Unknown);
    assert_eq!(
        serde_json::from_slice::<RuntimeStatus>(&serde_json::to_vec(&status).unwrap()).unwrap(),
        status
    );
}
#[test]
fn transactional_cas_two_writers_and_corruption_untouched() {
    let dir = scratch();
    let path = dir.path().canonicalize().unwrap().join("selection.json");
    let store = SelectionStore::new(path.clone()).unwrap();
    assert_eq!(store.load().unwrap(), None);
    assert!(!path.exists());
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let p = path.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                SelectionStore::new(p).unwrap().compare_and_swap(0, local())
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(store.load().unwrap().unwrap().revision(), 1);
    assert_eq!(store.compare_and_swap(1, local()).unwrap().revision(), 2);
    let lock = path.with_file_name("selection.json.lock");
    assert!(lock.exists());
    for bytes in [b"{broken".to_vec(),serde_json::to_vec(&json!({"schema_version":2,"revision":3,"selection":{"kind":"local","absolute_path":"/a"}})).unwrap()] {
        std::fs::write(&path,&bytes).unwrap(); assert!(store.load().is_err()); assert!(store.compare_and_swap(2,local()).is_err()); assert_eq!(std::fs::read(&path).unwrap(),bytes);
    }
}
#[test]
fn cross_process_writer() {
    let Some(path) = std::env::var_os("REFACT_CONTRACT_TEST_STORE") else {
        return;
    };
    let store = SelectionStore::new(path.into()).unwrap();
    let result = store.compare_and_swap(0, local());
    if result.is_err() {
        assert_eq!(store.load().unwrap().unwrap().revision(), 1);
    }
    std::process::exit(if result.is_ok() { 0 } else { 10 });
}
#[test]
fn cross_process_cas_one_winner() {
    let dir = scratch();
    let path = dir.path().canonicalize().unwrap().join("selection.json");
    let mut children: Vec<_> = (0..2)
        .map(|_| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "cross_process_writer"])
                .env("REFACT_CONTRACT_TEST_STORE", &path)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut codes: Vec<_> = children
        .iter_mut()
        .map(|p| p.wait().unwrap().code().unwrap())
        .collect();
    codes.sort();
    assert_eq!(codes, vec![0, 10]);
}
#[test]
fn javascript_lock_inventory_is_canonical_and_bound() {
    use std::collections::BTreeMap;
    let mut locks = BTreeMap::new();
    locks.insert("refact-agent/gui/package-lock.json".into(), "1".repeat(64));
    locks.insert("plugins/vscode/package-lock.json".into(), "2".repeat(64));
    let first = BuildInputs::javascript_locks_digest(&locks).unwrap();
    let reversed = locks
        .iter()
        .rev()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    assert_eq!(
        first,
        BuildInputs::javascript_locks_digest(&reversed).unwrap()
    );
    locks.insert("plugins/vscode/package-lock.json".into(), "3".repeat(64));
    assert_ne!(first, BuildInputs::javascript_locks_digest(&locks).unwrap());
    locks.insert("../package-lock.json".into(), "4".repeat(64));
    assert!(BuildInputs::javascript_locks_digest(&locks).is_err());
    assert!(BuildInputs::javascript_locks_digest(&BTreeMap::new()).is_err());
}
#[cfg(unix)]
#[test]
fn store_rejects_symlinks_and_readonly_without_overwrite() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = scratch();
    let p = dir.path().canonicalize().unwrap().join("selection");
    let other = dir.path().join("other");
    std::fs::write(&other, b"untouched").unwrap();
    symlink(&other, &p).unwrap();
    let store = SelectionStore::new(p.clone()).unwrap();
    assert!(store.load().is_err());
    assert!(store.compare_and_swap(0, local()).is_err());
    assert_eq!(std::fs::read(&other).unwrap(), b"untouched");
    std::fs::remove_file(&p).unwrap();
    let lockpath = p.with_file_name("selection.lock");
    std::fs::remove_file(&lockpath).unwrap();
    symlink(&other, &lockpath).unwrap();
    assert!(store.compare_and_swap(0, local()).is_err());
    assert_eq!(std::fs::read(&other).unwrap(), b"untouched");
    std::fs::remove_file(&lockpath).unwrap();
    store.compare_and_swap(0, local()).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o400)).unwrap();
    let before = std::fs::read(&p).unwrap();
    assert!(store.compare_and_swap(1, local()).is_err());
    assert_eq!(std::fs::read(&p).unwrap(), before);
}
