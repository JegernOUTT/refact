//! Hermetic adversarial tests for the pure Stable public qualification validator.
//! No network, filesystem or ambient repository state: all inputs are in-memory bytes.
use refact_self_update::public_qualification::*;
use refact_self_update::runtime_manifest::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const FIXTURE: &[u8] = include_bytes!("fixtures/runtime-manifest-v1.json");
/// The real repository inventory, compiled in; only IDs/flags are used, no jobs are run.
const REAL_INVENTORY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../tools/dev/ci_test_inventory.json"
));
const ARTIFACT_ID: u64 = 4242;
const UNAVAILABLE: &str = "Opt-in real-model/provider certification; not executed.";

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
/// Shaped like tools/dev/ci_test_inventory.json (extra per-entry fields included).
fn inventory() -> Value {
    json!({
        "schema": 1,
        "required_jobs": ["engine", "gui"],
        "entries": [
            {"id": "gui-types", "lane": "gui", "cwd": "refact-agent/gui", "command": ["npm", "run", "types"],
             "required": true, "platform": "linux-x64", "prerequisites": [], "features": [], "secrets": [], "kind": "check"},
            {"id": "engine-reusable", "lane": "engine", "cwd": "refact-agent/engine", "command": ["cargo", "test"],
             "required": true, "platform": "linux-x64", "prerequisites": [], "features": [], "secrets": [],
             "kind": "reusable", "source": ".github/workflows/agent_engine_check.yml", "commands": [["cargo", "test"]]},
            {"id": "python-test13_vision.py", "lane": "external", "cwd": ".", "command": ["python3", "x.py"],
             "required": false, "platform": "conditional", "prerequisites": [], "features": [], "secrets": [],
             "kind": "excluded", "unavailable": UNAVAILABLE}
        ]
    })
}
fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(v).unwrap()
}
fn stable_value() -> Value {
    let mut v: Value = serde_json::from_slice(FIXTURE).unwrap();
    v["build"]["channel"] = json!("stable");
    v["tests"] = json!("passed");
    v
}
fn build_inputs(v: &Value) -> BuildInputs {
    serde_json::from_value(v["build_inputs"].clone()).unwrap()
}
/// Mirrors the key set written by tools/dev/ci_tests.py main() for the aggregate report.
fn report(manifest: &Value, inventory_bytes: &[u8]) -> Value {
    let bi = build_inputs(manifest);
    let results = json!([
        {"id": "engine", "status": "PASS"},
        {"id": "gui", "status": "PASS"},
        {"id": "gui-types", "status": "PASS", "returncode": 0, "output_sha256": "a".repeat(64)},
        {"id": "engine-reusable", "status": "PASS", "evidence_source": "job", "job": "engine"},
        {"id": "python-test13_vision.py", "status": "EXCLUDED", "reason": UNAVAILABLE}
    ]);
    json!({
        "build_inputs": manifest["build_inputs"],
        "build_inputs_digest": bi.digest(),
        "repository_id": manifest["repository"]["id"].to_string(),
        "run_id": manifest["run"]["id"].to_string(),
        "run_attempt": manifest["run"]["attempt"].to_string(),
        "schema": 1,
        "schema_version": 1,
        "kind": "refact-tests-qualification",
        "source_sha": manifest["source_sha"],
        "inputs": {"rust": bi.rust_toolchain, "node": bi.node_version, "java": "17",
                   "build_profile": bi.profile, "test_profile": "ci-test"},
        "inventory_sha256": sha(inventory_bytes),
        "results": results,
        "scope": "selected supported lanes; exclusions are not passes",
        "status": "PASS",
        "excluded_coverage": [results[4].clone()],
        "coverage_complete": true,
        "tests": "passed",
        "test_profile": "ci-test"
    })
}
fn qref(m: &Value, report_bytes: &[u8]) -> String {
    format!(
        "github-artifact:{}/{}/{}/{}/{}",
        m["repository"]["id"],
        m["run"]["id"],
        m["run"]["attempt"],
        ARTIFACT_ID,
        sha(report_bytes)
    )
}
fn manifest(v: &Value) -> RuntimeManifest {
    RuntimeManifest::from_json(&serde_json::to_vec(v).unwrap()).unwrap()
}
fn trusted(m: &RuntimeManifest) -> TrustedSource {
    TrustedSource {
        repository: m.repository().clone(),
        workflow: m.workflow().clone(),
    }
}
fn source(m: &RuntimeManifest) -> SourceEvidence {
    SourceEvidence {
        repository: m.repository().clone(),
        workflow: m.workflow().clone(),
        run: m.run().clone(),
        source_sha: m.source_sha().into(),
        status: "completed".into(),
        conclusion: "success".into(),
    }
}
fn policy() -> TrustedQualificationPolicy {
    TrustedQualificationPolicy::from_inventory_bytes(&bytes(&inventory())).unwrap()
}

