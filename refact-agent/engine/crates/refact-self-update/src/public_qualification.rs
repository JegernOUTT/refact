//! Pure Stable qualification validator for public runtime publications.
//!
//! Everything here is a bounded, read-only decode of bytes the caller already holds:
//! no network, filesystem, staging or activation. Validation mirrors
//! `tools/dev/runtime_manifest.py::verify_qualification` and the report written by
//! `tools/dev/ci_tests.py` (schema 1, `tests-qualification.json`).
//!
//! Trust boundaries (documented, not authenticated here):
//! * `TrustedSource` and `SourceEvidence` are plain public structs. The caller's resolver
//!   must have obtained `SourceEvidence` from an independently validated GitHub API response;
//!   this module only checks agreement, it cannot authenticate it.
//! * The report is pinned by the digest inside the manifest `qualification_ref`
//!   (`github-artifact:<repo>/<run>/<attempt>/<artifact>/<sha256>`); the reference itself is a
//!   digest anchor bound to the manifest identity, not network provenance.
//! * Required coverage comes from `TrustedQualificationPolicy`, constructed by the caller from
//!   trusted (e.g. compiled-in) inventory bytes. The report cannot choose its required IDs.
use crate::runtime_manifest::{
    decode, digest, require, text, validate_qualification, validate_source_evidence, BuildInputs,
    Channel, ContractResult, QualificationEvidence, RuntimeManifest, SourceEvidence, TestResult,
    TrustedSource,
};
use serde::de::{Error as _, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};

pub const QUALIFICATION_KIND: &str = "refact-tests-qualification";
pub const QUALIFICATION_NAME: &str = "tests-qualification.json";
pub const MAX_QUALIFICATION_BYTES: usize = 1024 * 1024;
pub const MAX_INVENTORY_BYTES: usize = 1024 * 1024;
pub const MAX_INVENTORY_IDS: usize = 4096;
const MAX_ID: usize = 256;
const MAX_REASON: usize = 4096;

/// Rejects duplicate object keys anywhere in a JSON document (serde_json keeps the last one,
/// Python keeps the last one; either way the ambiguity is refused before typed decoding).
struct NoDuplicateKeys;
impl<'de> Deserialize<'de> for NoDuplicateKeys {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(NoDuplicateKeys)
    }
}
impl<'de> Visitor<'de> for NoDuplicateKeys {
    type Value = NoDuplicateKeys;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Self, E> {
        Ok(self)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self, A::Error> {
        while seq.next_element::<NoDuplicateKeys>()?.is_some() {}
        Ok(self)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self, A::Error> {
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(A::Error::custom("duplicate JSON key"));
            }
            map.next_value::<NoDuplicateKeys>()?;
        }
        Ok(self)
    }
}

fn strict_decode<T: serde::de::DeserializeOwned>(bytes: &[u8], limit: usize) -> ContractResult<T> {
    require(!bytes.is_empty(), "empty JSON document")?;
    decode::<NoDuplicateKeys>(bytes, limit)?;
    decode(bytes, limit)
}
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn decimal_u64(s: &str) -> ContractResult<u64> {
    let digits = s.bytes().all(|b| b.is_ascii_digit());
    require(
        !s.is_empty() && s.len() <= 20 && digits && !s.starts_with('0'),
        "invalid decimal identifier",
    )?;
    s.parse().map_err(|_| "invalid decimal identifier".into())
}

/// Parsed `github-artifact:<repository_id>/<run_id>/<run_attempt>/<artifact_id>/<sha256>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualificationRef {
    pub repository_id: u64,
    pub run_id: u64,
    pub run_attempt: u64,
    pub artifact_id: u64,
    pub sha256: String,
}
impl QualificationRef {
    pub fn parse(value: &str) -> ContractResult<Self> {
        require(text(value, 512), "invalid qualification artifact reference")?;
        let rest = value
            .strip_prefix("github-artifact:")
            .ok_or("invalid qualification artifact reference")?;
        let parts: Vec<&str> = rest.split('/').collect();
        require(
            parts.len() == 5 && digest(parts[4], 64),
            "invalid qualification artifact reference",
        )?;
        Ok(Self {
            repository_id: decimal_u64(parts[0])?,
            run_id: decimal_u64(parts[1])?,
            run_attempt: decimal_u64(parts[2])?,
            artifact_id: decimal_u64(parts[3])?,
            sha256: parts[4].into(),
        })
    }
}

