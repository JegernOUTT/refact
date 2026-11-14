use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::collections::BTreeMap;

pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;
pub const MAX_ARTIFACTS: usize = 64;
pub const MAX_EXTRACTED_FILES: usize = 256;
pub type ContractResult<T> = Result<T, String>;

pub(crate) fn require(ok: bool, message: &str) -> ContractResult<()> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
pub(crate) fn text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
pub(crate) fn digest(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn safe_path(s: &str) -> bool {
    text(s, 512)
        && !s.starts_with('/')
        && !s.contains(['\\', ':'])
        && s.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
fn portable_segment(s: &str) -> bool {
    let base = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.ends_with(['.', ' '])
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
        && !matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}
fn version(s: &str) -> bool {
    if !text(s, 128) {
        return false;
    }
    let (core, suffix) = s.split_once('-').map_or((s, None), |(a, b)| (a, Some(b)));
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
        && suffix.is_none_or(|v| {
            v.split(['.', '-'])
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric()))
        })
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    limit: usize,
) -> ContractResult<T> {
    require(bytes.len() <= limit, "JSON byte limit exceeded")?;
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub owner: String,
    pub name: String,
    pub id: u64,
}
impl Repository {
    pub fn validate(&self) -> ContractResult<()> {
        require(
            self.id > 0
                && [&self.owner, &self.name].iter().all(|s| {
                    text(s, 100)
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                        && *s != "."
                        && *s != ".."
                }),
            "invalid repository",
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub path: String,
    pub id: u64,
}
impl Workflow {
    pub fn validate(&self) -> ContractResult<()> {
        require(
            self.id > 0
                && safe_path(&self.path)
                && self.path.starts_with(".github/workflows/")
                && (self.path.ends_with(".yml") || self.path.ends_with(".yaml")),
            "invalid workflow",
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Stable,
    Preview,
    Nightly,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub id: u64,
    pub attempt: u64,
}
impl Run {
    pub fn validate(&self) -> ContractResult<()> {
        require(self.id > 0 && self.attempt > 0, "invalid run")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub version: String,
    pub channel: Channel,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentVersions {
    pub runtime: String,
    pub gui: String,
    pub vscode: String,
    pub jetbrains: String,
}
impl ComponentVersions {
    pub fn validate(&self) -> ContractResult<()> {
        require(
            version(&self.runtime)
                && [&self.gui, &self.vscode, &self.jetbrains].iter().all(|v| {
                    text(v, 128)
                        && v.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b".-+_".contains(&b))
                }),
            "invalid component version",
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interval {
    pub min: u32,
    pub max: u32,
}
impl Interval {
    pub fn contains(&self, value: u32) -> bool {
        self.min <= value && value <= self.max
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    pub daemon_api: Interval,
    pub worker_api: Interval,
    pub host_bridge: Interval,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    Runtime,
    Gui,
    Vscode,
    Jetbrains,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    TarGz,
    Zip,
    Vsix,
    Jar,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub component: Component,
    pub target: String,
    pub filename: String,
    pub format: ArtifactFormat,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extracted_files: Vec<ExtractedFile>,
}
pub fn native_target(target: &str) -> bool {
    matches!(
        target,
        "x86_64-unknown-linux-gnu"
            | "aarch64-unknown-linux-gnu"
            | "x86_64-apple-darwin"
            | "aarch64-apple-darwin"
            | "x86_64-pc-windows-msvc"
            | "i686-pc-windows-msvc"
            | "aarch64-pc-windows-msvc"
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestResult {
    Passed,
    Failed,
    NotRun,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildInputs {
    pub rust_toolchain: String,
    pub node_version: String,
    pub profile: String,
    pub cargo_lock_sha256: String,
    /// SHA256 of the compact sorted path-to-SHA256 JSON map produced by javascript_locks_digest.
    pub javascript_locks_sha256: String,
    pub config_sha256: String,
}
impl BuildInputs {
    pub fn javascript_locks_digest(lockfiles: &BTreeMap<String, String>) -> ContractResult<String> {
        require(
            !lockfiles.is_empty()
                && lockfiles.len() <= 32
                && lockfiles
                    .iter()
                    .all(|(path, hash)| safe_path(path) && digest(hash, 64)),
            "invalid JavaScript lockfile inventory",
        )?;
        Ok(hex::encode(Sha256::digest(
            serde_json::to_vec(lockfiles).map_err(|e| e.to_string())?,
        )))
    }
    fn validate(&self) -> ContractResult<()> {
        require(
            [&self.rust_toolchain, &self.node_version, &self.profile]
                .iter()
                .all(|v| text(v, 128))
                && [
                    &self.cargo_lock_sha256,
                    &self.javascript_locks_sha256,
                    &self.config_sha256,
                ]
                .iter()
                .all(|v| digest(v, 64)),
            "invalid build inputs",
        )
    }
    pub fn digest(&self) -> String {
        hex::encode(Sha256::digest(
            serde_json::to_vec(self).expect("build inputs serialization"),
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestWire {
    schema_version: u32,
    repository: Repository,
    source_sha: String,
    workflow: Workflow,
    run: Run,
    build: Build,
    build_inputs: BuildInputs,
    component_versions: ComponentVersions,
    compatibility: Compatibility,
    artifacts: Vec<Artifact>,
    tests: TestResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    qualification_ref: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ManifestWire", into = "ManifestWire")]
pub struct RuntimeManifest(ManifestWire);
impl From<RuntimeManifest> for ManifestWire {
    fn from(v: RuntimeManifest) -> Self {
        v.0
    }
}
impl TryFrom<ManifestWire> for RuntimeManifest {
    type Error = String;
    fn try_from(mut w: ManifestWire) -> ContractResult<Self> {
        require(
            serde_json::to_vec(&w).map_err(|e| e.to_string())?.len() <= MAX_MANIFEST_BYTES,
            "manifest byte limit exceeded",
        )?;
        require(w.schema_version == 1, "unsupported manifest schema")?;
        w.repository.validate()?;
        w.workflow.validate()?;
        w.run.validate()?;
        require(digest(&w.source_sha, 40), "invalid source SHA")?;
        require(version(&w.build.version), "invalid build version")?;
        w.build_inputs.validate()?;
        w.component_versions.validate()?;
        for i in [
            &w.compatibility.daemon_api,
            &w.compatibility.worker_api,
            &w.compatibility.host_bridge,
        ] {
            require(i.min <= i.max, "invalid compatibility interval")?;
        }
        require(
            !w.artifacts.is_empty() && w.artifacts.len() <= MAX_ARTIFACTS,
            "artifact count rejected",
        )?;
        let mut names = HashSet::new();
        let mut targets = HashSet::new();
        let mut runtime = false;
        for a in &mut w.artifacts {
            require(
                native_target(&a.target)
                    || (a.component != Component::Runtime && a.target == "universal"),
                "invalid artifact target",
            )?;
            require(
                portable_segment(&a.filename)
                    && !a.filename.contains('/')
                    && a.filename.len() <= 200
                    && a.size > 0
                    && digest(&a.sha256, 64),
                "invalid artifact identity",
            )?;
            require(
                names.insert(a.filename.to_ascii_lowercase())
                    && targets.insert((format!("{:?}", a.component), a.target.clone())),
                "duplicate artifact",
            )?;
            let format_ok = match a.component {
                Component::Runtime => {
                    runtime = true;
                    if a.target.contains("windows") {
                        a.format == ArtifactFormat::Zip
                    } else {
                        a.format == ArtifactFormat::TarGz
                    }
                }
                Component::Vscode => a.format == ArtifactFormat::Vsix,
                Component::Jetbrains => {
                    matches!(a.format, ArtifactFormat::Zip | ArtifactFormat::Jar)
                }
                Component::Gui => a.format == ArtifactFormat::TarGz,
            };
            require(format_ok, "component/target format mismatch")?;
            require(
                a.extracted_files.len() <= MAX_EXTRACTED_FILES,
                "extracted file count rejected",
            )?;
            let mut paths = HashSet::new();
            for f in &a.extracted_files {
                require(
                    safe_path(&f.path)
                        && f.path.split('/').all(portable_segment)
                        && f.size > 0
                        && digest(&f.sha256, 64)
                        && paths.insert(
                            if a.target.contains("windows")
                                || a.target.contains("apple")
                                || a.target == "universal"
                            {
                                f.path.to_ascii_lowercase()
                            } else {
                                f.path.clone()
                            },
                        ),
                    "invalid extracted file",
                )?;
            }
            a.extracted_files.sort_by(|a, b| a.path.cmp(&b.path));
        }
        require(runtime, "runtime bundle required")?;
        require(
            w.qualification_ref.as_ref().is_none_or(|s| text(s, 512)),
            "invalid qualification reference",
        )?;
        w.artifacts.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(Self(w))
    }
}
impl RuntimeManifest {
    pub fn from_json(bytes: &[u8]) -> ContractResult<Self> {
        decode(bytes, MAX_MANIFEST_BYTES)
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.0).expect("manifest serialization")
    }
    pub fn digest(&self) -> String {
        hex::encode(Sha256::digest(self.canonical_bytes()))
    }
    pub fn repository(&self) -> &Repository {
        &self.0.repository
    }
    pub fn workflow(&self) -> &Workflow {
        &self.0.workflow
    }
    pub fn run(&self) -> &Run {
        &self.0.run
    }
    pub fn source_sha(&self) -> &str {
        &self.0.source_sha
    }
    pub fn build(&self) -> &Build {
        &self.0.build
    }
    pub fn build_inputs(&self) -> &BuildInputs {
        &self.0.build_inputs
    }
    pub fn component_versions(&self) -> &ComponentVersions {
        &self.0.component_versions
    }
    pub fn artifacts(&self) -> &[Artifact] {
        &self.0.artifacts
    }
    pub fn compatibility(&self) -> &Compatibility {
        &self.0.compatibility
    }
    pub fn tests(&self) -> TestResult {
        self.0.tests
    }
    pub fn qualification_ref(&self) -> Option<&str> {
        self.0.qualification_ref.as_deref()
    }
    pub fn check_protocols(&self, daemon: u32, worker: u32, bridge: u32) -> ContractResult<()> {
        let c = &self.0.compatibility;
        require(
            c.daemon_api.contains(daemon)
                && c.worker_api.contains(worker)
                && c.host_bridge.contains(bridge),
            "protocol mismatch",
        )
    }
    pub(crate) fn check_channel(&self, channel: Channel) -> ContractResult<Qualification> {
        require(self.0.build.channel == channel, "channel mismatch")?;
        match self.0.tests {
            TestResult::Passed => Ok(Qualification::Tested),
            TestResult::NotRun if channel != Channel::Stable => Ok(Qualification::Untested),
            _ => Err("test qualification rejected".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Qualification {
    Tested,
    Untested,
}
#[derive(Clone, Debug)]
pub struct TrustedSource {
    pub repository: Repository,
    pub workflow: Workflow,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceEvidence {
    pub repository: Repository,
    pub workflow: Workflow,
    pub run: Run,
    pub source_sha: String,
    pub status: String,
    pub conclusion: String,
}
pub fn validate_source_evidence(
    manifest: &RuntimeManifest,
    trusted: &TrustedSource,
    evidence: &SourceEvidence,
) -> ContractResult<()> {
    trusted.repository.validate()?;
    trusted.workflow.validate()?;
    require(
        manifest.repository() == &trusted.repository && manifest.workflow() == &trusted.workflow,
        "untrusted repository/workflow",
    )?;
    require(
        evidence.repository == trusted.repository
            && evidence.workflow == trusted.workflow
            && evidence.run == *manifest.run()
            && evidence.source_sha == manifest.source_sha()
            && evidence.status == "completed"
            && evidence.conclusion == "success",
        "source evidence mismatch or incomplete run",
    )
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationEvidence {
    pub repository: Repository,
    pub workflow: Workflow,
    pub run: Run,
    pub source_sha: String,
    pub build_inputs_digest: String,
    pub status: String,
    pub conclusion: String,
    pub tests: TestResult,
}
pub fn validate_qualification(
    manifest: &RuntimeManifest,
    channel: Channel,
    trusted_tests: &TrustedSource,
    evidence: Option<&QualificationEvidence>,
) -> ContractResult<Qualification> {
    let declared = manifest.check_channel(channel)?;
    if let Some(e) = evidence {
        trusted_tests.repository.validate()?;
        trusted_tests.workflow.validate()?;
        e.run.validate()?;
        require(
            e.repository == trusted_tests.repository
                && e.repository == *manifest.repository()
                && e.workflow == trusted_tests.workflow
                && e.source_sha == manifest.source_sha()
                && e.build_inputs_digest == manifest.build_inputs().digest()
                && e.status == "completed"
                && e.conclusion == "success"
                && e.tests == TestResult::Passed
                && (declared == Qualification::Tested || channel != Channel::Stable),
            "qualification evidence mismatch",
        )?;
        Ok(Qualification::Tested)
    } else {
        require(
            channel != Channel::Stable && declared == Qualification::Untested,
            "independent tests evidence required",
        )?;
        Ok(Qualification::Untested)
    }
}