/// Binds `report` bytes into a stable manifest (reference digest) and verifies everything.
fn verify_with(
    mv: &Value,
    report: &Value,
    inv: &[u8],
    policy: &TrustedQualificationPolicy,
) -> ContractResult<VerifiedPublicQualification> {
    let rb = bytes(report);
    let mut mv = mv.clone();
    mv["qualification_ref"] = json!(qref(&mv, &rb));
    let m = manifest(&mv);
    verify_public_qualification(&m, &trusted(&m), &source(&m), policy, &rb, inv)
}
fn ok_case() -> (Value, Value, Vec<u8>) {
    let mv = stable_value();
    let inv = bytes(&inventory());
    let r = report(&mv, &inv);
    (mv, r, inv)
}

#[test]
fn producer_shaped_report_qualifies_same_outer_run() {
    let (mv, r, inv) = ok_case();
    let verified = verify_with(&mv, &r, &inv, &policy()).unwrap();
    let m = manifest(&stable_value());
    let e = verified.evidence();
    assert_eq!(e.tests, TestResult::Passed);
    // Same outer publish run as the build is accepted.
    assert_eq!(e.run, *m.run());
    assert_eq!(e.build_inputs_digest, m.build_inputs().digest());
    assert_eq!(verified.reference().artifact_id, ARTIFACT_ID);
    assert_eq!(verified.report_sha256(), sha(&bytes(&r)));
    assert_eq!(verified.inventory_sha256(), sha(&inv));
    assert_eq!(policy().required_ids().len(), 4);
}

#[test]
fn report_bytes_are_pinned_by_manifest_reference() {
    let (mut mv, r, inv) = ok_case();
    let rb = bytes(&r);
    mv["qualification_ref"] = json!(qref(&mv, &rb));
    let m = manifest(&mv);
    let p = policy();
    verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &rb, &inv).unwrap();
    // Semantically equal but different bytes fail: the digest pins exact bytes.
    let compact = serde_json::to_vec(&r).unwrap();
    assert!(
        verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &compact, &inv).is_err()
    );
    let mut trailing = rb.clone();
    trailing.push(b'\n');
    assert!(
        verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &trailing, &inv).is_err()
    );
    // A forged PASS report rebound under the same reference fails.
    let mut forged = r.clone();
    forged["results"][2]["output_sha256"] = json!("b".repeat(64));
    let fb = bytes(&forged);
    assert!(verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &fb, &inv).is_err());
    assert!(verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &[], &inv).is_err());
    let huge = vec![b' '; MAX_QUALIFICATION_BYTES + 1];
    assert!(verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &huge, &inv).is_err());
}

#[test]
fn reference_identity_and_syntax_are_strict() {
    let (mv, r, inv) = ok_case();
    let rb = bytes(&r);
    let good = qref(&mv, &rb);
    let d = sha(&rb);
    let p = policy();
    let bad = [
        format!("github-artifact:999/789/2/{ARTIFACT_ID}/{d}"),
        format!("github-artifact:123/790/2/{ARTIFACT_ID}/{d}"),
        format!("github-artifact:123/789/3/{ARTIFACT_ID}/{d}"),
        format!("github-artifact:123/789/2/0/{d}"),
        format!("github-artifact:0123/789/2/{ARTIFACT_ID}/{d}"),
        format!(
            "github-artifact:123/789/2/{ARTIFACT_ID}/{}",
            d.to_uppercase()
        ),
        format!("github-artifact:123/789/2/{ARTIFACT_ID}/{d}/x"),
        format!("github-artifact:123/789/2/99999999999999999999999/{d}"),
        format!("github-artifact:123/789/+2/{ARTIFACT_ID}/{d}"),
        format!("artifact:123/789/2/{ARTIFACT_ID}/{d}"),
    ];
    for q in bad {
        let mut v = mv.clone();
        v["qualification_ref"] = json!(q);
        let m = manifest(&v);
        assert!(
            verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &rb, &inv).is_err(),
            "{q}"
        );
    }
    let mut v = mv.clone();
    v["qualification_ref"] = json!(good);
    let m = manifest(&v);
    verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &rb, &inv).unwrap();
    // No reference at all: manifest tests=passed alone never qualifies.
    let m = manifest(&mv);
    assert!(verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &rb, &inv).is_err());
}

