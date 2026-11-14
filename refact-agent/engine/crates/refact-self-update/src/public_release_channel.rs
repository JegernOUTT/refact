//! Private bounded public metadata helpers; no staging, credentials or host expansion.
use super::*;
use crate::public_qualification::{
    verify_public_qualification, QualificationRef, MAX_INVENTORY_BYTES, MAX_QUALIFICATION_BYTES,
    QUALIFICATION_NAME,
};
use std::collections::HashSet;

pub(super) fn inventory_url(url: &str) -> bool {
    url.strip_prefix(&format!(
        "{BASE}/contents/tools/dev/ci_test_inventory.json?ref="
    ))
    .is_some_and(|sha| crate::runtime_manifest::digest(sha, 40))
}

#[derive(Deserialize)]
struct AnnotatedTag {
    sha: String,
    object: GitObject,
}
pub(super) async fn tag_commit(t: &impl PublicTransport, mut object: GitObject) -> Result<String> {
    let mut seen = HashSet::new();
    let mut depth = 0;
    loop {
        check(
            crate::runtime_manifest::digest(&object.sha, 40),
            "invalid tag object SHA",
        )?;
        check(seen.insert(object.sha.clone()), "annotated tag cycle")?;
        if object.kind == "commit" {
            return Ok(object.sha);
        }
        check(object.kind == "tag", "tag object is not commit or tag")?;
        check(depth < 4, "annotated tag depth exceeded")?;
        let tag: AnnotatedTag = api(t, &format!("git/tags/{}", object.sha)).await?;
        check(
            tag.sha == object.sha,
            "annotated tag object identity mismatch",
        )?;
        object = tag.object;
        depth += 1;
    }
}

pub(super) async fn qualify(
    t: &impl PublicTransport,
    release: &Release,
    manifest: &RuntimeManifest,
    trusted: &TrustedSource,
    source: &SourceEvidence,
    policy: &TrustedQualificationPolicy,
) -> Result<VerifiedPublicQualification> {
    let reference = QualificationRef::parse(
        manifest
            .qualification_ref()
            .ok_or_else(|| reject("qualification reference required"))?,
    )
    .map_err(reject)?;
    check(
        reference.repository_id == REPO_ID
            && reference.run_id == manifest.run().id
            && reference.run_attempt == manifest.run().attempt,
        "qualification artifact reference identity mismatch",
    )?;
    let report_asset = asset(release, QUALIFICATION_NAME)?;
    check(
        report_asset.size <= MAX_QUALIFICATION_BYTES as u64,
        "qualification byte limit exceeded",
    )?;
    check(
        report_asset.digest.as_deref() == Some(format!("sha256:{}", reference.sha256).as_str()),
        "qualification release asset digest mismatch",
    )?;
    let report = fetch(
        t,
        &format!("{BASE}/releases/assets/{}", report_asset.id),
        true,
        MAX_QUALIFICATION_BYTES,
    )
    .await?;
    check(
        report.len() as u64 == report_asset.size,
        "qualification asset size mismatch",
    )?;
    check_asset_digest(report_asset, &report)?;
    let url = format!(
        "{BASE}/contents/tools/dev/ci_test_inventory.json?ref={}",
        manifest.source_sha()
    );
    check(inventory_url(&url), "invalid source inventory URL")?;
    // API mode: redirects are forbidden. Only this exact fixed API path gets raw media type.
    let inventory = fetch(t, &url, false, MAX_INVENTORY_BYTES).await?;
    verify_public_qualification(manifest, trusted, source, policy, &report, &inventory)
        .map_err(reject)
}

#[derive(Deserialize)]
struct Publication {
    id: u64,
    tag_name: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
}
// GitHub's canonical UTC second precision is fixed-width and therefore lexically sortable.
// Reject malformed/impossible dates instead of treating arbitrary JSON strings as chronology.
fn timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
        || b.iter()
            .enumerate()
            .any(|(i, c)| !matches!(i, 4 | 7 | 10 | 13 | 16 | 19) && !c.is_ascii_digit())
    {
        return false;
    }
    let n = |a: usize, z: usize| s[a..z].parse::<u32>().unwrap_or(0);
    let year = n(0, 4);
    let month = n(5, 7);
    let day = n(8, 10);
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return false,
    };
    year > 0 && day > 0 && day <= days && n(11, 13) < 24 && n(14, 16) < 60 && n(17, 19) < 60
}

/// Select newest observed publication, not highest semver. Three complete pages maximum,
/// ten candidate resolutions maximum. This is a bounded fetched snapshot, not an atomic API view.
pub(super) async fn resolve(
    t: &impl PublicTransport,
    selection: &RuntimeSelection,
    target: &str,
    protocols: (u32, u32, u32),
    policy: &TrustedQualificationPolicy,
) -> Result<ResolvedPublicRelease> {
    let wanted = match selection.spec() {
        SelectionSpec::Channel {
            channel, ..
        } => *channel,
        _ => return Err(PublicReleaseError::InvalidSource),
    };
    let repo: Repo = api(t, "").await?;
    check(repo.valid(), "repository identity mismatch")?;
    let mut publications = Vec::new();
    let mut ids = HashSet::new();
    let mut tags = HashSet::new();
    let mut complete = false;
    for page in 1..=3 {
        let rows: Vec<Publication> = api(t, &format!("releases?per_page=100&page={page}")).await?;
        check(rows.len() <= 100, "release page count exceeded")?;
        let terminal = rows.len() < 100;
        for row in rows {
            if row.id == 0 || !ids.insert(row.id) || !tags.insert(row.tag_name.clone()) {
                return Err(PublicReleaseError::ChannelResolutionIncomplete);
            }
            if row.draft
                || !row.tag_name.starts_with("engine/v")
                || row.prerelease != (wanted != Channel::Stable)
            {
                continue;
            }
            check(
                row.published_at.as_deref().is_some_and(timestamp),
                "invalid release publication timestamp",
            )?;
            publications.push(row);
        }
        if terminal {
            complete = true;
            break;
        }
    }
    if !complete {
        return Err(PublicReleaseError::ChannelResolutionIncomplete);
    }
    publications.sort_by(|a, b| {
        b.published_at
            .cmp(&a.published_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    for (index, publication) in publications.into_iter().enumerate() {
        if index >= 10 {
            return Err(PublicReleaseError::ChannelResolutionIncomplete);
        }
        let pinned = RuntimeSelection::try_from(SelectionSpec::Pinned {
            repository: repository(),
            source: PinnedSource::Release {
                tag: publication.tag_name.clone(),
            },
            expected_sha: None,
            manifest_digest: None,
        })
        .map_err(reject)?;
        // Never fallback after malformed, missing or unqualified evidence. An opposite-channel
        // Preview/Nightly manifest may be skipped only after full existing validation succeeds.
        let mut resolved = resolve_exact(t, &pinned, target, protocols, policy).await?;
        check(
            resolved.release_id == publication.id,
            "release listing identity changed",
        )?;
        if resolved.manifest.build().channel != wanted {
            continue;
        }
        let trusted = TrustedSource {
            repository: repository(),
            workflow: resolved.source.workflow.clone(),
        };
        validate_selection_binding(
            selection,
            &resolved.manifest,
            &trusted,
            &resolved.source,
            Some(&resolved.release),
        )
        .map_err(reject)?;
        resolved.selection = selection.clone();
        return Ok(resolved);
    }
    Err(PublicReleaseError::ChannelReleaseNotFound)
}
