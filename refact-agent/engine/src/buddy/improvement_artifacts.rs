use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::DeploymentTarget;
use crate::buddy::improvement_evaluation::digest;
use crate::buddy::operator_store::{OperatorStoreError, Stored};
use crate::buddy::outcome_runtime::OutcomeRuntime;
use crate::buddy::resources::*;
use crate::worktrees::service::WorktreeService;

pub const ADAPTER: &str = "candidate_artifact";
pub const ACTION: &str = "stage_exact_artifact";
const NAMESPACE: &str = "improvement_artifacts";
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 4096;
const MAX_RETAINED: usize = 8;
const COMPATIBILITY_STATUS: &str = "structure_checked; metadata explicitly operator-attested, not build proof; ELF System V/Linux ABI is not OS proof; runtime compatibility and build provenance unverified; not installed or running";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    LinuxElf64,
    WindowsPe64,
    MacosMach64,
    JetbrainsZip,
    GuiDist,
    VscodeVsix,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactDraft {
    pub project: ProjectIdentity,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub proof_id: String,
    pub worktree_id: String,
    pub relative_path: String,
    pub format: ArtifactFormat,
    pub artifact_digest: String,
    pub manifest_digest: String,
    pub attested_version: String,
    pub attested_os: String,
    pub attested_arch: String,
    pub attested_host_requirement: String,
    pub retained_known_good: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactInspection {
    pub artifact_digest: String,
    pub manifest_digest: String,
    pub bytes: u64,
    pub files: BTreeMap<String, String>,
    pub checked_arch: Option<String>,
    pub package_version: Option<String>,
    pub package_host_requirement: Option<String>,
    pub compatibility_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ArtifactRecord {
    schema_version: u16,
    project: ProjectIdentity,
    draft: ArtifactDraft,
    inspection: ArtifactInspection,
}

fn err(message: impl std::fmt::Display) -> ResourceError {
    ResourceError::ValidationFailed(message.to_string())
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn validate_attestation(draft: &ArtifactDraft) -> Result<(), ResourceError> {
    use crate::buddy::improvement_evaluation::valid_digest;
    relative(&draft.relative_path)?;
    if !Path::new(&draft.project.canonical_root).is_absolute()
        || [
            &draft.project.project_id,
            &draft.candidate_id,
            &draft.proof_id,
            &draft.worktree_id,
            &draft.attested_version,
            &draft.attested_os,
            &draft.attested_arch,
            &draft.attested_host_requirement,
        ]
        .iter()
        .any(|v| !label(v))
        || [
            &draft.candidate_digest,
            &draft.artifact_digest,
            &draft.manifest_digest,
        ]
        .iter()
        .any(|v| !valid_digest(v))
        || draft.retained_known_good.len() >= MAX_RETAINED
    {
        return Err(err("invalid artifact attestation"));
    }
    let mut pins = std::collections::BTreeSet::new();
    for pin in &draft.retained_known_good {
        if !valid_digest(pin) || pin == &draft.artifact_digest || !pins.insert(pin) {
            return Err(err("invalid or duplicate retained artifact reference"));
        }
    }
    if draft.format == ArtifactFormat::LinuxElf64
        && (draft.attested_os != "linux"
            || !matches!(draft.attested_arch.as_str(), "x86_64" | "aarch64"))
    {
        return Err(err("unsupported ELF target attestation"));
    }
    let native_os = match draft.format {
        ArtifactFormat::WindowsPe64 => Some("windows"),
        ArtifactFormat::MacosMach64 => Some("macos"),
        _ => None,
    };
    if native_os.is_some_and(|os| draft.attested_os != os)
        || (native_os.is_some() && !matches!(draft.attested_arch.as_str(), "x86_64" | "aarch64"))
        || (draft.format == ArtifactFormat::JetbrainsZip && !draft.relative_path.ends_with(".zip"))
    {
        return Err(err("unsupported native/package attestation"));
    }
    if draft.format == ArtifactFormat::GuiDist && !draft.relative_path.ends_with("/dist") {
        return Err(err("GUI artifact must be native dist directory"));
    }
    if draft.format == ArtifactFormat::VscodeVsix && !draft.relative_path.ends_with(".vsix") {
        return Err(err("VSIX artifact required"));
    }
    Ok(())
}

fn validate_inspection(
    inspection: &ArtifactInspection,
    draft: &ArtifactDraft,
) -> Result<(), ResourceError> {
    validate_attestation(draft)?;
    if inspection.artifact_digest != draft.artifact_digest
        || inspection.manifest_digest != draft.manifest_digest
        || digest(&inspection.files).map_err(err)? != inspection.manifest_digest
        || inspection.bytes == 0
        || inspection.bytes > MAX_BYTES
        || inspection.files.is_empty()
        || inspection.files.len() > MAX_FILES
        || inspection.compatibility_status != COMPATIBILITY_STATUS
    {
        return Err(err("artifact content or manifest differs from attestation"));
    }
    for (name, value) in &inspection.files {
        relative(name)?;
        if !crate::buddy::improvement_evaluation::valid_digest(value) {
            return Err(err("invalid inspected file digest"));
        }
    }
    match draft.format {
        ArtifactFormat::LinuxElf64 | ArtifactFormat::WindowsPe64 | ArtifactFormat::MacosMach64 => {
            if inspection.checked_arch.as_deref() != Some(draft.attested_arch.as_str())
                || inspection.package_version.is_some()
                || inspection.package_host_requirement.is_some()
                || inspection.files
                    != BTreeMap::from([("engine".into(), draft.artifact_digest.clone())])
            {
                return Err(err("ELF inspection target mismatch"));
            }
        }
        ArtifactFormat::GuiDist | ArtifactFormat::VscodeVsix | ArtifactFormat::JetbrainsZip => {
            if inspection.checked_arch.is_some()
                || inspection.package_version.as_deref() != Some(draft.attested_version.as_str())
                || (matches!(
                    draft.format,
                    ArtifactFormat::VscodeVsix | ArtifactFormat::JetbrainsZip
                ) && inspection.package_host_requirement.as_deref()
                    != Some(draft.attested_host_requirement.as_str()))
                || (draft.format == ArtifactFormat::GuiDist
                    && inspection.package_host_requirement.is_some())
            {
                return Err(err("package inspection metadata mismatch"));
            }
        }
    }
    Ok(())
}

pub fn staged_readiness(
    store: &crate::buddy::operator_store::OperatorStore,
    candidate: &super::CandidateRecord,
) -> Result<bool, OperatorStoreError> {
    let Some(id) = candidate.candidate.build_identity.as_deref() else {
        return Ok(false);
    };
    let Some(record) = store.get_document::<ArtifactRecord>(NAMESPACE, id)? else {
        return Ok(false);
    };
    Ok(record.value.schema_version == 1
        && record.value.project == candidate.project
        && record.value.draft.project == candidate.project
        && record.value.draft.artifact_digest == id
        && validate_inspection(&record.value.inspection, &record.value.draft).is_ok()
        && record.value.draft.candidate_digest == digest(candidate)?
        && record.value.draft.candidate_id == candidate.candidate.candidate_id
        && verify_blob(&candidate.project, id).unwrap_or(false))
}

/// Native installer handoff: only a recorded, evaluated, privately staged image.
/// The installer must separately establish host compatibility; this is not it.
#[cfg(target_os = "linux")]
pub(super) fn installation_image(
    store: &crate::buddy::operator_store::OperatorStore,
    candidate: &super::CandidateRecord,
) -> Result<(ArtifactDraft, Vec<u8>), ResourceError> {
    if candidate.target != DeploymentTarget::EngineBuild
        || !staged_readiness(store, candidate).map_err(err)?
    {
        return Err(err("exact evaluated engine stage unavailable"));
    }
    let id = candidate
        .candidate
        .build_identity
        .as_deref()
        .ok_or_else(|| err("build missing"))?;
    let record = store
        .get_document::<ArtifactRecord>(NAMESPACE, id)
        .map_err(err)?
        .ok_or_else(|| err("stage missing"))?
        .value;
    if record.draft.format != ArtifactFormat::LinuxElf64 {
        return Err(err("native installation supports Linux ELF only"));
    }
    let root = safe::root(Path::new(&candidate.project.canonical_root))?;
    let bytes = read_file(
        safe::open(&root, &format!(".refact/candidate-artifacts/{id}"), false)?,
        MAX_BYTES,
    )?;
    let inspected = inspect_bytes(
        &record.draft.format,
        bytes.clone(),
        BTreeMap::from([("engine".into(), bytes.clone())]),
    )?
    .0;
    validate_inspection(&inspected, &record.draft)?;
    if inspected != record.inspection {
        return Err(err("staged inspection changed"));
    }
    Ok((record.draft, bytes))
}

/// Reinspect retained package bytes at the release boundary. Archive metadata is
/// never an installed/running observation. GUI dist must travel in its owner bundle.
#[cfg(target_os = "linux")]
pub(super) fn release_package(
    store: &crate::buddy::operator_store::OperatorStore,
    candidate: &super::CandidateRecord,
) -> Result<
    (
        ArtifactDraft,
        ArtifactInspection,
        Vec<u8>,
        BTreeMap<String, String>,
    ),
    ResourceError,
> {
    if !staged_readiness(store, candidate).map_err(err)? {
        return Err(err("exact evaluated package stage unavailable"));
    }
    let id = candidate
        .candidate
        .build_identity
        .as_deref()
        .ok_or_else(|| err("build missing"))?;
    let record = store
        .get_document::<ArtifactRecord>(NAMESPACE, id)
        .map_err(err)?
        .ok_or_else(|| err("stage missing"))?
        .value;
    if !matches!(
        record.draft.format,
        ArtifactFormat::VscodeVsix | ArtifactFormat::JetbrainsZip
    ) {
        return Err(err("GUI dist is embedded in the daemon or IDE package; stage and grant its matching owner bundle"));
    }
    let root = safe::root(Path::new(&candidate.project.canonical_root))?;
    let bytes = read_file(
        safe::open(&root, &format!(".refact/candidate-artifacts/{id}"), false)?,
        MAX_BYTES,
    )?;
    let files = zip_files(&bytes)?;
    let inspected = inspect_bytes(&record.draft.format, bytes.clone(), files)?.0;
    validate_inspection(&inspected, &record.draft)?;
    if inspected != record.inspection {
        return Err(err("retained package inspection changed"));
    }
    let mut installed_files = BTreeMap::new();
    for (path, hash) in &inspected.files {
        let relative = match record.draft.format {
            ArtifactFormat::VscodeVsix => path.strip_prefix("extension/"),
            ArtifactFormat::JetbrainsZip => path.split_once('/').map(|(_, tail)| tail),
            _ => None,
        };
        if let Some(path) = relative {
            // VS Code mutates package.json (__metadata); it is observed for version
            // separately, never used as byte-identity evidence for loaded code.
            if record.draft.format == ArtifactFormat::VscodeVsix && path == "package.json" {
                continue;
            }
            installed_files.insert(path.to_string(), hash.clone());
        }
    }
    if installed_files.is_empty() {
        return Err(err("installed code manifest empty"));
    }
    Ok((record.draft, inspected, bytes, installed_files))
}

/// Exact GUI-to-owner package binding. Daemon GUI is RustEmbed data, not a
/// mutable dist directory; IDE GUI is packaged resources. This describes the
/// owning release action without implying that a browser has loaded those bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuiOwnerBinding {
    pub gui_artifact_digest: String,
    pub owner_artifact_digest: String,
    pub serving_files: BTreeMap<String, String>,
    pub bundle_bytes_verified: bool,
    pub required_owner: String,
}
#[cfg(target_os = "linux")]
pub(crate) fn gui_owner_binding(
    store: &crate::buddy::operator_store::OperatorStore,
    gui: &super::CandidateRecord,
    owner: &super::CandidateRecord,
) -> Result<GuiOwnerBinding, ResourceError> {
    if gui.target != DeploymentTarget::GuiBuild
        || !staged_readiness(store, gui).map_err(err)?
        || !staged_readiness(store, owner).map_err(err)?
    {
        return Err(err("evaluated GUI and owner stages required"));
    }
    let gui_id = gui
        .candidate
        .build_identity
        .as_deref()
        .ok_or_else(|| err("GUI build missing"))?;
    let owner_id = owner
        .candidate
        .build_identity
        .as_deref()
        .ok_or_else(|| err("owner build missing"))?;
    let gui_record = store
        .get_document::<ArtifactRecord>(NAMESPACE, gui_id)
        .map_err(err)?
        .ok_or_else(|| err("GUI stage missing"))?
        .value;
    if gui_record.draft.format != ArtifactFormat::GuiDist {
        return Err(err("GUI format mismatch"));
    }
    let serving_files: BTreeMap<String, String> = ["chat/index.umd.cjs", "chat/style.css"]
        .into_iter()
        .map(|path| {
            Ok((
                path.to_string(),
                gui_record
                    .inspection
                    .files
                    .get(path)
                    .ok_or_else(|| err("GUI serving entry missing"))?
                    .clone(),
            ))
        })
        .collect::<Result<_, ResourceError>>()?;
    let (required_owner, verified) = if owner.target == DeploymentTarget::EngineBuild {
        // An ELF string scan cannot prove which embedded bytes the server uses.
        // The exact owner grant installs/restarts the engine; after restart the
        // native running report independently reads ChatGuiAsset and kernel hash.
        ("daemon_engine", false)
    } else {
        let (artifact, _, bytes, _) = release_package(store, owner)?;
        let files = zip_files(&bytes)?;
        let verified = match artifact.format {
            ArtifactFormat::VscodeVsix => serving_files.iter().all(|(path, expected)| {
                files
                    .get(&format!(
                        "extension/node_modules/refact-chat-js/dist/{path}"
                    ))
                    .is_some_and(|b| hash(b) == *expected)
            }),
            ArtifactFormat::JetbrainsZip => {
                let mut matched = BTreeMap::new();
                let mut expanded = files.values().map(|b| b.len() as u64).sum::<u64>();
                let mut entries = files.len();
                for (path, bytes) in &files {
                    if !path.ends_with(".jar") {
                        continue;
                    }
                    let nested = zip_files_bounded(
                        bytes,
                        MAX_BYTES.saturating_sub(expanded),
                        MAX_FILES.saturating_sub(entries),
                    )?;
                    expanded += nested.values().map(|b| b.len() as u64).sum::<u64>();
                    entries += nested.len();
                    for (path, expected) in &serving_files {
                        if let Some(b) = nested.get(&format!("dist/{path}")) {
                            if matched.insert(path.clone(), hash(b) == *expected).is_some() {
                                return Err(err("ambiguous bundled GUI resource"));
                            }
                        }
                    }
                }
                matched.len() == serving_files.len() && matched.values().all(|v| *v)
            }
            _ => false,
        };
        if !verified {
            return Err(err("GUI dist differs from embedded owner package"));
        }
        (
            if artifact.format == ArtifactFormat::VscodeVsix {
                "vscode"
            } else {
                "jetbrains"
            },
            true,
        )
    };
    Ok(GuiOwnerBinding {
        gui_artifact_digest: gui_id.into(),
        owner_artifact_digest: owner_id.into(),
        serving_files,
        bundle_bytes_verified: verified,
        required_owner: required_owner.into(),
    })
}

/// Native daemon serving identity. Reads compiled-in assets from this process,
/// not package declarations or a caller-provided GUI build string. The caller
/// must additionally bind a fresh daemon generation and kernel engine image.
pub(crate) fn running_gui_files() -> Result<BTreeMap<String, String>, ResourceError> {
    let mut files = BTreeMap::new();
    for path in ["chat/index.umd.cjs", "chat/style.css"] {
        let asset = crate::http::routers::gui::ChatGuiAsset::get(&format!("dist/{path}"))
            .ok_or_else(|| err("native embedded GUI asset missing"))?;
        files.insert(path.to_string(), hash(asset.data.as_ref()));
    }
    Ok(files)
}

/// Native daemon observation. In debug builds RustEmbed reads disk, so it is
/// explicitly not a running embedded build identity. The browser's loaded asset
/// identity is never inferred from this server-side observation.
#[cfg(target_os = "linux")]
pub fn observe_daemon_gui(binding: &GuiOwnerBinding) -> Result<serde_json::Value, ResourceError> {
    if binding.required_owner != "daemon_engine" {
        return Err(err("not daemon-owned GUI"));
    }
    let kernel = refact_self_update::candidate_install::running_digest().map_err(err)?;
    let mut observed = BTreeMap::new();
    for path in ["chat/index.umd.cjs", "chat/style.css"] {
        let asset = crate::http::routers::gui::ChatGuiAsset::get(&format!("dist/{path}"))
            .ok_or_else(|| err("embedded GUI unavailable"))?;
        observed.insert(path.to_string(), hash(&asset.data));
    }
    let verified = !cfg!(debug_assertions)
        && kernel == binding.owner_artifact_digest
        && observed == binding.serving_files;
    Ok(
        serde_json::json!({"schema_version":1,"owner":"daemon_engine","kernel_digest":kernel,
        "serving_files":observed,"embedded_owner_verified":verified,"browser_loaded_verified":false,
        "provenance":if cfg!(debug_assertions) { "debug_disk_resources_not_embedded_proof" } else { "kernel_image_and_native_RustEmbed_bytes" }}),
    )
}

/// Structural dependency inventory, NOT runtime proof. A canary receipt must
/// bind the serialized snapshot, artifact and fixed protocol; install re-snapshots
/// the host and requires equality. Presence of these files proves no ABI symbols.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinuxRuntimeSnapshot {
    pub arch: String,
    pub interpreter: String,
    pub kernel_release: String,
    /// Guest absolute loader search path aliases -> immutable source binding.
    pub files: BTreeMap<String, LinuxRuntimeFile>,
}
#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinuxRuntimeFile {
    pub canonical_path: String,
    pub digest: String,
}