#[test]
fn manifest_channel_and_source_evidence_gate() {
    let (mv, r, inv) = ok_case();
    let p = policy();
    for (field, value) in [("channel", "preview"), ("tests", "not_run")] {
        let mut v = mv.clone();
        if field == "channel" {
            v["build"]["channel"] = json!(value);
        } else {
            v["tests"] = json!(value);
        }
        let rb = bytes(&r);
        v["qualification_ref"] = json!(qref(&v, &rb));
        let m = manifest(&v);
        assert!(verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &rb, &inv).is_err());
    }
    let rb = bytes(&r);
    let mut v = mv.clone();
    v["qualification_ref"] = json!(qref(&v, &rb));
    let m = manifest(&v);
    for n in 0..4 {
        let mut s = source(&m);
        let mut t = trusted(&m);
        match n {
            0 => s.conclusion = "failure".into(),
            1 => s.status = "in_progress".into(),
            2 => s.source_sha = "0".repeat(40),
            _ => t.workflow.id += 1,
        }
        assert!(
            verify_public_qualification(&m, &t, &s, &p, &rb, &inv).is_err(),
            "{n}"
        );
    }
}

#[test]
fn forged_report_semantics_fail_even_when_correctly_rebound() {
    let (mv, r, inv) = ok_case();
    let p = policy();
    let mutations: Vec<(&str, Value)> = vec![
        ("/status", json!("BLOCKED")),
        ("/tests", json!("blocked")),
        ("/coverage_complete", json!(false)),
        ("/coverage_complete", json!("true")),
        ("/schema", json!(2)),
        ("/schema_version", json!(2)),
        ("/kind", json!("other")),
        ("/repository_id", json!("999")),
        ("/repository_id", json!(123)),
        ("/run_id", json!("790")),
        ("/run_attempt", json!("1")),
        ("/run_attempt", json!("02")),
        ("/source_sha", json!("0".repeat(40))),
        ("/build_inputs/profile", json!("dev")),
        ("/build_inputs_digest", json!("0".repeat(64))),
        ("/inputs/rust", json!("1.0.0")),
        ("/inputs/build_profile", json!("dev")),
        ("/inputs/test_profile", json!("release")),
        ("/test_profile", json!("release")),
        ("/inventory_sha256", json!("0".repeat(64))),
        ("/results/0/status", json!("FAIL")),
        ("/results/2/status", json!("BLOCKED")),
        ("/results/2/status", json!("EXCLUDED")),
        ("/results/3/status", json!("pass")),
        ("/results/4/reason", json!("other reason")),
        ("/results/4/status", json!("SKIPPED")),
        ("/results/2/output_sha256", json!("Z".repeat(64))),
        ("/results/1/id", json!("gui-types")),
        ("/results/1/id", json!("unknown-job")),
        ("/excluded_coverage", json!([])),
        ("/scope", json!("")),
    ];
    for (pointer, value) in mutations {
        let mut forged = r.clone();
        *forged.pointer_mut(pointer).unwrap() = value.clone();
        assert!(
            verify_with(&mv, &forged, &inv, &p).is_err(),
            "{pointer}={value}"
        );
    }
    // Missing or extra results, unknown fields (top level, nested), missing field.
    let mut forged = r.clone();
    forged["results"].as_array_mut().unwrap().pop();
    assert!(verify_with(&mv, &forged, &inv, &p).is_err());
    let mut forged = r.clone();
    let dup = forged["results"][0].clone();
    forged["results"].as_array_mut().unwrap().push(dup);
    assert!(verify_with(&mv, &forged, &inv, &p).is_err());
    for target in ["", "/inputs", "/results/0", "/build_inputs"] {
        let mut forged = r.clone();
        forged.pointer_mut(target).unwrap()["required_ids"] = json!(["engine"]);
        assert!(
            verify_with(&mv, &forged, &inv, &p).is_err(),
            "unknown at {target}"
        );
    }
    let mut forged = r.clone();
    forged.as_object_mut().unwrap().remove("scope");
    assert!(verify_with(&mv, &forged, &inv, &p).is_err());
}

