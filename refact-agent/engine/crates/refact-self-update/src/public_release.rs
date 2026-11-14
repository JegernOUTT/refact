//! Bounded public release resolution, without staging or execution.
//! Selected-SHA workflow dispatches remain unsupported; channel scans fail closed when incomplete.
use crate::runtime_artifact::{
    verify_runtime_artifact, ArtifactLimits, VerifiedRuntimeArtifact, MAX_ARCHIVE_BYTES,
    MAX_RUNTIME_BYTES,
};
use crate::runtime_manifest::{
    validate_qualification, validate_source_evidence, Channel, Component, Qualification,
    Repository, Run, RuntimeManifest, SourceEvidence, TestResult, TrustedSource, Workflow,
    MAX_MANIFEST_BYTES,
};
use crate::runtime_selection::{
    validate_selection_binding, PinnedSource, ResolvedReleaseEvidence, RuntimeSelection,
    SelectionSpec,
};
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::{future::Future, pin::Pin, time::Duration};
use crate::public_qualification::{TrustedQualificationPolicy, VerifiedPublicQualification};
#[path = "public_release_channel.rs"]
mod channel;
use channel::inventory_url;

const BASE: &str = "https://api.github.com/repos/JegernOUTT/refact";
const REPO_ID: u64 = 1254216370;
const API_LIMIT: usize = 1024 * 1024;
const MANIFEST_NAME: &str = "runtime-manifest.json";