/// Narrow system-only lookup roots. Canary must mount these individual files,
/// not the directories: no ld.so.cache/preload/config, hwcaps, user or project roots.
#[cfg(target_os = "linux")]
pub(crate) fn linux_library_roots(arch: &str) -> Result<Vec<String>, ResourceError> {
    let triple = match arch {
        "x86_64" => "x86_64-linux-gnu",
        "aarch64" => "aarch64-linux-gnu",
        _ => return Err(err("unsupported runtime architecture")),
    };
    Ok(vec![
        format!("/usr/lib/{triple}"),
        format!("/lib/{triple}"),
        "/usr/lib64".into(),
        "/lib64".into(),
        "/usr/lib".into(),
        "/lib".into(),
    ])
}

#[cfg(target_os = "linux")]
fn trusted_linux_runtime_file(path: &Path) -> Result<(LinuxRuntimeFile, Vec<u8>), ResourceError> {
    use std::os::unix::fs::MetadataExt;
    if !path.is_absolute()
        || path.components().any(|p| {
            !matches!(
                p,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err(err("noncanonical runtime path"));
    }
    let canonical = path.canonicalize()?;
    if !["/usr/lib", "/usr/lib64", "/lib", "/lib64"]
        .iter()
        .any(|root| canonical.starts_with(root))
    {
        return Err(err("runtime outside canonical trusted system libraries"));
    }
    // Check both the requested alias and every canonical ancestor. Symlink
    // permission bits are meaningless, but even alias links must be root-owned.
    for leaf in [path, canonical.as_path()] {
        let mut parent = Some(leaf);
        while let Some(p) = parent {
            let m = std::fs::symlink_metadata(p)?;
            if m.uid() != 0 || (!m.file_type().is_symlink() && m.mode() & 0o022 != 0) {
                return Err(err("runtime path not root-owned and nonwritable"));
            }
            parent = p.parent();
        }
    }
    let directory = safe::root(canonical.parent().ok_or_else(|| err("runtime parent"))?)?;
    let file = safe::child(
        &directory,
        canonical
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| err("runtime name"))?,
        false,
        false,
    )?;
    let m = file.metadata()?;
    if m.uid() != 0 || m.mode() & 0o022 != 0 {
        return Err(err("runtime file permissions changed"));
    }
    let bytes = read_file(file, 64 * 1024 * 1024)?;
    if path.canonicalize()? != canonical {
        return Err(err("runtime alias changed"));
    }
    Ok((
        LinuxRuntimeFile {
            canonical_path: canonical.to_string_lossy().into_owned(),
            digest: hash(&bytes),
        },
        bytes,
    ))
}

/// Only these freshly verified bytes may be copied into the private canary
/// sandbox. No candidate-supplied source paths, env vars or commands are accepted.
#[cfg(target_os = "linux")]
pub(crate) fn linux_runtime_file_bytes(file: &LinuxRuntimeFile) -> Result<Vec<u8>, ResourceError> {
    let (observed, bytes) = trusted_linux_runtime_file(Path::new(&file.canonical_path))?;
    if &observed != file {
        return Err(err("runtime fingerprint changed"));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_runtime_snapshot(
    bytes: &[u8],
    arch: &str,
) -> Result<LinuxRuntimeSnapshot, ResourceError> {
    if arch != std::env::consts::ARCH {
        return Err(err("foreign runtime architecture"));
    }
    let image = linux_dependencies(bytes, arch)?;
    if !image.executable_entry {
        return Err(err("entry outside executable file segment"));
    }
    let expected = match arch {
        "x86_64" => "/lib64/ld-linux-x86-64.so.2",
        "aarch64" => "/lib/ld-linux-aarch64.so.1",
        _ => return Err(err("unsupported runtime architecture")),
    };
    if image.interpreter.as_deref() != Some(expected) {
        return Err(err("unknown dynamic interpreter"));
    }
    let roots = linux_library_roots(arch)?;
    let mut files = BTreeMap::new();
    let mut queue = vec![(
        expected.to_string(),
        trusted_linux_runtime_file(Path::new(expected))?,
    )];
    let mut names = std::collections::BTreeSet::new();
    let mut pending = image.needed;
    let mut total = 0usize;
    loop {
        while let Some((path, (fingerprint, content))) = queue.pop() {
            if files.contains_key(&path) {
                continue;
            }
            total = total
                .checked_add(content.len())
                .ok_or_else(|| err("runtime size overflow"))?;
            if files.len() >= 64 || total > 256 * 1024 * 1024 {
                return Err(err("runtime closure bound"));
            }
            let dependency = linux_dependencies(&content, arch)?;
            // glibc itself has PT_INTERP for diagnostic execution; it may only
            // name the same trusted loader, never an alternate interpreter.
            if dependency
                .interpreter
                .as_deref()
                .is_some_and(|p| p != expected)
            {
                return Err(err("foreign interpreter in shared dependency"));
            }
            pending.extend(dependency.needed);
            files.insert(path, fingerprint);
        }
        let Some(name) = pending.pop() else {
            break;
        };
        if !names.insert(name.clone()) {
            continue;
        }
        if names.len() > 64 {
            return Err(err("runtime dependency count"));
        }
        let mut found: Option<String> = None;
        for root in &roots {
            let path = Path::new(root).join(&name);
            if !path.try_exists()? {
                continue;
            }
            let (fingerprint, content) = trusted_linux_runtime_file(&path)?;
            if found.as_ref().is_some_and(|old| old != &fingerprint.digest) {
                return Err(err("ambiguous system dependency resolution"));
            }
            found = Some(fingerprint.digest.clone());
            queue.push((path.to_string_lossy().into_owned(), (fingerprint, content)));
        }
        if found.is_none() {
            return Err(err("system dependency unavailable"));
        }
    }
    let mut uts = std::mem::MaybeUninit::<libc::utsname>::uninit();
    if unsafe { libc::uname(uts.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let uts = unsafe { uts.assume_init() };
    let kernel_release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) }
        .to_str()
        .map_err(err)?
        .to_string();
    Ok(LinuxRuntimeSnapshot {
        arch: arch.into(),
        interpreter: expected.into(),
        kernel_release,
        files,
    })
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct LinuxDependencies {
    interpreter: Option<String>,
    needed: Vec<String>,
    executable_entry: bool,
}

/// Bounded ELF64 PT_DYNAMIC parser. No section table, loader execution or ldd.
#[cfg(target_os = "linux")]
fn linux_dependencies(bytes: &[u8], arch: &str) -> Result<LinuxDependencies, ResourceError> {
    let read = |at: usize, n: usize| {
        bytes
            .get(at..at.checked_add(n).ok_or_else(|| err("ELF overflow"))?)
            .ok_or_else(|| err("truncated ELF"))
    };
    let u16at = |at| -> Result<u16, ResourceError> {
        Ok(u16::from_le_bytes(read(at, 2)?.try_into().map_err(err)?))
    };
    let u32at = |at| -> Result<u32, ResourceError> {
        Ok(u32::from_le_bytes(read(at, 4)?.try_into().map_err(err)?))
    };
    let u64at = |at| -> Result<u64, ResourceError> {
        Ok(u64::from_le_bytes(read(at, 8)?.try_into().map_err(err)?))
    };
    if read(0, 7)? != b"\x7fELF\x02\x01\x01"
        || !matches!(read(7, 1)?[0], 0 | 3)
        || read(8, 1)?[0] != 0
        || !matches!(u16at(16)?, 2 | 3)
        || u32at(20)? != 1
        || u16at(18)?
            != match arch {
                "x86_64" => 62,
                "aarch64" => 183,
                _ => return Err(err("ELF architecture")),
            }
        || u16at(52)? != 64
        || u16at(54)? != 56
    {
        return Err(err("unsupported runtime ELF"));
    }
    let offset = usize::try_from(u64at(32)?).map_err(err)?;
    let count = usize::from(u16at(56)?);
    if offset < 64 || count == 0 || count > 256 {
        return Err(err("ELF program headers bound"));
    }
    read(offset, count * 56)?;
    let mut loads = Vec::new();
    let mut dynamic = None;
    let mut interpreter = None;
    let string = |b: &[u8]| -> Result<String, ResourceError> {
        let end = b
            .iter()
            .position(|v| *v == 0)
            .ok_or_else(|| err("unterminated ELF string"))?;
        if end == 0 || end > 255 {
            return Err(err("ELF string bound"));
        }
        let s = std::str::from_utf8(&b[..end]).map_err(err)?;
        if s.chars().any(char::is_control) {
            return Err(err("ELF string control"));
        }
        Ok(s.to_string())
    };
    for p in (offset..offset + count * 56).step_by(56) {
        let start = usize::try_from(u64at(p + 8)?).map_err(err)?;
        let size = usize::try_from(u64at(p + 32)?).map_err(err)?;
        let segment = read(start, size)?;
        match u32at(p)? {
            1 => {
                loads.push((u64at(p + 16)?, start, size));
            }
            2 => {
                if size == 0
                    || size > 65536
                    || size % 16 != 0
                    || dynamic.replace((start, size)).is_some()
                {
                    return Err(err("ambiguous dynamic segment"));
                }
            }
            3 => {
                if size > 256
                    || interpreter.replace(string(segment)?).is_some()
                    || segment.last() != Some(&0)
                {
                    return Err(err("ambiguous interpreter"));
                }
            }
            _ => {}
        }
    }
    let (start, size) = dynamic.ok_or_else(|| err("dynamic segment missing"))?;
    let mut table = None;
    let mut length = None;
    let mut needed = Vec::new();
    let mut terminated = false;
    for p in (start..start + size).step_by(16) {
        let value = u64at(p + 8)?;
        match u64at(p)? {
            0 => {
                terminated = true;
                break;
            }
            1 => {
                if needed.len() >= 64 {
                    return Err(err("NEEDED bound"));
                }
                needed.push(value);
            }
            5 => {
                if table.replace(value).is_some() {
                    return Err(err("duplicate STRTAB"));
                }
            }
            10 => {
                if length.replace(value).is_some() {
                    return Err(err("duplicate STRSZ"));
                }
            }
            // Loader search paths, audit and alternate/filter objects are forbidden.
            15 | 29 | 0x6ffffefb | 0x6ffffefc | 0x7ffffffd | 0x7fffffff => {
                return Err(err("dynamic dependency escape"))
            }
            _ => {}
        }
    }
    if !terminated {
        return Err(err("unterminated dynamic table"));
    }
    let table = table.ok_or_else(|| err("STRTAB missing"))?;
    let length = length
        .filter(|n| *n > 0 && *n <= 16 * 1024 * 1024)
        .ok_or_else(|| err("STRSZ bound"))?;
    let mut mapped = None;
    for (address, offset, size) in loads {
        if let Some(delta) = table.checked_sub(address) {
            if delta
                .checked_add(length)
                .is_some_and(|end| end <= size as u64)
            {
                if mapped
                    .replace(read(offset + delta as usize, length as usize)?)
                    .is_some()
                {
                    return Err(err("ambiguous string mapping"));
                }
            }
        }
    }
    let strings = mapped.ok_or_else(|| err("unmapped dynamic strings"))?;
    let needed = needed
        .into_iter()
        .map(|index| {
            let index = usize::try_from(index).map_err(err)?;
            let name = string(strings.get(index..).ok_or_else(|| err("NEEDED index"))?)?;
            relative(&name)?;
            if name.contains('/') {
                return Err(err("non-basename NEEDED"));
            }
            Ok(name)
        })
        .collect::<Result<Vec<_>, ResourceError>>()?;
    let entry = u64at(24)?;
    let mut executable_entry = false;
    for p in (offset..offset + count * 56).step_by(56) {
        if u32at(p)? != 1 {
            continue;
        }
        let address = u64at(p + 16)?;
        let size = u64at(p + 32)?;
        let memory = u64at(p + 40)?;
        let alignment = u64at(p + 48)?;
        if size > memory
            || address.checked_add(memory).is_none()
            || (alignment > 1
                && (!alignment.is_power_of_two()
                    || u64at(p + 8)? % alignment != address % alignment))
        {
            return Err(err("invalid dynamic load segment"));
        }
        executable_entry |= u32at(p + 4)? & 1 != 0
            && entry >= address
            && entry
                < address
                    .checked_add(size)
                    .ok_or_else(|| err("entry overflow"))?;
    }
    Ok(LinuxDependencies {
        interpreter,
        needed,
        executable_entry,
    })
}

fn relative(path: &str) -> Result<(), ResourceError> {
    if path.is_empty()
        || path.len() > 1024
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || Path::new(path).is_absolute()
    {
        return Err(err("invalid artifact path"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod safe {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd};
    pub fn child(
        parent: &std::fs::File,
        name: &str,
        directory: bool,
        create: bool,
    ) -> Result<std::fs::File, ResourceError> {
        relative(name)?;
        if name.contains('/') {
            return Err(err("single component required"));
        }
        let name = std::ffi::CString::new(name).map_err(err)?;
        let flags = libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if directory {
                libc::O_RDONLY | libc::O_DIRECTORY
            } else if create {
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
            } else {
                libc::O_RDONLY
            };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        let meta = file.metadata()?;
        if (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
            return Err(err("artifact is not a regular file/directory"));
        }
        Ok(file)
    }
    pub fn root(path: &Path) -> Result<std::fs::File, ResourceError> {
        if !path.is_absolute() {
            return Err(err("absolute native root required"));
        }
        let mut file = std::fs::File::open("/")?;
        for part in path.components() {
            match part {
                std::path::Component::RootDir => {}
                std::path::Component::Normal(name) => {
                    file = child(
                        &file,
                        name.to_str().ok_or_else(|| err("non UTF8 root"))?,
                        true,
                        false,
                    )?
                }
                _ => return Err(err("non canonical root")),
            }
        }
        Ok(file)
    }
    pub fn open(
        root: &std::fs::File,
        path: &str,
        directory: bool,
    ) -> Result<std::fs::File, ResourceError> {
        relative(path)?;
        let parts: Vec<_> = path.split('/').collect();
        let mut current = root.try_clone()?;
        for (i, part) in parts.iter().enumerate() {
            current = child(&current, part, directory || i + 1 < parts.len(), false)?;
        }
        Ok(current)
    }
    pub fn names(dir: &std::fs::File) -> Result<Vec<String>, ResourceError> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
            if names.len() >= MAX_FILES {
                return Err(err("artifact entry bound exceeded"));
            }
            names.push(
                entry?
                    .file_name()
                    .into_string()
                    .map_err(|_| err("non UTF8 entry"))?,
            );
        }
        names.sort();
        Ok(names)
    }
    pub fn publish(
        dir: &std::fs::File,
        temporary: &str,
        expected: &str,
    ) -> Result<(), ResourceError> {
        let from = std::ffi::CString::new(temporary).map_err(err)?;
        let to = std::ffi::CString::new(expected).map_err(err)?;
        let result = unsafe {
            libc::renameat2(
                dir.as_raw_fd(),
                from.as_ptr(),
                dir.as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        dir.sync_all()?;
        Ok(())
    }
    pub fn mkdir(parent: &std::fs::File, name: &str) -> Result<std::fs::File, ResourceError> {
        let c = std::ffi::CString::new(name).map_err(err)?;
        let result = unsafe { libc::mkdirat(parent.as_raw_fd(), c.as_ptr(), 0o700) };
        if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = child(parent, name, true, false)?;
        use std::os::unix::fs::MetadataExt;
        if file.metadata()?.uid() != unsafe { libc::geteuid() }
            || (name != ".refact" && file.metadata()?.mode() & 0o077 != 0)
        {
            return Err(err("artifact storage directory is not private"));
        }
        Ok(file)
    }
}

fn read_file(mut file: std::fs::File, budget: u64) -> Result<Vec<u8>, ResourceError> {
    let before = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.nlink() != 1 {
            return Err(err("hard-linked artifact rejected"));
        }
    }
    if !before.is_file() || before.len() > budget {
        return Err(err("artifact file size/type rejected"));
    }
    let mut bytes = Vec::new();
    (&mut file).take(budget + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if bytes.len() as u64 != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(err("artifact changed while reading"));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn tree(
    dir: &std::fs::File,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut u64,
    depth: usize,
    visited: &mut usize,
) -> Result<(), ResourceError> {
    if depth > 16 {
        return Err(err("artifact depth exceeded"));
    }
    let before = safe::names(dir)?;
    for name in &before {
        *visited += 1;
        if *visited > MAX_FILES {
            return Err(err("artifact total entry bound exceeded"));
        }
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if let Ok(child) = safe::child(dir, name, true, false) {
            tree(&child, &path, files, total, depth + 1, visited)?;
        } else {
            if files.len() >= MAX_FILES {
                return Err(err("artifact file bound exceeded"));
            }
            let bytes = read_file(safe::child(dir, name, false, false)?, MAX_BYTES - *total)?;
            *total += bytes.len() as u64;
            files.insert(path, bytes);
        }
    }
    if before != safe::names(dir)? {
        return Err(err("artifact directory changed"));
    }
    Ok(())
}

fn zip_files(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, ResourceError> {
    zip_files_bounded(bytes, MAX_BYTES, MAX_FILES)
}

fn zip_files_bounded(
    bytes: &[u8],
    budget: u64,
    entry_budget: usize,
) -> Result<BTreeMap<String, Vec<u8>>, ResourceError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(err)?;
    if archive.len() > entry_budget {
        return Err(err("package entry bound exceeded"));
    }
    let mut files = BTreeMap::new();
    let mut names = std::collections::BTreeSet::new();
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(err)?;
        let name = entry.name().trim_end_matches('/').to_string();
        relative(&name)?;
        if !names.insert(name.clone())
            || entry.unix_mode().is_some_and(|m| {
                m & 0o170000 != 0 && m & 0o170000 != 0o100000 && m & 0o170000 != 0o040000
            })
        {
            return Err(err("duplicate or special package entry"));
        }
        if entry.is_dir() {
            continue;
        }
        if entry.size() > budget - total {
            return Err(err("expanded package bound exceeded"));
        }
        let mut value = Vec::new();
        (&mut entry)
            .take(budget - total + 1)
            .read_to_end(&mut value)?;
        total += value.len() as u64;
        if total > budget || value.len() as u64 != entry.size() {
            return Err(err("expanded package size changed"));
        }
        files.insert(name, value);
    }
    Ok(files)
}

// Only little-endian 64-bit executable images are supported; fat/universal,
// DLL, object and hybrid machine formats are deliberately rejected.
fn native_header(format: &ArtifactFormat, b: &[u8]) -> Result<String, ResourceError> {
    let u16at = |p: usize| -> Result<u16, ResourceError> {
        Ok(u16::from_le_bytes(
            b.get(p..p + 2)
                .ok_or_else(|| err("truncated header"))?
                .try_into()
                .map_err(err)?,
        ))
    };
    let u32at = |p: usize| -> Result<u32, ResourceError> {
        Ok(u32::from_le_bytes(
            b.get(p..p + 4)
                .ok_or_else(|| err("truncated header"))?
                .try_into()
                .map_err(err)?,
        ))
    };
    let u64at = |p: usize| -> Result<u64, ResourceError> {
        Ok(u64::from_le_bytes(
            b.get(p..p + 8)
                .ok_or_else(|| err("truncated header"))?
                .try_into()
                .map_err(err)?,
        ))
    };
    let range = |start: u64, size: u64| -> Result<(), ResourceError> {
        if start
            .checked_add(size)
            .is_none_or(|end| end > b.len() as u64)
        {
            return Err(err("image range outside artifact"));
        }
        Ok(())
    };
    let machine = if *format == ArtifactFormat::WindowsPe64 {
        if b.get(..2) != Some(b"MZ") {
            return Err(err("DOS signature missing"));
        }
        let p = u32at(60)? as usize;
        if p < 64 || b.get(p..p + 4) != Some(b"PE\0\0") {
            return Err(err("PE signature missing"));
        }
        let sections = u16at(p + 6)? as usize;
        let optional = u16at(p + 20)? as usize;
        let flags = u16at(p + 22)?;
        if sections == 0
            || sections > 96
            || optional < 112
            || optional > 240
            || flags & 2 == 0
            || flags & 0x2000 != 0
            || u16at(p + 24)? != 0x20b
        {
            return Err(err("unsupported PE executable"));
        }
        range((p + 24) as u64, optional as u64 + sections as u64 * 40)?;
        let directories = u32at(p + 24 + 108)? as usize;
        if directories > 16 || optional != 112 + directories * 8 {
            return Err(err("ambiguous PE optional header"));
        }
        let headers = u32at(p + 24 + 60)? as u64;
        range(0, headers)?;
        if headers < (p + 24 + optional + sections * 40) as u64 {
            return Err(err("PE headers overlap sections"));
        }
        let mut ranges = Vec::new();
        let mut executable = false;
        for i in 0..sections {
            let s = p + 24 + optional + i * 40;
            let size = u32at(s + 16)? as u64;
            let start = u32at(s + 20)? as u64;
            range(start, size)?;
            if size > 0 {
                if start < headers || ranges.iter().any(|&(a, z)| start < z && a < start + size) {
                    return Err(err("overlapping PE section"));
                }
                ranges.push((start, start + size));
                executable |= u32at(s + 36)? & 0x20000000 != 0;
            }
        }
        if !executable {
            return Err(err("PE executable section missing"));
        }
        match u16at(p + 4)? {
            0x8664 => "x86_64",
            0xaa64 => "aarch64",
            _ => return Err(err("unsupported PE machine")),
        }
    } else {
        if u32at(0)? != 0xfeedfacf || u32at(12)? != 2 {
            return Err(err("unsupported MachO executable"));
        }
        let machine = match (u32at(4)?, u32at(8)? & 0x00ffffff) {
            (0x01000007, 3) => "x86_64",
            (0x0100000c, 0) => "aarch64",
            _ => return Err(err("unsupported MachO CPU subtype")),
        };
        let count = u32at(16)? as usize;
        let size = u32at(20)? as usize;
        if count == 0 || count > 1024 {
            return Err(err("MachO command count rejected"));
        }
        range(32, size as u64)?;
        let mut p = 32;
        let mut executable = false;
        for _ in 0..count {
            let cmd = u32at(p)?;
            let len = u32at(p + 4)? as usize;
            if len < 8 || len % 8 != 0 || p + len > 32 + size {
                return Err(err("invalid MachO load command"));
            }
            if cmd == 0x19 {
                if len < 72 || len != 72 + u32at(p + 64)? as usize * 80 {
                    return Err(err("invalid MachO segment"));
                }
                range(u64at(p + 40)?, u64at(p + 48)?)?;
                executable |= u64at(p + 48)? > 0 && u32at(p + 60)? & 4 != 0;
            }
            p += len;
        }
        if p != 32 + size || !executable {
            return Err(err("MachO executable segment missing"));
        }
        machine
    };
    Ok(machine.into())
}

fn jetbrains_manifest(
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(String, String), ResourceError> {
    let mut descriptor = None;
    let mut expanded = files.values().map(|v| v.len() as u64).sum::<u64>();
    let mut entries = files.len();
    let mut root = None;
    for (path, bytes) in files {
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() < 2 || root.as_ref().is_some_and(|r| r != &parts[0]) {
            return Err(err("plugin ZIP must have one root"));
        }
        root = Some(parts[0]);
        if !path.ends_with(".jar") {
            continue;
        }
        if parts.len() != 3 || parts[1] != "lib" {
            return Err(err("unsupported plugin JAR layout"));
        }
        let nested = zip_files_bounded(
            bytes,
            MAX_BYTES.saturating_sub(expanded),
            MAX_FILES.saturating_sub(entries),
        )?;
        entries += nested.len();
        expanded = expanded
            .checked_add(nested.values().map(|v| v.len() as u64).sum::<u64>())
            .ok_or_else(|| err("nested size overflow"))?;
        if entries > MAX_FILES || expanded > MAX_BYTES {
            return Err(err("nested package bound exceeded"));
        }
        if let Some(xml) = nested.get("META-INF/plugin.xml") {
            if descriptor.replace(xml.clone()).is_some() {
                return Err(err("ambiguous plugin descriptors"));
            }
        }
    }
    let xml = descriptor.ok_or_else(|| err("nested plugin descriptor missing"))?;
    if xml.len() > 65536 {
        return Err(err("plugin XML bound exceeded"));
    }
    let text = std::str::from_utf8(&xml).map_err(err)?;
    let doc = roxmltree::Document::parse(text).map_err(err)?;
    let root = doc.root_element();
    if root.tag_name().name() != "idea-plugin" || root.tag_name().namespace().is_some() {
        return Err(err("plugin XML root rejected"));
    }
    let scalar = |name: &str| -> Result<String, ResourceError> {
        let nodes: Vec<_> = root.children().filter(|n| n.has_tag_name(name)).collect();
        if nodes.len() != 1 || nodes[0].children().any(|n| n.is_element()) {
            return Err(err("ambiguous plugin metadata"));
        }
        let value = nodes[0].text().unwrap_or("").trim();
        if !label(value) {
            return Err(err("plugin metadata missing"));
        }
        Ok(value.into())
    };
    if scalar("id")? != "com.smallcloud.codify" {
        return Err(err("unsupported plugin identity"));
    }
    let version = scalar("version")?;
    let hosts: Vec<_> = root
        .children()
        .filter(|n| n.has_tag_name("idea-version"))
        .collect();
    if hosts.len() != 1 {
        return Err(err("plugin host requirement ambiguous"));
    }
    let since = hosts[0]
        .attribute("since-build")
        .filter(|v| label(v))
        .ok_or_else(|| err("plugin since-build missing"))?;
    let until = hosts[0].attribute("until-build").unwrap_or("*");
    if !label(until) {
        return Err(err("invalid plugin until-build"));
    }
    Ok((version, format!("since-build={since};until-build={until}")))
}

fn inspect_bytes(
    format: &ArtifactFormat,
    bytes: Vec<u8>,
    files: BTreeMap<String, Vec<u8>>,
) -> Result<(ArtifactInspection, Vec<u8>), ResourceError> {
    let mut checked_arch = None;
    let mut package_version = None;
    let mut package_host_requirement = None;
    match format {
        ArtifactFormat::WindowsPe64 | ArtifactFormat::MacosMach64 => {
            checked_arch = Some(native_header(format, &bytes)?);
        }
        ArtifactFormat::JetbrainsZip => {
            let (version, host) = jetbrains_manifest(&files)?;
            package_version = Some(version);
            package_host_requirement = Some(host);
        }
        ArtifactFormat::LinuxElf64 => {
            if bytes.len() < 64
                || &bytes[..7] != b"\x7fELF\x02\x01\x01"
                || !matches!(bytes[7], 0 | 3)
                || bytes[8] != 0
                || bytes[20..24] != [1, 0, 0, 0]
                || !matches!(u16::from_le_bytes([bytes[16], bytes[17]]), 2 | 3)
            {
                return Err(err("unsupported ELF header"));
            }
            checked_arch = Some(
                match u16::from_le_bytes([bytes[18], bytes[19]]) {
                    62 => "x86_64",
                    183 => "aarch64",
                    _ => return Err(err("unsupported ELF machine")),
                }
                .into(),
            );
            let offset = u64::from_le_bytes(bytes[32..40].try_into().map_err(err)?);
            let count = u16::from_le_bytes([bytes[56], bytes[57]]) as u64;
            if bytes[52..56] != [64, 0, 56, 0]
                || count == 0
                || count > 256
                || offset < 64
                || offset
                    .checked_add(count * 56)
                    .is_none_or(|end| end > bytes.len() as u64)
            {
                return Err(err("invalid ELF program header table"));
            }
            let mut load = false;
            for i in 0..count {
                let p = offset as usize + i as usize * 56;
                let start = u64::from_le_bytes(bytes[p + 8..p + 16].try_into().map_err(err)?);
                let size = u64::from_le_bytes(bytes[p + 32..p + 40].try_into().map_err(err)?);
                if start
                    .checked_add(size)
                    .is_none_or(|end| end > bytes.len() as u64)
                {
                    return Err(err("ELF segment outside artifact"));
                }
                load |= bytes[p..p + 4] == [1, 0, 0, 0];
            }
            if !load {
                return Err(err("ELF load segment missing"));
            }
        }
        ArtifactFormat::GuiDist => {
            for required in [
                "chat/index.js",
                "chat/index.umd.cjs",
                "chat/style.css",
                "events/index.js",
                "events/index.umd.cjs",
            ] {
                if files.get(required).is_none_or(Vec::is_empty) {
                    return Err(err("GUI dist required output missing"));
                }
            }
            let manifest: serde_json::Value = serde_json::from_slice(
                files
                    .get("package.json")
                    .ok_or_else(|| err("GUI package metadata missing"))?,
            )
            .map_err(err)?;
            if manifest["name"] != "refact-chat-js"
                || manifest["main"] != "dist/chat/index.umd.cjs"
                || manifest["module"] != "dist/chat/index.js"
            {
                return Err(err("GUI package identity/entrypoints mismatch"));
            }
            package_version = Some(
                manifest["version"]
                    .as_str()
                    .filter(|v| label(v))
                    .ok_or_else(|| err("GUI package version missing"))?
                    .into(),
            );
        }
        ArtifactFormat::VscodeVsix => {
            if !files.contains_key("extension.vsixmanifest") {
                return Err(err("VSIX manifest missing"));
            }
            let manifest: serde_json::Value = serde_json::from_slice(
                files
                    .get("extension/package.json")
                    .ok_or_else(|| err("VSIX package manifest missing"))?,
            )
            .map_err(err)?;
            for field in ["name", "publisher", "version", "main"] {
                if manifest[field].as_str().is_none_or(|v| !label(v)) {
                    return Err(err("VSIX manifest identity missing"));
                }
            }
            if manifest["name"] != "codify" || manifest["publisher"] != "smallcloud" {
                return Err(err("unsupported VSIX package identity"));
            }
            let main = manifest["main"].as_str().unwrap();
            let main = main.strip_prefix("./").unwrap_or(main);
            relative(main)?;
            if !files.contains_key(&format!("extension/{main}")) {
                return Err(err("VSIX entrypoint missing"));
            }
            package_version = Some(manifest["version"].as_str().unwrap().into());
            package_host_requirement = Some(
                manifest["engines"]["vscode"]
                    .as_str()
                    .filter(|v| label(v))
                    .ok_or_else(|| err("VSIX host requirement missing"))?
                    .into(),
            );
            let xml = std::str::from_utf8(&files["extension.vsixmanifest"]).map_err(err)?;
            if xml.len() > 65536 || xml.contains("<!") {
                return Err(err("unsupported VSIX XML declarations"));
            }
            let property = regex::Regex::new(r#"<Property\s+([^<>]+)/>"#).map_err(err)?;
            let id = regex::Regex::new(r#"\bId="([^"]*)""#).map_err(err)?;
            let value = regex::Regex::new(r#"\bValue="([^"]*)""#).map_err(err)?;
            let mut hosts = Vec::new();
            for entry in property.captures_iter(xml) {
                let ids: Vec<_> = id.captures_iter(&entry[1]).collect();
                if ids
                    .iter()
                    .any(|id| &id[1] == "Microsoft.VisualStudio.Code.Engine")
                {
                    let values: Vec<_> = value.captures_iter(&entry[1]).collect();
                    if ids.len() != 1 || values.len() != 1 {
                        return Err(err("VSIX XML host ambiguous"));
                    }
                    hosts.push(values[0][1].to_string());
                }
            }
            if hosts.len() != 1 || Some(&hosts[0]) != package_host_requirement.as_ref() {
                return Err(err("VSIX XML/package host mismatch"));
            }
            let identity = regex::Regex::new(r#"<Identity\s+([^<>]+)/>"#).map_err(err)?;
            let matches: Vec<_> = identity.captures_iter(xml).collect();
            if matches.len() != 1 {
                return Err(err("VSIX XML identity missing or ambiguous"));
            }
            for (attribute, field) in [
                ("Id", "name"),
                ("Version", "version"),
                ("Publisher", "publisher"),
            ] {
                let pattern =
                    regex::Regex::new(&format!(r#"\b{}="([^"]*)""#, attribute)).map_err(err)?;
                let values: Vec<_> = pattern.captures_iter(&matches[0][1]).collect();
                if values.len() != 1 || &values[0][1] != manifest[field].as_str().unwrap() {
                    return Err(err("VSIX XML/package identity mismatch"));
                }
            }
        }
    }
    let manifest: BTreeMap<_, _> = files
        .iter()
        .map(|(name, bytes)| (name.clone(), hash(bytes)))
        .collect();
    let result = ArtifactInspection {
        artifact_digest: hash(&bytes),
        manifest_digest: digest(&manifest).map_err(err)?,
        bytes: bytes.len() as u64,
        files: manifest,
        checked_arch,
        package_version,
        package_host_requirement,
        compatibility_status: COMPATIBILITY_STATUS.into(),
    };
    Ok((result, bytes))
}

#[cfg(target_os = "linux")]
fn snapshot(
    root: &Path,
    draft: &ArtifactDraft,
) -> Result<(ArtifactInspection, Vec<u8>), ResourceError> {
    let root = safe::root(root)?;
    let mut files = BTreeMap::new();
    let bytes = if draft.format == ArtifactFormat::GuiDist {
        let directory = safe::open(&root, &draft.relative_path, true)?;
        tree(&directory, "", &mut files, &mut 0, 0, &mut 0)?;
        let parent = draft
            .relative_path
            .strip_suffix("/dist")
            .ok_or_else(|| err("GUI artifact must be native dist directory"))?;
        if files.contains_key("package.json") {
            return Err(err("unexpected GUI dist package manifest"));
        }
        files.insert(
            "package.json".into(),
            read_file(
                safe::open(&root, &format!("{parent}/package.json"), false)?,
                1024 * 1024,
            )?,
        );
        let mut bytes = Vec::new();
        for (path, content) in &files {
            bytes.extend_from_slice(&(path.len() as u64).to_le_bytes());
            bytes.extend_from_slice(path.as_bytes());
            bytes.extend_from_slice(&(content.len() as u64).to_le_bytes());
            bytes.extend_from_slice(content);
        }
        if bytes.len() as u64 > MAX_BYTES {
            return Err(err("snapshot bound exceeded"));
        }
        bytes
    } else {
        let bytes = read_file(safe::open(&root, &draft.relative_path, false)?, MAX_BYTES)?;
        files = if matches!(
            draft.format,
            ArtifactFormat::VscodeVsix | ArtifactFormat::JetbrainsZip
        ) {
            zip_files(&bytes)?
        } else {
            BTreeMap::from([("engine".into(), bytes.clone())])
        };
        bytes
    };
    inspect_bytes(&draft.format, bytes, files)
}
#[cfg(not(target_os = "linux"))]
fn snapshot(_: &Path, _: &ArtifactDraft) -> Result<(ArtifactInspection, Vec<u8>), ResourceError> {
    Err(err(
        "safe artifact reader currently supports Linux hosts only",
    ))
}

pub struct ArtifactAdapter {
    runtime: OutcomeRuntime,
    worktrees: WorktreeService,
    drafts: Arc<dyn DraftRegistry<ArtifactDraft>>,
}
impl ArtifactAdapter {
    pub fn new(
        runtime: OutcomeRuntime,
        worktrees: WorktreeService,
        drafts: Arc<dyn DraftRegistry<ArtifactDraft>>,
    ) -> Self {
        Self {
            runtime,
            worktrees,
            drafts,
        }
    }
    async fn bound(&self, spec: &ActionSpec) -> Result<ArtifactDraft, ResourceError> {
        if spec.project != self.runtime.project()
            || self.worktrees.source_workspace_root() != Path::new(&spec.project.canonical_root)
            || spec.adapter != ADAPTER
            || spec.kind != ACTION
            || spec.resource.resource_kind != "candidate_artifact"
            || spec.confirmation
                != refact_buddy_core::operator_types::ConfirmationClass::UserConfirmationRequired
            || spec.authority_class != refact_buddy_core::operator_types::AuthorityClass::Mutating
        {
            return Err(err("artifact action scope mismatch"));
        }
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != spec.project
            || spec.resource.resource_id != draft.candidate_id
            || spec.expected_version != digest(&draft).map_err(err)?
            || spec.resource.version != spec.expected_version
        {
            return Err(err("artifact draft changed"));
        }
        validate_attestation(&draft)?;
        let checked = draft.clone();
        self.runtime
            .with_store(move |store| {
                let candidate = super::activation_gate(
                    store,
                    &checked.candidate_id,
                    &checked.candidate_digest,
                    &checked.proof_id,
                )?;
                let target = match checked.format {
                    ArtifactFormat::LinuxElf64
                    | ArtifactFormat::WindowsPe64
                    | ArtifactFormat::MacosMach64 => DeploymentTarget::EngineBuild,
                    ArtifactFormat::GuiDist => DeploymentTarget::GuiBuild,
                    ArtifactFormat::VscodeVsix | ArtifactFormat::JetbrainsZip => {
                        DeploymentTarget::PluginPackage
                    }
                };
                if candidate.project != checked.project
                    || candidate.candidate.project != checked.project
                    || candidate.target != target
                    || candidate.candidate.worktree_ref.as_deref() != Some(&checked.worktree_id)
                    || candidate.candidate.build_identity.as_deref()
                        != Some(&checked.artifact_digest)
                {
                    return Err(OperatorStoreError::Conflict(
                        "artifact differs from evaluated candidate",
                    ));
                }
                Ok(())
            })
            .await
            .map_err(err)?;
        Ok(draft)
    }
    pub async fn inspect(&self, spec: &ActionSpec) -> Result<ArtifactInspection, ResourceError> {
        let draft = self.bound(spec).await?;
        self.read(&draft).await.map(|v| v.0)
    }
    async fn read(
        &self,
        draft: &ArtifactDraft,
    ) -> Result<(ArtifactInspection, Vec<u8>), ResourceError> {
        self.validate_source(draft).await?;
        self.worktrees
            .validate_registered_worktree_checkout(&draft.worktree_id)
            .await
            .map_err(err)?;
        let view = self
            .worktrees
            .get_worktree(&draft.worktree_id)
            .await
            .map_err(err)?;
        if view.meta.root == self.worktrees.source_workspace_root()
            || view.meta.source_workspace_root != self.worktrees.source_workspace_root()
        {
            return Err(err("isolated registered artifact worktree required"));
        }
        let root = view.meta.root;
        let exact = draft.clone();
        let (inspection, bytes) = tokio::task::spawn_blocking(move || {
            let first = snapshot(&root, &exact)?;
            let second = snapshot(&root, &exact)?;
            if first != second {
                return Err(err("artifact changed between snapshots"));
            }
            Ok::<_, ResourceError>(first)
        })
        .await
        .map_err(err)??;
        validate_inspection(&inspection, draft)?;
        if hash(&bytes) != inspection.artifact_digest {
            return Err(err("inspection blob digest mismatch"));
        }
        self.validate_source(draft).await?;
        Ok((inspection, bytes))
    }

    async fn validate_source(&self, draft: &ArtifactDraft) -> Result<(), ResourceError> {
        let checked = draft.clone();
        let candidate = self
            .runtime
            .with_store(move |store| {
                super::activation_gate(
                    store,
                    &checked.candidate_id,
                    &checked.candidate_digest,
                    &checked.proof_id,
                )
            })
            .await
            .map_err(err)?;
        let source = crate::buddy::improvement_runner::inspect_source(
            &self.worktrees,
            &draft.worktree_id,
            &candidate.target_files,
        )
        .await
        .map_err(err)?;
        if source.source_digest != candidate.source_identity
            || source.patch_digest != candidate.content_digest
        {
            return Err(err("evaluated source changed before artifact staging"));
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn stage_blob(
    project: &ProjectIdentity,
    bytes: &[u8],
    expected: &str,
) -> Result<(), ResourceError> {
    if !crate::buddy::improvement_evaluation::valid_digest(expected)
        || bytes.len() as u64 > MAX_BYTES
        || hash(bytes) != expected
    {
        return Err(err("staging bytes changed"));
    }
    let project = safe::root(Path::new(&project.canonical_root))?;
    let refact = safe::mkdir(&project, ".refact")?;
    let dir = safe::mkdir(&refact, "candidate-artifacts")?;
    let names = safe::names(&dir)?;
    let mut total = 0u64;
    for name in &names {
        let file = safe::child(&dir, name, false, false)?;
        total = total
            .checked_add(file.metadata()?.len())
            .ok_or_else(|| err("artifact quota overflow"))?;
    }
    if names.iter().any(|name| name == expected) {
        if read_file(safe::child(&dir, expected, false, false)?, MAX_BYTES)? == bytes {
            return Ok(());
        }
        return Err(err("retained artifact changed; no overwrite"));
    }
    if (!names.contains(&format!("pending-{expected}")) && names.len() >= MAX_RETAINED)
        || total
            + if names.contains(&format!("pending-{expected}")) {
                0
            } else {
                bytes.len() as u64
            }
            > 1024 * 1024 * 1024
    {
        return Err(err(
            "private artifact retention quota reached; explicit owner cleanup required",
        ));
    }
    // A crash never publishes partial bytes under the immutable digest. A
    // complete pending image can be reconciled only by this freshly granted
    // exact stage request; partial/unknown files remain for explicit cleanup.
    let temporary = format!("pending-{expected}");
    if names.contains(&temporary) {
        if read_file(safe::child(&dir, &temporary, false, false)?, MAX_BYTES)? != bytes {
            return Err(err("interrupted partial stage requires explicit cleanup"));
        }
    } else {
        let mut file = safe::child(&dir, &temporary, false, true)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    safe::publish(&dir, &temporary, expected)?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn stage_blob(_: &ProjectIdentity, _: &[u8], _: &str) -> Result<(), ResourceError> {
    Err(err("safe staging supports Linux hosts only"))
}

#[cfg(target_os = "linux")]
fn verify_blob(project: &ProjectIdentity, expected: &str) -> Result<bool, ResourceError> {
    if !crate::buddy::improvement_evaluation::valid_digest(expected) {
        return Err(err("invalid retained digest"));
    }
    let root = safe::root(Path::new(&project.canonical_root))?;
    let bytes = read_file(
        safe::open(
            &root,
            &format!(".refact/candidate-artifacts/{expected}"),
            false,
        )?,
        MAX_BYTES,
    )?;
    Ok(hash(&bytes) == expected)
}
#[cfg(not(target_os = "linux"))]
fn verify_blob(_: &ProjectIdentity, _: &str) -> Result<bool, ResourceError> {
    Err(err("safe staging supports Linux hosts only"))
}

#[async_trait::async_trait]
impl AsyncResourceAdapter for ArtifactAdapter {
    fn adapter_id(&self) -> &str {
        ADAPTER
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
        let draft = self.bound(spec).await?;
        Ok(ResourcePreview { resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(), proposed_version: draft.artifact_digest, reload_needed: false, diff: Some("Attested immutable artifact proposal only. Source bytes are not read until explicit granted inspection or staging; no installed/running identity inferred.".into()) })
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let draft = self.bound(spec).await?;
        let (inspection, bytes) = self.read(&draft).await?;
        let id = draft.artifact_digest.clone();
        let record = ArtifactRecord {
            schema_version: 1,
            project: draft.project.clone(),
            draft,
            inspection,
        };
        self.runtime
            .with_store(move |store| {
                for pin in &record.draft.retained_known_good {
                    let retained: Stored<ArtifactRecord> = store
                        .get_document(NAMESPACE, pin)?
                        .ok_or(OperatorStoreError::Conflict(
                            "known-good staged reference unavailable",
                        ))?;
                    if retained.value.draft.format != record.draft.format {
                        return Err(OperatorStoreError::Conflict("known-good target differs"));
                    }
                    if !verify_blob(&record.project, pin)
                        .map_err(|_| OperatorStoreError::Conflict("known-good bytes unavailable"))?
                    {
                        return Err(OperatorStoreError::Conflict("known-good bytes changed"));
                    }
                }
                if let Some(old) = store.get_document::<ArtifactRecord>(NAMESPACE, &id)? {
                    if old.value != record {
                        return Err(OperatorStoreError::Conflict(
                            "immutable artifact identity reused",
                        ));
                    }
                    stage_blob(&record.project, &bytes, &id)
                        .map_err(|_| OperatorStoreError::Conflict("retained artifact changed"))?;
                    return Ok(());
                }
                store.reserve_native_documents(
                    "artifact_stage",
                    &id,
                    &[(NAMESPACE, id.clone(), 1)],
                )?;
                stage_blob(&record.project, &bytes, &id).map_err(|_| {
                    OperatorStoreError::Conflict(
                        "private artifact staging failed; inspect retained bytes before retry",
                    )
                })?;
                store.put_reserved_document(
                    "artifact_stage",
                    &id,
                    NAMESPACE,
                    &id,
                    &record,
                    None,
                )?;
                Ok(())
            })
            .await
            .map_err(err)?;
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: String::new(),
            expected_post_version: self
                .drafts
                .get(spec.parameter_ref.as_deref().unwrap())?
                .artifact_digest,
            state: ResourceApplyState::Applied,
        })
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        let draft = self.bound(spec).await?;
        if draft.artifact_digest != post {
            return Ok(false);
        }
        let id = post.to_string();
        self.runtime
            .with_store(move |store| {
                let record = store.get_document::<ArtifactRecord>(NAMESPACE, &id)?;
                match record {
                    Some(record)
                        if record.value.draft == draft
                            && record.value.project == draft.project
                            && record.value.schema_version == 1
                            && validate_inspection(&record.value.inspection, &draft).is_ok() =>
                    {
                        verify_blob(&record.value.project, &id).map_err(|_| {
                            OperatorStoreError::Conflict("staged artifact unavailable or changed")
                        })
                    }
                    _ => Ok(false),
                }
            })
            .await
            .map_err(err)
    }
    async fn recover(
        &self,
        _: &ActionSpec,
        _: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        Err(err("staging has no installation/activation recovery; retained artifacts are references only"))
    }
}

/// Separate exact mutating action: only unreferenced orphan or interrupted
/// pending files. Recorded/active/known-good blobs are never retention victims.
pub const CLEANUP_ADAPTER: &str = "candidate_artifact_cleanup";
pub const CLEANUP_ACTION: &str = "remove_exact_orphan_artifacts";
const CLEANUP_NAMESPACE: &str = "improvement_artifact_cleanup";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCleanupDraft {
    pub project: ProjectIdentity,
    /// Native basename -> exact current content hash, not a glob or age policy.
    pub files: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct CleanupRecord {
    schema_version: u16,
    project: ProjectIdentity,
    draft: ArtifactCleanupDraft,
    complete: bool,
}

fn protected_artifacts(
    store: &crate::buddy::operator_store::OperatorStore,
) -> Result<std::collections::BTreeSet<String>, OperatorStoreError> {
    let mut protected = std::collections::BTreeSet::new();
    // Fail closed rather than pretending a truncated retention graph is complete.
    for namespace in [NAMESPACE, super::CANDIDATE_NAMESPACE] {
        let mut cursor = None;
        let mut count = 0;
        loop {
            let page = store.list_documents::<serde_json::Value>(namespace, cursor, 200)?;
            count += page.entries.len();
            if count > MAX_FILES {
                return Err(OperatorStoreError::Conflict(
                    "artifact reference scan bound exceeded",
                ));
            }
            for entry in page.entries {
                if namespace == NAMESPACE {
                    let record: ArtifactRecord = serde_json::from_value(entry.value)?;
                    protected.insert(record.draft.artifact_digest);
                    protected.extend(record.draft.retained_known_good);
                } else {
                    let candidate: super::CandidateRecord = serde_json::from_value(entry.value)?;
                    for id in [
                        candidate.candidate.build_identity,
                        candidate.candidate.previous_known_good_identity,
                        candidate.engine_build,
                        candidate.gui_build,
                        candidate.plugin_package,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        protected.insert(id.strip_prefix("sha256:").unwrap_or(&id).to_string());
                    }
                }
            }
            cursor = page.next_after;
            if cursor.is_none() {
                break;
            }
        }
    }
    Ok(protected)
}

#[cfg(target_os = "linux")]
fn cleanup_files(
    project: &ProjectIdentity,
    draft: &ArtifactCleanupDraft,
    protected: &std::collections::BTreeSet<String>,
    retry: bool,
    remove: bool,
) -> Result<(), ResourceError> {
    use std::os::fd::AsRawFd;
    let root = safe::root(Path::new(&project.canonical_root))?;
    let dir = safe::open(&root, ".refact/candidate-artifacts", true)?;
    use std::os::unix::fs::MetadataExt;
    let metadata = dir.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err(err("cleanup storage not private and owned"));
    }
    let names = safe::names(&dir)?;
    // Validate the entire explicit set before any effect. Fresh retries may see
    // missing files only after an exact durable cleanup claim exists.
    for (name, expected) in &draft.files {
        let target = name.strip_prefix("pending-").unwrap_or(name);
        if protected.contains(target) && (!name.starts_with("pending-") || expected == target) {
            return Err(err("active, pinned, or known-good artifact protected"));
        }
        if !names.contains(name) {
            if retry {
                continue;
            }
            return Err(err("cleanup target missing"));
        }
        let bytes = read_file(safe::child(&dir, name, false, false)?, MAX_BYTES)?;
        if hash(&bytes) != *expected {
            return Err(err("cleanup target content changed"));
        }
    }
    if remove {
        for name in draft.files.keys().filter(|name| names.contains(name)) {
            // Re-read immediately before unlink; no extraction or recursive removal.
            let bytes = read_file(safe::child(&dir, name, false, false)?, MAX_BYTES)?;
            if hash(&bytes) != draft.files[name] {
                return Err(err("cleanup target changed before unlink"));
            }
            let name = std::ffi::CString::new(name.as_str()).map_err(err)?;
            if unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        dir.sync_all()?;
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn cleanup_files(
    _: &ProjectIdentity,
    _: &ArtifactCleanupDraft,
    _: &std::collections::BTreeSet<String>,
    _: bool,
    _: bool,
) -> Result<(), ResourceError> {
    Err(err("safe artifact cleanup supports Linux hosts only"))
}

pub struct ArtifactCleanupAdapter {
    runtime: OutcomeRuntime,
    drafts: Arc<dyn DraftRegistry<ArtifactCleanupDraft>>,
}
impl ArtifactCleanupAdapter {
    pub fn new(
        runtime: OutcomeRuntime,
        drafts: Arc<dyn DraftRegistry<ArtifactCleanupDraft>>,
    ) -> Self {
        Self { runtime, drafts }
    }
    fn bound(&self, spec: &ActionSpec) -> Result<ArtifactCleanupDraft, ResourceError> {
        use crate::buddy::improvement_evaluation::valid_digest;
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if spec.project != self.runtime.project()
            || draft.project != spec.project
            || spec.adapter != CLEANUP_ADAPTER
            || spec.kind != CLEANUP_ACTION
            || spec.resource.resource_kind != CLEANUP_ADAPTER
            || spec.resource.resource_id != "orphan_artifacts"
            || spec.confirmation
                != refact_buddy_core::operator_types::ConfirmationClass::UserConfirmationRequired
            || spec.authority_class != refact_buddy_core::operator_types::AuthorityClass::Mutating
            || spec.expected_version != digest(&draft).map_err(err)?
            || spec.resource.version != spec.expected_version
            || draft.files.is_empty()
            || draft.files.len() > MAX_RETAINED
        {
            return Err(err("exact artifact cleanup scope mismatch"));
        }
        for (name, content) in &draft.files {
            if !valid_digest(name.strip_prefix("pending-").unwrap_or(name))
                || !valid_digest(content)
            {
                return Err(err("invalid cleanup file binding"));
            }
        }
        Ok(draft)
    }
}
#[async_trait::async_trait]
impl AsyncResourceAdapter for ArtifactCleanupAdapter {
    fn adapter_id(&self) -> &str {
        CLEANUP_ADAPTER
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
        self.bound(spec)?;
        Ok(ResourcePreview { resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(), proposed_version: spec.expected_version.clone(), reload_needed: false, diff: Some("Remove only explicit exact orphan/partial staging files. Re-check all candidate and known-good references at effect. No recorded artifact, activation or automatic retention deletion.".into()) })
    }
    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let draft = self.bound(spec)?;
        let id = spec.expected_version.clone();
        self.runtime
            .with_store(move |store| {
                let protected = protected_artifacts(store)?;
                let old = store.get_document::<CleanupRecord>(CLEANUP_NAMESPACE, &id)?;
                if old.as_ref().is_some_and(|v| {
                    v.value.schema_version != 1
                        || v.value.project != draft.project
                        || v.value.draft != draft
                }) {
                    return Err(OperatorStoreError::Conflict("cleanup identity conflict"));
                }
                cleanup_files(&draft.project, &draft, &protected, old.is_some(), false).map_err(
                    |_| OperatorStoreError::Conflict("exact cleanup preflight rejected"),
                )?;
                let mut record = CleanupRecord {
                    schema_version: 1,
                    project: draft.project.clone(),
                    draft,
                    complete: false,
                };
                let claim = if let Some(old) = old {
                    old
                } else {
                    store.reserve_native_documents(
                        "artifact_cleanup",
                        &id,
                        &[(CLEANUP_NAMESPACE, id.clone(), 2)],
                    )?;
                    store.put_reserved_document(
                        "artifact_cleanup",
                        &id,
                        CLEANUP_NAMESPACE,
                        &id,
                        &record,
                        None,
                    )?
                };
                cleanup_files(&record.project, &record.draft, &protected, true, true).map_err(
                    |_| {
                        OperatorStoreError::Conflict(
                            "cleanup interrupted; fresh exact authority required",
                        )
                    },
                )?;
                record.complete = true;
                store.put_reserved_document(
                    "artifact_cleanup",
                    &id,
                    CLEANUP_NAMESPACE,
                    &id,
                    &record,
                    Some(claim.revision),
                )?;
                Ok(())
            })
            .await
            .map_err(err)?;
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: String::new(),
            expected_post_version: spec.expected_version.clone(),
            state: ResourceApplyState::Applied,
        })
    }
    async fn verify(&self, spec: &ActionSpec, post: &str) -> Result<bool, ResourceError> {
        let draft = self.bound(spec)?;
        if post != spec.expected_version {
            return Ok(false);
        }
        let id = post.to_string();
        self.runtime
            .with_store(move |store| {
                let record = store.get_document::<CleanupRecord>(CLEANUP_NAMESPACE, &id)?;
                #[cfg(target_os = "linux")]
                if record.is_some_and(|v| {
                    v.value.schema_version == 1
                        && v.value.project == draft.project
                        && v.value.complete
                        && v.value.draft == draft
                }) {
                    let root = safe::root(Path::new(&draft.project.canonical_root))
                        .map_err(|_| OperatorStoreError::Conflict("cleanup root unavailable"))?;
                    let dir =
                        safe::open(&root, ".refact/candidate-artifacts", true).map_err(|_| {
                            OperatorStoreError::Conflict("cleanup directory unavailable")
                        })?;
                    let names = safe::names(&dir).map_err(|_| {
                        OperatorStoreError::Conflict("cleanup directory unavailable")
                    })?;
                    return Ok(draft.files.keys().all(|name| !names.contains(name)));
                }
                #[cfg(not(target_os = "linux"))]
                let _ = (record, draft);
                Ok(false)
            })
            .await
            .map_err(err)
    }
    async fn recover(
        &self,
        _: &ActionSpec,
        _: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        Err(err(
            "interrupted cleanup requires fresh exact action authority; no automatic replay",
        ))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    fn project(root: &Path) -> ProjectIdentity {
        ProjectIdentity {
            project_id: "artifact-fixture".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        }
    }
    fn elf() -> Vec<u8> {
        let mut bytes = vec![0; 120];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16] = 2;
        bytes[18] = 62;
        bytes[20] = 1;
        bytes[32] = 64;
        bytes[52] = 64;
        bytes[54] = 56;
        bytes[56] = 1;
        bytes[64] = 1;
        bytes[96] = 120;
        bytes
    }
    fn dynamic_elf() -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        b[16] = 3;
        b[18] = 62;
        b[20] = 1;
        b[32] = 64;
        b[52] = 64;
        b[54] = 56;
        b[56] = 3;
        let put = |b: &mut Vec<u8>, p, v: u64| b[p..p + 8].copy_from_slice(&v.to_le_bytes());
        b[64] = 1;
        b[68] = 5;
        put(&mut b, 96, 512);
        put(&mut b, 104, 512);
        b[120] = 2;
        put(&mut b, 128, 256);
        put(&mut b, 152, 80);
        let loader = b"/lib64/ld-linux-x86-64.so.2\0";
        b[176] = 3;
        put(&mut b, 184, 400);
        put(&mut b, 208, loader.len() as u64);
        b[400..400 + loader.len()].copy_from_slice(loader);
        for (p, tag, v) in [(256, 5, 352), (272, 10, 32), (288, 1, 1)] {
            put(&mut b, p, tag);
            put(&mut b, p + 8, v);
        }
        b[353..363].copy_from_slice(b"libc.so.6\0");
        b
    }
    #[test]
    fn bounded_dynamic_inventory_rejects_loader_search_escape_and_truncation() {
        let b = dynamic_elf();
        let image = linux_dependencies(&b, "x86_64").unwrap();
        assert_eq!(
            image.interpreter.as_deref(),
            Some("/lib64/ld-linux-x86-64.so.2")
        );
        assert_eq!(image.needed, ["libc.so.6"]);
        assert!(image.executable_entry);
        for tag in [15u64, 29, 0x6ffffefb, 0x6ffffefc, 0x7ffffffd, 0x7fffffff] {
            let mut bad = b.clone();
            bad[304..312].copy_from_slice(&tag.to_le_bytes());
            assert!(linux_dependencies(&bad, "x86_64").is_err());
        }
        let mut bad = b.clone();
        bad[353] = b'/';
        assert!(linux_dependencies(&bad, "x86_64").is_err());
        assert!(linux_dependencies(&b, "aarch64").is_err());
        for n in 0..b.len() {
            assert!(linux_dependencies(&b[..n], "x86_64").is_err());
        }
    }

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
    #[test]
    fn cleanup_exact_set_preflight_reference_and_private_directory_guards() {
        let root = tempfile::tempdir().unwrap();
        let project = project(root.path());
        let native = safe::root(root.path()).unwrap();
        let dir = safe::mkdir(
            &safe::mkdir(&native, ".refact").unwrap(),
            "candidate-artifacts",
        )
        .unwrap();
        let bytes = elf();
        let id = hash(&bytes);
        let path = root.path().join(".refact/candidate-artifacts");
        std::fs::write(path.join(&id), &bytes).unwrap();
        let draft = ArtifactCleanupDraft {
            project: project.clone(),
            files: BTreeMap::from([(id.clone(), id.clone())]),
        };
        let protected = std::collections::BTreeSet::from([id.clone()]);
        assert!(cleanup_files(&project, &draft, &protected, false, true).is_err());
        assert!(path.join(&id).exists());
        let pending = format!("pending-{id}");
        std::fs::write(path.join(&pending), &bytes).unwrap();
        let complete = ArtifactCleanupDraft {
            project: project.clone(),
            files: BTreeMap::from([(pending.clone(), id.clone())]),
        };
        assert!(cleanup_files(&project, &complete, &protected, false, true).is_err());
        // Partial bytes are not the protected artifact: exact explicit cleanup may
        // remove only that damaged image, never the published or complete image.
        std::fs::write(path.join(&pending), b"partial").unwrap();
        let partial = ArtifactCleanupDraft {
            project: project.clone(),
            files: BTreeMap::from([(pending.clone(), hash(b"partial"))]),
        };
        cleanup_files(&project, &partial, &protected, false, true).unwrap();
        assert!(path.join(&id).exists());
        assert!(cleanup_files(&project, &partial, &protected, false, true).is_err());
        cleanup_files(&project, &partial, &protected, true, true).unwrap();
        let mut mismatched = draft.clone();
        mismatched.files.insert("f".repeat(64), hash(b"expected"));
        std::fs::write(path.join("f".repeat(64)), b"changed").unwrap();
        assert!(cleanup_files(&project, &mismatched, &Default::default(), false, true).is_err());
        assert!(path.join(&id).exists()); // whole set preflight precedes any unlink
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(cleanup_files(&project, &draft, &Default::default(), false, true).is_err());
        drop(dir);
    }

    #[test]
    fn interrupted_staging_requires_exact_complete_bytes_and_never_publishes_partial() {
        let root = tempfile::tempdir().unwrap();
        let project = project(root.path());
        let bytes = elf();
        let id = hash(&bytes);
        let native = safe::root(root.path()).unwrap();
        let dir = safe::mkdir(
            &safe::mkdir(&native, ".refact").unwrap(),
            "candidate-artifacts",
        )
        .unwrap();
        let pending = format!("pending-{id}");
        let mut file = safe::child(&dir, &pending, false, true).unwrap();
        file.write_all(&bytes[..10]).unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert!(stage_blob(&project, &bytes, &id).is_err());
        assert!(!root
            .path()
            .join(".refact/candidate-artifacts")
            .join(&id)
            .exists());
        // Simulate the crash after fsync but before atomic publish, not a cleanup policy.
        std::fs::write(
            root.path()
                .join(".refact/candidate-artifacts")
                .join(&pending),
            &bytes,
        )
        .unwrap();
        stage_blob(&project, &bytes, &id).unwrap();
        assert!(verify_blob(&project, &id).unwrap());
        assert!(!safe::names(&dir).unwrap().contains(&pending));
    }
    #[test]
    fn scratch_file_mutation_symlink_and_size_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("engine"), elf()).unwrap();
        let root = safe::root(dir.path()).unwrap();
        let first = read_file(safe::open(&root, "engine", false).unwrap(), MAX_BYTES).unwrap();
        assert!(read_file(safe::open(&root, "engine", false).unwrap(), 1).is_err());
        std::fs::write(dir.path().join("engine"), b"changed").unwrap();
        assert_ne!(
            hash(&first),
            hash(&read_file(safe::open(&root, "engine", false).unwrap(), MAX_BYTES).unwrap())
        );
        std::os::unix::fs::symlink("engine", dir.path().join("link")).unwrap();
        assert!(safe::open(&root, "link", false).is_err());
        std::os::unix::fs::symlink(dir.path(), dir.path().join("parent")).unwrap();
        assert!(safe::open(&root, "parent/engine", false).is_err());
        assert!(safe::open(&root, "../engine", false).is_err());
    }

    #[test]
    fn vsix_manifest_mismatch_and_archive_traversal_rejected() {
        let mut files = BTreeMap::from([
            ("extension.vsixmanifest".into(), br#"<Identity Id="codify" Version="1" Publisher="smallcloud"/><Property Id="Microsoft.VisualStudio.Code.Engine" Value="^1.90.0"/>"#.to_vec()),
            ("extension/package.json".into(), br#"{"name":"codify","publisher":"smallcloud","version":"1","main":"out/main.js","engines":{"vscode":"^1.90.0"}}"#.to_vec()),
            ("extension/out/main.js".into(), b"built".to_vec()),
        ]);
        assert!(inspect_bytes(&ArtifactFormat::VscodeVsix, vec![], files.clone()).is_ok());
        for (field, replacement) in [("name", "other"), ("publisher", "other"), ("version", "2")] {
            let mut changed = files.clone();
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&changed["extension/package.json"]).unwrap();
            manifest[field] = serde_json::json!(replacement);
            changed.insert(
                "extension/package.json".into(),
                serde_json::to_vec(&manifest).unwrap(),
            );
            assert!(
                inspect_bytes(&ArtifactFormat::VscodeVsix, vec![], changed).is_err(),
                "{field}"
            );
        }
        let mut changed = files.clone();
        changed.insert("extension.vsixmanifest".into(), br#"<Identity Id="codify" Version="1" Publisher="smallcloud"/><Property Id="Microsoft.VisualStudio.Code.Engine" Value="^1.80.0"/>"#.to_vec());
        assert!(inspect_bytes(&ArtifactFormat::VscodeVsix, vec![], changed).is_err());
        files.insert(
            "extension.vsixmanifest".into(),
            br#"<Identity Id="fixture" Version="2" Publisher="publisher"/>"#.to_vec(),
        );
        assert!(inspect_bytes(&ArtifactFormat::VscodeVsix, vec![], files).is_err());
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file("../escape", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"bad").unwrap();
        assert!(zip_files(&writer.finish().unwrap().into_inner()).is_err());
    }

    #[test]
    fn attestation_digest_arch_os_manifest_and_retention_mismatches() {
        let bytes = elf();
        let inspection = inspect_bytes(
            &ArtifactFormat::LinuxElf64,
            bytes.clone(),
            BTreeMap::from([("engine".into(), bytes)]),
        )
        .unwrap()
        .0;
        let draft = ArtifactDraft {
            project: project(Path::new("/fixture")),
            candidate_id: "candidate".into(),
            candidate_digest: "a".repeat(64),
            proof_id: "proof".into(),
            worktree_id: "worktree".into(),
            relative_path: "target/release/engine".into(),
            format: ArtifactFormat::LinuxElf64,
            artifact_digest: inspection.artifact_digest.clone(),
            manifest_digest: inspection.manifest_digest.clone(),
            attested_version: "1.0.0".into(),
            attested_os: "linux".into(),
            attested_arch: "x86_64".into(),
            attested_host_requirement: "operator attests Linux host".into(),
            retained_known_good: vec![],
        };
        validate_inspection(&inspection, &draft).unwrap();
        for field in [
            "artifact_digest",
            "manifest_digest",
            "attested_arch",
            "attested_os",
            "worktree_id",
            "relative_path",
        ] {
            let mut value = serde_json::to_value(&draft).unwrap();
            value[field] = serde_json::json!(match field {
                "artifact_digest" | "manifest_digest" => "b".repeat(64),
                "attested_arch" => "aarch64".into(),
                "attested_os" => "windows".into(),
                "relative_path" => "../escape".into(),
                _ => String::new(),
            });
            let changed: ArtifactDraft = serde_json::from_value(value).unwrap();
            assert!(
                validate_inspection(&inspection, &changed).is_err(),
                "{field}"
            );
        }
        let mut changed = draft.clone();
        changed.retained_known_good = vec!["b".repeat(64); 2];
        assert!(validate_attestation(&changed).is_err());
        let mut changed = inspection.clone();
        changed.files.insert("engine".into(), "c".repeat(64));
        assert!(validate_inspection(&changed, &draft).is_err());
        for (offset, value) in [(7, 9), (18, 3), (20, 0), (54, 0)] {
            let mut bytes = elf();
            bytes[offset] = value;
            assert!(inspect_bytes(&ArtifactFormat::LinuxElf64, bytes, BTreeMap::new()).is_err());
        }
        assert!(serde_json::from_str::<ArtifactFormat>("\"macho\"").is_err());
    }

    #[test]
    fn gui_snapshot_reads_native_manifest_and_detects_package_and_blob_changes() {
        let root = tempfile::tempdir().unwrap();
        let mut draft = ArtifactDraft {
            project: project(root.path()),
            candidate_id: "candidate".into(),
            candidate_digest: "a".repeat(64),
            proof_id: "proof".into(),
            worktree_id: "worktree".into(),
            relative_path: "gui/dist".into(),
            format: ArtifactFormat::GuiDist,
            artifact_digest: "b".repeat(64),
            manifest_digest: "c".repeat(64),
            attested_version: String::new(),
            attested_os: "linux".into(),
            attested_arch: "x86_64".into(),
            attested_host_requirement: "operator-attested browser".into(),
            retained_known_good: vec![],
        };
        for name in [
            "chat/index.js",
            "chat/index.umd.cjs",
            "chat/style.css",
            "events/index.js",
            "events/index.umd.cjs",
        ] {
            let path = root.path().join("gui/dist").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"built fixture output").unwrap();
        }
        // A dist is not a package manifest: the actual adjacent package.json is mandatory.
        assert!(snapshot(root.path(), &draft).is_err());
        std::fs::write(
            root.path().join("gui/package.json"),
            include_bytes!("../../../gui/package.json"),
        )
        .unwrap();
        let (inspection, bytes) = snapshot(root.path(), &draft).unwrap();
        draft.artifact_digest = inspection.artifact_digest.clone();
        draft.manifest_digest = inspection.manifest_digest.clone();
        draft.attested_version = inspection.package_version.clone().unwrap();
        validate_inspection(&inspection, &draft).unwrap();
        stage_blob(&draft.project, &bytes, &draft.artifact_digest).unwrap();
        assert!(verify_blob(&draft.project, &draft.artifact_digest).unwrap());
        draft.attested_version.push_str("-different");
        assert!(validate_inspection(&inspection, &draft).is_err());
        draft.attested_version = inspection.package_version.unwrap();
        std::fs::write(root.path().join("gui/dist/chat/index.js"), b"changed").unwrap();
        let changed = snapshot(root.path(), &draft).unwrap().0;
        assert!(validate_inspection(&changed, &draft).is_err());
        std::fs::write(root.path().join("gui/dist/package.json"), b"{}").unwrap();
        assert!(snapshot(root.path(), &draft).is_err());
    }
    #[test]
    fn exact_private_staging_duplicate_mutation_and_recovery_pin_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let bytes = elf();
        let id = hash(&bytes);
        stage_blob(&project, &bytes, &id).unwrap();
        stage_blob(&project, &bytes, &id).unwrap();
        assert!(verify_blob(&project, &id).unwrap());
        assert!(!dir.path().join("installed").exists());
        for i in 1..MAX_RETAINED {
            let value = vec![i as u8];
            stage_blob(&project, &value, &hash(&value)).unwrap();
        }
        assert!(stage_blob(&project, b"over quota", &hash(b"over quota")).is_err());
        std::fs::write(
            dir.path().join(".refact/candidate-artifacts").join(&id),
            b"corrupt",
        )
        .unwrap();
        assert!(!verify_blob(&project, &id).unwrap());
        assert!(stage_blob(&project, &bytes, &id).is_err());
    }
}