#[test]
fn duplicate_keys_are_rejected_before_typed_decode() {
    let (mv, r, inv) = ok_case();
    let p = policy();
    // Text-level duplicate: a later "status" key would win in serde_json and Python.
    let text = String::from_utf8(bytes(&r)).unwrap();
    for (from, to) in [
        (
            "\"status\": \"PASS\",\n  \"excluded_coverage\"",
            "\"status\": \"BLOCKED\",\n  \"status\": \"PASS\",\n  \"excluded_coverage\"",
        ),
        (
            "\"id\": \"engine\",",
            "\"id\": \"engine\",\n      \"id\": \"engine\",",
        ),
    ] {
        assert!(text.contains(from), "fixture shape changed: {from}");
        let dup = text.replacen(from, to, 1).into_bytes();
        let mut v = mv.clone();
        v["qualification_ref"] = json!(qref(&v, &dup));
        let m = manifest(&v);
        assert!(
            verify_public_qualification(&m, &trusted(&m), &source(&m), &p, &dup, &inv).is_err()
        );
    }
    let inv_text = String::from_utf8(inv.clone()).unwrap();
    let dup_inv = inv_text.replacen("\"schema\": 1,", "\"schema\": 1,\n  \"schema\": 1,", 1);
    assert_ne!(dup_inv, inv_text);
    assert!(TrustedQualificationPolicy::from_inventory_bytes(dup_inv.as_bytes()).is_err());
    let mut forged = report(&mv, dup_inv.as_bytes());
    forged["inventory_sha256"] = json!(sha(dup_inv.as_bytes()));
    assert!(verify_with(&mv, &forged, dup_inv.as_bytes(), &p).is_err());
}

#[test]
fn source_inventory_is_hash_bound_and_cannot_drop_trusted_coverage() {
    let (mv, r, inv) = ok_case();
    let p = policy();
    // Different inventory bytes than the report's hash.
    let mut other = inventory();
    other["entries"][2]["unavailable"] = json!("changed");
    assert!(verify_with(&mv, &r, &bytes(&other), &p).is_err());
    // A self-consistent report+inventory that demotes a trusted-required entry fails.
    let mut weak = inventory();
    weak["entries"][0]["required"] = json!(false);
    weak["entries"][0]["unavailable"] = json!(UNAVAILABLE);
    let wb = bytes(&weak);
    let mut forged = report(&mv, &wb);
    forged["results"][2] = json!({"id": "gui-types", "status": "EXCLUDED", "reason": UNAVAILABLE});
    let excluded = json!([forged["results"][2], forged["results"][4]]);
    forged["excluded_coverage"] = excluded;
    assert!(verify_with(&mv, &forged, &wb, &p).is_err());
    // The same weakened inventory would pass only under a weaker caller policy.
    let lax = TrustedQualificationPolicy::from_inventory_bytes(&wb).unwrap();
    verify_with(&mv, &forged, &wb, &lax).unwrap();
    // Dropping a required job from the source inventory also fails.
    let mut dropped = inventory();
    dropped["required_jobs"] = json!(["engine"]);
    let db = bytes(&dropped);
    let mut forged = report(&mv, &db);
    forged["results"].as_array_mut().unwrap().remove(1);
    let excluded = json!([forged["results"][3]]);
    forged["excluded_coverage"] = excluded;
    assert!(verify_with(&mv, &forged, &db, &p).is_err());
    // Adding extra required coverage in source is fine as long as it passed.
    let mut extra = inventory();
    extra["required_jobs"] = json!(["engine", "gui", "tooling"]);
    let eb = bytes(&extra);
    let mut forged = report(&mv, &eb);
    forged["results"]
        .as_array_mut()
        .unwrap()
        .insert(2, json!({"id": "tooling", "status": "PASS"}));
    verify_with(&mv, &forged, &eb, &p).unwrap();
    let _ = r;
}