#[derive(Deserialize)]
struct InventoryEntryWire {
    id: String,
    required: bool,
    #[serde(default)]
    unavailable: Option<String>,
}
#[derive(Deserialize)]
struct InventoryWire {
    schema: u32,
    required_jobs: Vec<String>,
    entries: Vec<InventoryEntryWire>,
}
/// Validated inventory: unique, disjoint job/entry IDs; required flags are typed booleans.
struct Inventory {
    jobs: Vec<String>,
    entries: Vec<InventoryEntryWire>,
}
impl Inventory {
    fn parse(bytes: &[u8]) -> ContractResult<Self> {
        let w: InventoryWire = strict_decode(bytes, MAX_INVENTORY_BYTES)?;
        require(w.schema == 1, "unsupported test inventory schema")?;
        require(
            !w.required_jobs.is_empty()
                && w.required_jobs.len() + w.entries.len() <= MAX_INVENTORY_IDS,
            "test inventory size rejected",
        )?;
        let mut ids = HashSet::new();
        for id in w
            .required_jobs
            .iter()
            .chain(w.entries.iter().map(|e| &e.id))
        {
            require(text(id, MAX_ID), "invalid test inventory id")?;
            require(ids.insert(id.as_str()), "duplicate test inventory ids")?;
        }
        for e in &w.entries {
            require(
                e.unavailable.as_ref().is_none_or(|u| text(u, MAX_REASON)),
                "invalid test inventory unavailability",
            )?;
        }
        Ok(Self {
            jobs: w.required_jobs,
            entries: w.entries,
        })
    }
    fn required_ids(&self) -> BTreeSet<String> {
        let entries = self
            .entries
            .iter()
            .filter(|e| e.required)
            .map(|e| e.id.clone());
        self.jobs.iter().cloned().chain(entries).collect()
    }
}