#[cfg(test)]
#[path = "public_release_http_tests.rs"]
mod http_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicReleaseError {
    InvalidSource,
    AuthenticationRequired,
    ChannelResolutionUnavailable,
    ChannelResolutionIncomplete,
    ChannelReleaseNotFound,
    QualificationUnavailable,
    AnnotatedTagUnsupported,
    SelectedShaDispatchUnsupported,
    Rejected(String),
}
pub type Result<T> = std::result::Result<T, PublicReleaseError>;
fn reject(message: impl Into<String>) -> PublicReleaseError {
    PublicReleaseError::Rejected(message.into())
}
fn check(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(reject(message))
    }
}
fn repository() -> Repository {
    Repository {
        owner: "JegernOUTT".into(),
        name: "refact".into(),
        id: REPO_ID,
    }
}
fn segment(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Only HTTPS github.com URLs in the fixed repository. No filesystem paths.
/// `channel` applies only to repository and /releases inputs.
/// Exact pinned release/run inputs carry no channel constraint; the manifest supplies its channel.
pub fn parse_source_url(input: &str, channel: Channel) -> Result<RuntimeSelection> {
    let invalid = || PublicReleaseError::InvalidSource;
    if input.chars().any(|c| c.is_control() || c.is_whitespace()) || input.contains('\\') {
        return Err(invalid());
    }
    let url = Url::parse(input).map_err(|_| invalid())?;
    // Comparing the raw authority also rejects explicit default ports and URL normalization tricks.
    if !input.starts_with("https://github.com/")
        || url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let raw_path = input
        .strip_prefix("https://github.com/")
        .ok_or_else(invalid)?;
    let parts: Vec<String> = raw_path
        .split('/')
        .map(|p| {
            percent_decode_str(p)
                .decode_utf8()
                .map(|p| p.into_owned())
                .map_err(|_| invalid())
        })
        .collect::<Result<_>>()?;
    if parts.iter().any(|p| {
        p.is_empty()
            || p == "."
            || p == ".."
            || p.contains(['/', '\\', ':'])
            || p.chars().any(char::is_control)
    }) || parts.len() < 2
        || parts[0] != "JegernOUTT"
        || parts[1] != "refact"
    {
        return Err(invalid());
    }
    let spec = match parts.get(2).map(String::as_str) {
        None => SelectionSpec::Channel {
            repository: repository(),
            channel,
            workflow: None,
        },
        Some("releases") if parts.len() == 3 => SelectionSpec::Channel {
            repository: repository(),
            channel,
            workflow: None,
        },
        Some("releases") if parts.len() >= 5 && parts[3] == "tag" => SelectionSpec::Pinned {
            repository: repository(),
            source: PinnedSource::Release {
                tag: parts[4..].join("/"),
            },
            expected_sha: None,
            manifest_digest: None,
        },
        Some("actions") if (parts.len() == 5 || parts.len() == 7) && parts[3] == "runs" => {
            let id = parts[4].parse::<u64>().map_err(|_| invalid())?;
            let attempt = if parts.len() == 7 && parts[5] == "attempts" {
                parts[6].parse::<u64>().map_err(|_| invalid())?
            } else if parts.len() == 5 {
                1
            } else {
                return Err(invalid());
            };
            SelectionSpec::Pinned {
                repository: repository(),
                source: PinnedSource::Run {
                    id,
                    attempt,
                },
                expected_sha: None,
                manifest_digest: None,
            }
        }
        _ => return Err(invalid()),
    };
    RuntimeSelection::try_from(spec).map_err(|_| invalid())
}

/// One unauthenticated read. Implementations are trusted I/O infrastructure: they must not
/// add credentials, launch tools, or fabricate responses. Injection is not self-authorization.
/// Implementations must bound allocation while reading; the resolver also checks returned lengths.
#[derive(Debug)]
pub struct PublicResponse {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}
pub type ReadFuture<'a> = Pin<Box<dyn Future<Output = Result<PublicResponse>> + Send + 'a>>;
pub trait PublicTransport: Sync {
    fn get<'a>(&'a self, url: &'a str, asset: bool, max_bytes: usize) -> ReadFuture<'a>;
}
/// Fresh client: no caller headers, cookie jar, authorization, proxy environment or redirect policy.
pub struct ReqwestPublicTransport {
    client: reqwest::Client,
}
impl ReqwestPublicTransport {
    fn builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .user_agent("refact-public-runtime-resolver")
    }

    pub fn new() -> Result<Self> {
        Self::from_builder(Self::builder())
    }

    fn from_builder(builder: reqwest::ClientBuilder) -> Result<Self> {
        Ok(Self {
            client: builder
                .build()
                .map_err(|e| reject(e.without_url().to_string()))?,
        })
    }

    #[cfg(test)]
    fn loopback_test(addr: std::net::SocketAddr, timeout: Duration) -> Self {
        Self::from_builder(
            Self::builder()
                .tls_certs_only([reqwest::Certificate::from_der(include_bytes!(
                    "../tests/fixtures/public-release-http/test-ca.der"
                ))
                .unwrap()])
                .resolve("api.github.com", addr)
                .resolve("release-assets.githubusercontent.com", addr)
                .http1_only()
                .timeout(timeout),
        )
        .unwrap()
    }
}
impl PublicTransport for ReqwestPublicTransport {
    fn get<'a>(&'a self, url: &'a str, asset: bool, max_bytes: usize) -> ReadFuture<'a> {
        Box::pin(async move {
            // Defense in depth even when this adapter is called outside the resolver.
            check(
                max_bytes > 0 && max_bytes <= MAX_ARCHIVE_BYTES as usize,
                "invalid HTTP bound",
            )?;
            let parsed = Url::parse(url).map_err(|_| PublicReleaseError::InvalidSource)?;
            let authority = url
                .strip_prefix("https://")
                .and_then(|s| s.split('/').next())
                .unwrap_or("");
            check(
                url.starts_with("https://")
                    && safe_http_url(&parsed, asset)
                    && !authority.contains([':', '@']),
                "HTTP origin rejected",
            )?;
            let mut response = self
                .client
                .get(parsed)
                .header(
                    reqwest::header::ACCEPT,
                    if asset {
                        "application/octet-stream"
                    } else if inventory_url(url) {
                        "application/vnd.github.raw+json"
                    } else {
                        "application/vnd.github+json"
                    },
                )
                .send()
                .await
                .map_err(|e| reject(e.without_url().to_string()))?;
            let status = response.status().as_u16();
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .map(|s| s.to_str().map(str::to_owned))
                .transpose()
                .map_err(|_| reject("invalid redirect"))?;
            check(
                response
                    .content_length()
                    .is_none_or(|n| n <= max_bytes as u64),
                "HTTP body limit exceeded",
            )?;
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| reject(e.without_url().to_string()))?
            {
                check(
                    chunk.len() <= max_bytes.saturating_sub(body.len()),
                    "HTTP body limit exceeded",
                )?;
                body.extend_from_slice(&chunk);
            }
            Ok(PublicResponse {
                status,
                location,
                body,
            })
        })
    }
}
fn safe_http_url(url: &Url, asset: bool) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
        && (url.host_str() == Some("api.github.com")
            || (asset
                && matches!(
                    url.host_str(),
                    Some(
                        "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                            | "github-releases.githubusercontent.com"
                    )
                )))
}
async fn fetch(t: &impl PublicTransport, url: &str, asset: bool, cap: usize) -> Result<Vec<u8>> {
    let mut current = url.to_owned();
    for redirects in 0..=3 {
        let response = t.get(&current, asset, cap).await?;
        check(response.body.len() <= cap, "HTTP body limit exceeded")?;
        if response.status == 200 {
            return Ok(response.body);
        }
        if asset && matches!(response.status, 301 | 302 | 303 | 307 | 308) && redirects < 3 {
            let location = response
                .location
                .ok_or_else(|| reject("redirect missing location"))?;
            let next = Url::parse(&location).map_err(|_| reject("invalid redirect"))?;
            // Only absolute storage URLs; reject explicit ports even if the URL parser normalizes 443.
            let authority = location
                .strip_prefix("https://")
                .and_then(|s| s.split('/').next())
                .unwrap_or("");
            check(
                location.starts_with("https://")
                    && safe_http_url(&next, true)
                    && next.host_str() != Some("api.github.com")
                    && !authority.contains([':', '@']),
                "asset redirect origin rejected",
            )?;
            current = location;
        } else {
            return Err(reject(format!("public HTTP status {}", response.status)));
        }
    }
    Err(reject("asset redirect limit exceeded"))
}
async fn api<T: DeserializeOwned>(t: &impl PublicTransport, path: &str) -> Result<T> {
    let url = if path.is_empty() {
        BASE.to_owned()
    } else {
        format!("{BASE}/{path}")
    };
    let bytes = fetch(t, &url, false, API_LIMIT).await?;
    serde_json::from_slice(&bytes).map_err(|e| reject(e.to_string()))
}
#[derive(Deserialize)]
struct Repo {
    id: u64,
    full_name: String,
}
impl Repo {
    fn valid(&self) -> bool {
        self.id == REPO_ID && self.full_name == "JegernOUTT/refact"
    }
}
#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
    size: u64,
    state: String,
    digest: Option<String>,
}
#[derive(Deserialize)]
struct Release {
    id: u64,
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct GitObject {
    sha: String,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
struct GitRef {
    #[serde(rename = "ref")]
    name: String,
    object: GitObject,
}
#[derive(Deserialize)]
struct ApiWorkflow {
    id: u64,
    path: String,
    state: String,
}
#[derive(Deserialize)]
struct ApiRun {
    id: u64,
    run_attempt: u64,
    workflow_id: u64,
    path: String,
    head_sha: String,
    status: String,
    event: String,
    conclusion: Option<String>,
    repository: Repo,
    head_repository: Option<Repo>,
}

/// Validated metadata only; not installed or running evidence. Fields cannot be replaced by callers.
#[derive(Debug)]
pub struct ResolvedPublicRelease {
    selection: RuntimeSelection,
    manifest: RuntimeManifest,
    source: SourceEvidence,
    release: ResolvedReleaseEvidence,
    qualification: Qualification,
    target: String,
    protocols: (u32, u32, u32),
    asset_id: u64,
    release_id: u64,
    public_qualification: Option<VerifiedPublicQualification>,
}
impl ResolvedPublicRelease {
    pub fn manifest(&self) -> &RuntimeManifest {
        &self.manifest
    }
    pub fn source_evidence(&self) -> &SourceEvidence {
        &self.source
    }
    pub fn release_evidence(&self) -> &ResolvedReleaseEvidence {
        &self.release
    }
    pub fn qualification(&self) -> Qualification {
        self.qualification
    }
    pub fn public_qualification(&self) -> Option<&VerifiedPublicQualification> {
        self.public_qualification.as_ref()
    }
    pub fn selection(&self) -> &RuntimeSelection {
        &self.selection
    }
}
#[derive(Debug)]
pub struct VerifiedPublicCandidate {
    resolved: ResolvedPublicRelease,
    artifact: VerifiedRuntimeArtifact,
}
impl VerifiedPublicCandidate {
    pub fn resolved(&self) -> &ResolvedPublicRelease {
        &self.resolved
    }
    pub fn artifact(&self) -> &VerifiedRuntimeArtifact {
        &self.artifact
    }
}
fn asset<'a>(release: &'a Release, name: &str) -> Result<&'a Asset> {
    let mut matches = release.assets.iter().filter(|a| a.name == name);
    let a = matches
        .next()
        .ok_or_else(|| reject("release asset absent"))?;
    check(
        matches.next().is_none() && a.id > 0 && a.size > 0 && a.state == "uploaded",
        "release asset rejected",
    )?;
    Ok(a)
}
fn trusted_path(channel: Channel, path: &str) -> bool {
    match channel {
        Channel::Stable => matches!(
            path,
            ".github/workflows/release.yml"
                | ".github/workflows/plugin_release.yml"
                | ".github/workflows/agent_engine_release.yml"
        ),
        Channel::Preview | Channel::Nightly => matches!(
            path,
            ".github/workflows/prerelease.yml" | ".github/workflows/plugin_build.yml"
        ),
    }
}
/// Uses coverage policy compiled from this client's trusted inventory, never a downloaded policy.
pub async fn resolve_public_release(
    t: &impl PublicTransport,
    selection: &RuntimeSelection,
    target: &str,
    protocols: (u32, u32, u32),
) -> Result<ResolvedPublicRelease> {
    let policy = TrustedQualificationPolicy::from_inventory_bytes(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../tools/dev/ci_test_inventory.json"
    )))
    .map_err(reject)?;
    resolve_public_release_with_policy(t, selection, target, protocols, &policy).await
}