#[test]
fn inventory_shape_is_validated() {
    let cases: Vec<Value> = vec![
        json!({"schema": 2, "required_jobs": ["a"], "entries": []}),
        json!({"schema": 1, "required_jobs": [], "entries": []}),
        json!({"schema": 1, "required_jobs": ["a", "a"], "entries": []}),
        json!({"schema": 1, "required_jobs": ["a"], "entries": [{"id": "a", "required": true}]}),
        json!({"schema": 1, "required_jobs": ["a"], "entries": [{"id": "b", "required": "true"}]}),
        json!({"schema": 1, "required_jobs": ["a"], "entries": [{"id": "b"}]}),
        json!({"schema": 1, "required_jobs": [""], "entries": []}),
        json!({"schema": 1, "required_jobs": ["a\nb"], "entries": []}),
        json!({"schema": 1, "required_jobs": ["a"]}),
        json!([]),
    ];
    for c in cases {
        assert!(
            TrustedQualificationPolicy::from_inventory_bytes(&bytes(&c)).is_err(),
            "{c}"
        );
    }
    let ids: Vec<String> = (0..=MAX_INVENTORY_IDS).map(|i| format!("j{i}")).collect();
    let big = json!({"schema": 1, "required_jobs": ids, "entries": []});
    assert!(TrustedQualificationPolicy::from_inventory_bytes(&bytes(&big)).is_err());
    assert!(
        TrustedQualificationPolicy::from_inventory_bytes(&vec![b' '; MAX_INVENTORY_BYTES + 1])
            .is_err()
    );
    assert!(TrustedQualificationPolicy::from_inventory_bytes(b"").is_err());
}

#[test]
fn qualification_ref_parser_roundtrip() {
    let d = "a".repeat(64);
    let r = QualificationRef::parse(&format!("github-artifact:1/2/3/4/{d}")).unwrap();
    assert_eq!(
        (r.repository_id, r.run_id, r.run_attempt, r.artifact_id),
        (1, 2, 3, 4)
    );
    assert_eq!(r.sha256, d);
    for bad in [
        "",
        "github-artifact:",
        "github-artifact:1/2/3/4",
        "github-artifact:1/2/3/4/abc",
    ] {
        assert!(QualificationRef::parse(bad).is_err(), "{bad}");
    }
    assert!(QualificationRef::parse(&format!("github-artifact:{}", "1/".repeat(100_000))).is_err());
}

#[test]
fn oversized_source_inventory_is_rejected_before_digest_comparison() {
    let (mv, r, _) = ok_case();
    let error = verify_with(&mv, &r, &vec![b' '; MAX_INVENTORY_BYTES + 1], &policy()).unwrap_err();
    assert_eq!(error, "inventory byte limit exceeded");
}

