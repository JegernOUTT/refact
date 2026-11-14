use crate::runtime_manifest::{
    decode, digest, require, text, validate_source_evidence, Channel, ContractResult, Repository,
    Run, RuntimeManifest, SourceEvidence, TrustedSource, Workflow,
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

pub const MAX_SELECTION_BYTES: usize = 16 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedReleaseEvidence {
    pub repository_id: u64,
    pub tag: String,
    pub source_sha: String,
    pub manifest_digest: String,
}
pub fn validate_selection_binding(
    selection: &RuntimeSelection,
    manifest: &RuntimeManifest,
    trusted: &TrustedSource,
    source: &SourceEvidence,
    release: Option<&ResolvedReleaseEvidence>,
) -> ContractResult<()> {
    selection.require_managed_overwrite()?;
    validate_source_evidence(manifest, trusted, source)?;
    match selection.spec() {
        SelectionSpec::Channel {
            repository,
            channel,
            workflow,
        } => require(
            repository == manifest.repository()
                && *channel == manifest.build().channel
                && workflow.as_ref().is_none_or(|w| w == manifest.workflow()),
            "channel selection mismatch",
        ),
        SelectionSpec::Pinned {
            repository,
            source,
            expected_sha,
            manifest_digest,
        } => {
            require(
                repository == manifest.repository()
                    && expected_sha
                        .as_ref()
                        .is_none_or(|s| s == manifest.source_sha())
                    && manifest_digest
                        .as_ref()
                        .is_none_or(|d| *d == manifest.digest()),
                "pinned selection identity mismatch",
            )?;
            match source {
                PinnedSource::Run {
                    id,
                    attempt,
                } => require(
                    *id == manifest.run().id && *attempt == manifest.run().attempt,
                    "pinned run mismatch",
                ),
                PinnedSource::Release {
                    tag,
                } => {
                    let evidence = release.ok_or("resolved release evidence required")?;
                    require(
                        evidence.repository_id == repository.id
                            && evidence.tag == *tag
                            && evidence.source_sha == manifest.source_sha()
                            && evidence.manifest_digest == manifest.digest(),
                        "pinned release mismatch",
                    )
                }
            }
        }
        SelectionSpec::Local {
            ..
        } => Err("local runtime cannot resolve as managed".into()),
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PinnedSource {
    Release { tag: String },
    Run { id: u64, attempt: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SelectionSpec {
    Channel {
        repository: Repository,
        channel: Channel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow: Option<Workflow>,
    },
    Pinned {
        repository: Repository,
        source: PinnedSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_sha: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        manifest_digest: Option<String>,
    },
    Local {
        absolute_path: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SelectionSpec", into = "SelectionSpec")]
pub struct RuntimeSelection(SelectionSpec);
impl From<RuntimeSelection> for SelectionSpec {
    fn from(s: RuntimeSelection) -> Self {
        s.0
    }
}
impl TryFrom<SelectionSpec> for RuntimeSelection {
    type Error = String;
    fn try_from(s: SelectionSpec) -> ContractResult<Self> {
        require(
            serde_json::to_vec(&s).map_err(|e| e.to_string())?.len() <= MAX_SELECTION_BYTES,
            "selection byte limit exceeded",
        )?;
        match &s {
            SelectionSpec::Channel {
                repository,
                workflow,
                ..
            } => {
                repository.validate()?;
                if let Some(w) = workflow {
                    w.validate()?;
                }
            }
            SelectionSpec::Pinned {
                repository,
                source,
                expected_sha,
                manifest_digest,
            } => {
                repository.validate()?;
                match source {
                    PinnedSource::Release {
                        tag,
                    } => require(
                        text(tag, 200)
                            && !tag.contains(['\\', ':'])
                            && tag
                                .split('/')
                                .all(|p| !p.is_empty() && p != "." && p != ".."),
                        "invalid release tag",
                    )?,
                    PinnedSource::Run {
                        id,
                        attempt,
                    } => Run {
                        id: *id,
                        attempt: *attempt,
                    }
                    .validate()?,
                }
                require(
                    expected_sha.as_ref().is_none_or(|s| digest(s, 40))
                        && manifest_digest.as_ref().is_none_or(|s| digest(s, 64)),
                    "invalid selection digest",
                )?;
            }
            SelectionSpec::Local {
                absolute_path,
            } => require(
                valid_absolute_path(absolute_path),
                "local path must be absolute and normalized",
            )?,
        }
        Ok(Self(s))
    }
}
pub(crate) fn valid_absolute_path(s: &str) -> bool {
    text(s, 4096)
        && Path::new(s).is_absolute()
        && !s
            .split(if cfg!(windows) {
                &['/', '\\'][..]
            } else {
                &['/'][..]
            })
            .any(|p| p == "." || p == "..")
        && !Path::new(s)
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
}
pub fn selection_store_path(host_config_directory: &Path) -> ContractResult<std::path::PathBuf> {
    require(
        host_config_directory.is_absolute(),
        "absolute host config directory required",
    )?;
    Ok(host_config_directory.join("runtime-selection.json"))
}
impl RuntimeSelection {
    pub fn from_json(bytes: &[u8]) -> ContractResult<Self> {
        decode(bytes, MAX_SELECTION_BYTES)
    }
    pub fn spec(&self) -> &SelectionSpec {
        &self.0
    }
    pub fn is_managed(&self) -> bool {
        !matches!(self.0, SelectionSpec::Local { .. })
    }
    pub fn require_managed_overwrite(&self) -> ContractResult<()> {
        require(
            self.is_managed(),
            "local runtime is unmanaged; overwrite forbidden",
        )
    }
}