/// The policy must be independently trusted by the caller, not derived from release/source bytes.
pub async fn resolve_public_release_with_policy(
    t: &impl PublicTransport,
    selection: &RuntimeSelection,
    target: &str,
    protocols: (u32, u32, u32),
    policy: &TrustedQualificationPolicy,
) -> Result<ResolvedPublicRelease> {
    if let SelectionSpec::Channel {
        repository: repo, ..
    } = selection.spec()
    {
        check(*repo == repository(), "foreign repository")?;
        check(
            crate::runtime_manifest::native_target(target),
            "unsupported target",
        )?;
        return channel::resolve(t, selection, target, protocols, policy).await;
    }
    resolve_exact(t, selection, target, protocols, policy).await
}

async fn resolve_exact(
    t: &impl PublicTransport,
    selection: &RuntimeSelection,
    target: &str,
    protocols: (u32, u32, u32),
    policy: &TrustedQualificationPolicy,
) -> Result<ResolvedPublicRelease> {
    let tag = match selection.spec() {
        SelectionSpec::Local {
            ..
        } => return Err(PublicReleaseError::InvalidSource),
        SelectionSpec::Channel {
            repository: repo, ..
        } => {
            check(*repo == repository(), "foreign repository")?;
            return Err(PublicReleaseError::ChannelResolutionUnavailable);
        }
        SelectionSpec::Pinned {
            repository: repo,
            source,
            ..
        } => {
            check(*repo == repository(), "foreign repository")?;
            match source {
                PinnedSource::Run {
                    ..
                } => return Err(PublicReleaseError::AuthenticationRequired),
                PinnedSource::Release {
                    tag,
                } => tag.clone(),
            }
        }
    };
    check(
        crate::runtime_manifest::native_target(target),
        "unsupported target",
    )?;
    let repo: Repo = api(t, "").await?;
    check(repo.valid(), "repository identity mismatch")?;
    let release: Release = api(t, &format!("releases/tags/{}", segment(&tag))).await?;
    check(
        release.id > 0 && release.tag_name == tag && !release.draft,
        "release not exact published tag",
    )?;
    let reference: GitRef = api(t, &format!("git/ref/tags/{}", segment(&tag))).await?;
    check(
        reference.name == format!("refs/tags/{tag}"),
        "tag reference mismatch",
    )?;
    let source_sha = channel::tag_commit(t, reference.object).await?;
    let ma = asset(&release, MANIFEST_NAME)?;
    check(
        ma.size <= MAX_MANIFEST_BYTES as u64,
        "manifest byte limit exceeded",
    )?;
    let bytes = fetch(
        t,
        &format!("{BASE}/releases/assets/{}", ma.id),
        true,
        MAX_MANIFEST_BYTES,
    )
    .await?;
    check(
        bytes.len() as u64 == ma.size,
        "manifest asset size mismatch",
    )?;
    check_asset_digest(ma, &bytes)?;
    let manifest = RuntimeManifest::from_json(&bytes).map_err(reject)?;
    check(
        tag == format!("engine/v{}", manifest.build().version),
        "release tag/build version mismatch",
    )?;
    check(source_sha == manifest.source_sha(), "tag source mismatch")?;
    let channel = manifest.build().channel;
    check(
        release.prerelease == (channel != Channel::Stable),
        "release channel mismatch",
    )?;
    check(
        trusted_path(channel, &manifest.workflow().path),
        "workflow path not approved",
    )?;
    // Resolve workflow ID from an independently allowlisted path, never trust the manifest ID.
    let filename = manifest
        .workflow()
        .path
        .rsplit('/')
        .next()
        .ok_or_else(|| reject("workflow path"))?;
    let workflow: ApiWorkflow = api(t, &format!("actions/workflows/{}", segment(filename))).await?;
    check(
        workflow.id > 0 && workflow.path == manifest.workflow().path && workflow.state == "active",
        "workflow API identity mismatch",
    )?;
    let trusted = TrustedSource {
        repository: repository(),
        workflow: Workflow {
            id: workflow.id,
            path: workflow.path,
        },
    };
    let run: ApiRun = api(
        t,
        &format!(
            "actions/runs/{}/attempts/{}",
            manifest.run().id,
            manifest.run().attempt
        ),
    )
    .await?;
    check(
        matches!(
            run.event.as_str(),
            "workflow_dispatch" | "schedule" | "push"
        ),
        "unapproved run event",
    )?;
    check(
        run.repository.valid() && run.head_repository.as_ref().is_some_and(Repo::valid),
        "fork or repository mismatch",
    )?;
    if run.head_sha != manifest.source_sha() {
        return Err(PublicReleaseError::SelectedShaDispatchUnsupported);
    }
    let source = SourceEvidence {
        repository: repository(),
        workflow: Workflow {
            id: run.workflow_id,
            path: run.path,
        },
        run: Run {
            id: run.id,
            attempt: run.run_attempt,
        },
        source_sha: run.head_sha,
        status: run.status,
        conclusion: run.conclusion.unwrap_or_default(),
    };
    validate_source_evidence(&manifest, &trusted, &source).map_err(reject)?;
    let evidence = ResolvedReleaseEvidence {
        repository_id: REPO_ID,
        tag,
        source_sha: manifest.source_sha().into(),
        manifest_digest: manifest.digest(),
    };
    validate_selection_binding(selection, &manifest, &trusted, &source, Some(&evidence))
        .map_err(reject)?;
    manifest
        .check_protocols(protocols.0, protocols.1, protocols.2)
        .map_err(reject)?;
    let public_qualification = if channel == Channel::Stable {
        Some(channel::qualify(t, &release, &manifest, &trusted, &source, policy).await?)
    } else {
        check(
            manifest.tests() == TestResult::NotRun && manifest.qualification_ref().is_none(),
            "public qualification unsupported",
        )?;
        None
    };
    let qualification = validate_qualification(
        &manifest,
        channel,
        &trusted,
        public_qualification.as_ref().map(|q| q.evidence()),
    )
    .map_err(reject)?;
    let runtime = manifest
        .artifacts()
        .iter()
        .find(|a| a.component == Component::Runtime && a.target == target)
        .ok_or_else(|| reject("runtime target absent"))?;
    let archive = asset(&release, &runtime.filename)?;
    check(
        archive.size == runtime.size
            && archive.digest.as_deref() == Some(format!("sha256:{}", runtime.sha256).as_str()),
        "archive asset binding mismatch",
    )?;
    Ok(ResolvedPublicRelease {
        selection: selection.clone(),
        manifest,
        source,
        release: evidence,
        qualification,
        target: target.into(),
        protocols,
        asset_id: archive.id,
        release_id: release.id,
        public_qualification,
    })
}
fn check_asset_digest(asset: &Asset, bytes: &[u8]) -> Result<()> {
    use sha2::{Digest, Sha256};
    check(
        asset.digest.as_deref()
            == Some(format!("sha256:{}", hex::encode(Sha256::digest(bytes))).as_str()),
        "asset digest mismatch",
    )
}
/// Consume the privately constructed resolution; bind the current selection, target and protocols
/// again before archive I/O. Caller budgets are validated before reading, not just by extraction.
pub async fn fetch_verified_candidate(
    t: &impl PublicTransport,
    resolved: ResolvedPublicRelease,
    current: &RuntimeSelection,
    target: &str,
    protocols: (u32, u32, u32),
    limits: ArtifactLimits,
) -> Result<VerifiedPublicCandidate> {
    check(
        *current == resolved.selection
            && target == resolved.target
            && protocols == resolved.protocols,
        "candidate context changed",
    )?;
    check(
        limits.archive_bytes > 0
            && limits.archive_bytes <= MAX_ARCHIVE_BYTES
            && limits.runtime_bytes > 0
            && limits.runtime_bytes <= MAX_RUNTIME_BYTES,
        "invalid artifact limits",
    )?;
    let a = resolved
        .manifest
        .artifacts()
        .iter()
        .find(|a| a.component == Component::Runtime && a.target == target)
        .ok_or_else(|| reject("runtime target absent"))?;
    check(
        a.size <= limits.archive_bytes,
        "archive byte limit exceeded",
    )?;
    let bytes = fetch(
        t,
        &format!("{BASE}/releases/assets/{}", resolved.asset_id),
        true,
        a.size as usize,
    )
    .await?;
    let artifact =
        verify_runtime_artifact(&resolved.manifest, target, &bytes, limits).map_err(reject)?;
    Ok(VerifiedPublicCandidate {
        resolved,
        artifact,
    })
}