/// FIXTURE report over the real inventory IDs (statuses synthesized, not produced by CI):
/// mandatory jobs/entries PASS, optional entries EXCLUDED with their exact unavailability.
fn real_report(mv: &Value) -> Value {
    let inv: Value = serde_json::from_slice(REAL_INVENTORY).unwrap();
    let mut results: Vec<Value> = inv["required_jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| json!({"id": j, "status": "PASS"}))
        .collect();
    for e in inv["entries"].as_array().unwrap() {
        results.push(
            match (e["required"].as_bool().unwrap(), e.get("unavailable")) {
                (false, Some(u)) => json!({"id": e["id"], "status": "EXCLUDED", "reason": u}),
                _ => json!({"id": e["id"], "status": "PASS"}),
            },
        );
    }
    let excluded: Vec<Value> = results
        .iter()
        .filter(|r| r["status"] == "EXCLUDED")
        .cloned()
        .collect();
    let mut r = report(mv, REAL_INVENTORY);
    r["results"] = json!(results);
    r["excluded_coverage"] = json!(excluded);
    r
}

#[test]
fn real_inventory_policy_is_exact_and_fixture_report_qualifies() {
    let inv: Value = serde_json::from_slice(REAL_INVENTORY).unwrap();
    let mut expected: std::collections::BTreeSet<String> = inv["required_jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j.as_str().unwrap().to_string())
        .collect();
    for e in inv["entries"].as_array().unwrap() {
        if e["required"] == json!(true) {
            expected.insert(e["id"].as_str().unwrap().to_string());
        }
    }
    let p = TrustedQualificationPolicy::from_inventory_bytes(REAL_INVENTORY).unwrap();
    assert_eq!(p.required_ids(), &expected);
    for id in [
        "engine",
        "build-freshness",
        "engine-reusable",
        "gui-playwright",
    ] {
        assert!(p.required_ids().contains(id), "{id}");
    }
    assert!(!p.required_ids().contains("buddy-live"));
    let mv = stable_value();
    let r = real_report(&mv);
    assert!(r["excluded_coverage"].as_array().unwrap().len() > 1);
    let verified = verify_with(&mv, &r, REAL_INVENTORY, &p).unwrap();
    assert_eq!(verified.inventory_sha256(), sha(REAL_INVENTORY));
    // Reordered or altered excluded coverage vectors are rejected.
    let mut forged = r.clone();
    forged["excluded_coverage"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let error = verify_with(&mv, &forged, REAL_INVENTORY, &p).unwrap_err();
    assert_eq!(error, "excluded coverage mismatch");
    let mut forged = r.clone();
    forged["excluded_coverage"][0]["reason"] = json!("different");
    let error = verify_with(&mv, &forged, REAL_INVENTORY, &p).unwrap_err();
    assert_eq!(error, "excluded coverage mismatch");
    let mut forged = r;
    forged["excluded_coverage"].as_array_mut().unwrap().pop();
    let error = verify_with(&mv, &forged, REAL_INVENTORY, &p).unwrap_err();
    assert_eq!(error, "excluded coverage mismatch");
}

#[test]
fn exclusion_semantics_fail_with_exact_reasons() {
    let (mv, r, inv) = ok_case();
    let p = policy();
    let excluded = json!({"status": "EXCLUDED", "reason": UNAVAILABLE});
    // Required job EXCLUDED.
    let mut forged = r.clone();
    forged["results"][0]["status"] = excluded["status"].clone();
    forged["results"][0]["reason"] = excluded["reason"].clone();
    let ex = json!([forged["results"][0], forged["results"][4]]);
    forged["excluded_coverage"] = ex;
    let error = verify_with(&mv, &forged, &inv, &p).unwrap_err();
    assert_eq!(error, "required job did not pass");
    // Required entry EXCLUDED.
    let mut forged = r.clone();
    forged["results"][2] = json!({"id": "gui-types", "status": "EXCLUDED", "reason": UNAVAILABLE});
    let ex = json!([forged["results"][2], forged["results"][4]]);
    forged["excluded_coverage"] = ex;
    let error = verify_with(&mv, &forged, &inv, &p).unwrap_err();
    assert_eq!(error, "inventory entry did not qualify");
    // Optional entry EXCLUDED although the inventory declares no unavailability.
    let mut no_reason = inventory();
    no_reason["entries"][2]
        .as_object_mut()
        .unwrap()
        .remove("unavailable");
    let nb = bytes(&no_reason);
    let mut forged = report(&mv, &nb);
    forged["results"][4] = json!({"id": "python-test13_vision.py", "status": "EXCLUDED"});
    let ex = json!([forged["results"][4]]);
    forged["excluded_coverage"] = ex;
    let error = verify_with(&mv, &forged, &nb, &p).unwrap_err();
    assert_eq!(error, "inventory entry did not qualify");
    // Empty unavailability is invalid inventory, both as policy and as source inventory.
    let mut empty = inventory();
    empty["entries"][2]["unavailable"] = json!("");
    let eb = bytes(&empty);
    let error = TrustedQualificationPolicy::from_inventory_bytes(&eb).unwrap_err();
    assert_eq!(error, "invalid test inventory unavailability");
    let mut forged = report(&mv, &eb);
    forged["results"][4]["reason"] = json!("");
    let ex = json!([forged["results"][4]]);
    forged["excluded_coverage"] = ex;
    let error = verify_with(&mv, &forged, &eb, &p).unwrap_err();
    assert_eq!(error, "invalid test inventory unavailability");
}