/// Mandatory coverage fixed by the caller (e.g. from a compiled-in trusted inventory).
/// The qualification report and the source inventory can only extend it, never shrink it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedQualificationPolicy {
    required: BTreeSet<String>,
}
impl TrustedQualificationPolicy {
    /// `bytes` must come from a trusted channel (not the report or downloaded release assets).
    pub fn from_inventory_bytes(bytes: &[u8]) -> ContractResult<Self> {
        let required = Inventory::parse(bytes)?.required_ids();
        require(
            !required.is_empty(),
            "trusted inventory has no required coverage",
        )?;
        Ok(Self {
            required,
        })
    }
    pub fn required_ids(&self) -> &BTreeSet<String> {
        &self.required
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LaneInputs {
    rust: String,
    node: String,
    java: String,
    build_profile: String,
    test_profile: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultWire {
    id: String,
    status: String,
    #[serde(default)]
    returncode: Option<i64>,
    #[serde(default)]
    output_sha256: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    evidence_source: Option<String>,
    #[serde(default)]
    job: Option<String>,
}
/// Exact key set written by `tools/dev/ci_tests.py main()` for the aggregated report.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportWire {
    build_inputs: BuildInputs,
    build_inputs_digest: String,
    repository_id: String,
    run_id: String,
    run_attempt: String,
    schema: u32,
    schema_version: u32,
    kind: String,
    source_sha: String,
    inputs: LaneInputs,
    inventory_sha256: String,
    results: Vec<ResultWire>,
    scope: String,
    status: String,
    excluded_coverage: Vec<ResultWire>,
    coverage_complete: bool,
    tests: String,
    test_profile: String,
}

/// Produced only by `verify_public_qualification`; fields are private so a manifest's
/// `tests: passed` alone can never be turned into this value.
#[derive(Clone, Debug)]
pub struct VerifiedPublicQualification {
    evidence: QualificationEvidence,
    reference: QualificationRef,
    inventory_sha256: String,
}
impl VerifiedPublicQualification {
    pub fn evidence(&self) -> &QualificationEvidence {
        &self.evidence
    }
    pub fn reference(&self) -> &QualificationRef {
        &self.reference
    }
    pub fn report_sha256(&self) -> &str {
        &self.reference.sha256
    }
    pub fn inventory_sha256(&self) -> &str {
        &self.inventory_sha256
    }
}

fn validate_result(r: &ResultWire) -> ContractResult<()> {
    require(
        text(&r.id, MAX_ID)
            && matches!(r.status.as_str(), "PASS" | "FAIL" | "BLOCKED" | "EXCLUDED")
            && r.output_sha256.as_ref().is_none_or(|s| digest(s, 64))
            && r.reason.as_ref().is_none_or(|s| text(s, MAX_REASON))
            && r.evidence_source.as_ref().is_none_or(|s| text(s, 64))
            && r.job.as_ref().is_none_or(|s| text(s, MAX_ID)),
        "invalid qualification result",
    )
}

/// Validates a Stable public qualification for `manifest` and returns evidence only on success.
///
/// * `trusted`/`source`: identity policy and the caller's independently resolved run evidence
///   (see module docs; agreement only, not authentication).
/// * `policy`: caller-fixed mandatory coverage; never taken from the report.
/// * `report_bytes`: exact `tests-qualification.json` bytes; must hash to the manifest reference.
/// * `source_inventory_bytes`: the source-SHA `ci_test_inventory.json` bytes; must hash to the
///   report's `inventory_sha256` and must not drop any policy-required ID.
///
/// The Tests job may run inside the same outer publish run as the build; the reference must
/// name exactly the manifest repository/run/attempt.
pub fn verify_public_qualification(
    manifest: &RuntimeManifest,
    trusted: &TrustedSource,
    source: &SourceEvidence,
    policy: &TrustedQualificationPolicy,
    report_bytes: &[u8],
    source_inventory_bytes: &[u8],
) -> ContractResult<VerifiedPublicQualification> {
    require(
        manifest.build().channel == Channel::Stable && manifest.tests() == TestResult::Passed,
        "stable manifest with passed tests required",
    )?;
    validate_source_evidence(manifest, trusted, source)?;
    let qref = manifest
        .qualification_ref()
        .ok_or("qualification reference required")?;
    let reference = QualificationRef::parse(qref)?;
    require(
        reference.repository_id == manifest.repository().id
            && reference.run_id == manifest.run().id
            && reference.run_attempt == manifest.run().attempt,
        "qualification artifact reference identity mismatch",
    )?;
    require(
        !report_bytes.is_empty() && report_bytes.len() <= MAX_QUALIFICATION_BYTES,
        "qualification byte limit exceeded",
    )?;
    require(
        sha256_hex(report_bytes) == reference.sha256,
        "qualification file digest mismatch",
    )?;
    let report: ReportWire = strict_decode(report_bytes, MAX_QUALIFICATION_BYTES)?;
    require(
        report.schema == 1 && report.schema_version == 1 && report.kind == QUALIFICATION_KIND,
        "qualification schema mismatch",
    )?;
    require(
        decimal_u64(&report.repository_id)? == reference.repository_id
            && decimal_u64(&report.run_id)? == reference.run_id
            && decimal_u64(&report.run_attempt)? == reference.run_attempt,
        "qualification run identity mismatch",
    )?;
    require(
        report.source_sha == manifest.source_sha(),
        "qualification source SHA mismatch",
    )?;
    let inputs = manifest.build_inputs();
    require(
        &report.build_inputs == inputs,
        "qualification build inputs mismatch",
    )?;
    require(
        report.build_inputs_digest == inputs.digest(),
        "qualification digest mismatch",
    )?;
    let lane = &report.inputs;
    require(
        lane.rust == inputs.rust_toolchain
            && lane.node == inputs.node_version
            && lane.build_profile == inputs.profile
            && text(&lane.java, 32)
            && lane.test_profile == "ci-test"
            && report.test_profile == "ci-test",
        "qualification lane inputs mismatch",
    )?;
    require(
        text(&report.scope, MAX_REASON),
        "invalid qualification scope",
    )?;
    require(
        report.status == "PASS" && report.tests == "passed" && report.coverage_complete,
        "qualification did not pass",
    )?;
    require(
        digest(&report.inventory_sha256, 64),
        "invalid inventory digest",
    )?;
    require(
        source_inventory_bytes.len() <= MAX_INVENTORY_BYTES,
        "inventory byte limit exceeded",
    )?;
    require(
        sha256_hex(source_inventory_bytes) == report.inventory_sha256,
        "qualification inventory mismatch",
    )?;
    let inventory = Inventory::parse(source_inventory_bytes)?;
    require(
        policy.required.is_subset(&inventory.required_ids()),
        "source inventory drops trusted required coverage",
    )?;
    require(
        report.results.len() <= MAX_INVENTORY_IDS
            && report.results.len() == inventory.jobs.len() + inventory.entries.len(),
        "qualification results do not match inventory",
    )?;
    let mut by_id = std::collections::HashMap::new();
    for r in &report.results {
        validate_result(r)?;
        require(
            by_id.insert(r.id.as_str(), r).is_none(),
            "duplicate qualification result",
        )?;
    }
    let result = |id: &str| {
        by_id
            .get(id)
            .copied()
            .ok_or("qualification results do not match inventory")
    };
    for job in &inventory.jobs {
        require(
            result(job.as_str())?.status == "PASS",
            "required job did not pass",
        )?;
    }
    for e in &inventory.entries {
        let r = result(e.id.as_str())?;
        let excluded_ok = !e.required
            && r.status == "EXCLUDED"
            && e.unavailable.as_ref().is_some_and(|u| !u.is_empty())
            && r.reason == e.unavailable;
        require(
            r.status == "PASS" || excluded_ok,
            "inventory entry did not qualify",
        )?;
    }
    let excluded: Vec<&ResultWire> = report
        .results
        .iter()
        .filter(|r| r.status == "EXCLUDED")
        .collect();
    require(
        report.excluded_coverage.len() == excluded.len()
            && report
                .excluded_coverage
                .iter()
                .zip(excluded)
                .all(|(a, b)| a == b),
        "excluded coverage mismatch",
    )?;
    let evidence = QualificationEvidence {
        repository: manifest.repository().clone(),
        workflow: trusted.workflow.clone(),
        run: manifest.run().clone(),
        source_sha: manifest.source_sha().into(),
        build_inputs_digest: report.build_inputs_digest,
        status: source.status.clone(),
        conclusion: source.conclusion.clone(),
        tests: TestResult::Passed,
    };
    validate_qualification(manifest, Channel::Stable, trusted, Some(&evidence))?;
    Ok(VerifiedPublicQualification {
        evidence,
        reference,
        inventory_sha256: report.inventory_sha256,
    })
}
