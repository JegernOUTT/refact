use crate::*;
use rusqlite::Connection;
use std::collections::HashSet;

pub fn block_anchors(block: &RichBlock) -> Vec<&Anchor> {
    match &block.content {
        RichBlockContent::Code {
            anchor,
        } => vec![anchor],
        RichBlockContent::Html {
            anchors, ..
        }
        | RichBlockContent::Diff {
            anchors, ..
        } => anchors.iter().collect(),
        _ => vec![],
    }
}

pub fn block_evidence_ids(block: &RichBlock) -> &[String] {
    match &block.content {
        RichBlockContent::Sources {
            evidence_ids, ..
        }
        | RichBlockContent::Evidence {
            evidence_ids,
        } => evidence_ids,
        _ => &[],
    }
}

pub fn tour_version_candidate_id(version: &TourVersion) -> Result<Option<&str>> {
    if let Some(id) = &version.candidate_id {
        return Ok(Some(id));
    }
    let mut ids = version
        .anchors
        .iter()
        .map(|a| a.position.candidate_id.as_str());
    let first = ids.next();
    if ids.any(|id| Some(id) != first) {
        return Err(invalid("ambiguous legacy tour candidate"));
    }
    Ok(first)
}

pub fn resolve_tour_target<'a>(
    review: &'a Review,
    target: &TourDiscussionTarget,
) -> Result<(&'a TourVersion, Option<&'a RichBlock>)> {
    if target.block_id.is_some() != target.block_revision.is_some() {
        return Err(invalid("block ID and revision must be paired"));
    }
    let step = review
        .tour
        .iter()
        .find(|s| s.id == target.step_id)
        .ok_or_else(|| invalid("unknown target topic"))?;
    let version = std::iter::once(&step.current)
        .chain(step.history.iter())
        .find(|v| v.version == target.version)
        .ok_or_else(|| invalid("unknown target version"))?;
    if tour_version_candidate_id(version)? != Some(target.candidate_id.as_str()) {
        return Err(invalid("target candidate mismatch"));
    }
    let block = match &target.block_id {
        Some(id) => Some(
            version
                .blocks
                .iter()
                .find(|b| &b.id == id && Some(b.revision) == target.block_revision)
                .ok_or_else(|| invalid("unknown target block revision"))?,
        ),
        None => None,
    };
    Ok((version, block))
}

pub(crate) fn normalize_revisions(next: &mut [RichBlock], previous: &[&RichBlock]) -> Result<()> {
    for block in next {
        let old_anchors: Vec<Anchor> = previous
            .iter()
            .filter(|old| old.id == block.id)
            .max_by_key(|old| old.revision)
            .into_iter()
            .flat_map(|old| block_anchors(old))
            .cloned()
            .collect();
        match &mut block.content {
            RichBlockContent::Code {
                anchor,
            } => crate::store::preserve_anchors(std::slice::from_mut(anchor), &old_anchors),
            RichBlockContent::Html {
                anchors, ..
            }
            | RichBlockContent::Diff {
                anchors, ..
            } => crate::store::preserve_anchors(anchors, &old_anchors),
            _ => {}
        }
        block.revision = if let Some(old) = previous
            .iter()
            .filter(|old| old.id == block.id)
            .max_by_key(|old| old.revision)
        {
            if old.content == block.content {
                old.revision
            } else {
                old.revision
                    .checked_add(1)
                    .ok_or_else(|| invalid("block revision overflow"))?
            }
        } else {
            1
        };
    }
    Ok(())
}

pub(crate) fn validate_blocks(
    c: &Connection,
    review: &Review,
    blocks: &[RichBlock],
    candidate_id: Option<&str>,
) -> Result<()> {
    let mut ids = HashSet::new();
    for block in blocks {
        if block.id.is_empty() || !ids.insert(&block.id) || block.revision == 0 {
            return Err(invalid("invalid block identity/revision"));
        }
        if let RichBlockContent::Html {
            static_fallback: Some(fallback),
            ..
        } = &block.content
        {
            if fallback.trim().is_empty() || fallback.len() > 64 * 1024 {
                return Err(invalid("invalid HTML static fallback"));
            }
        }
        for anchor in block_anchors(block) {
            if Some(anchor.position.candidate_id.as_str()) != candidate_id {
                return Err(invalid("block anchor candidate mismatch"));
            }
            crate::reconcile::validate_anchor(c, review, anchor)?;
        }
        if let RichBlockContent::Diff {
            candidate_id: id,
            path,
            anchors,
        } = &block.content
        {
            if Some(id.as_str()) != candidate_id {
                return Err(invalid("diff candidate mismatch"));
            }
            validate_path(review, id, path)?;
            if anchors.iter().any(|a| a.position.path != *path) {
                return Err(invalid("diff anchor path mismatch"));
            }
        }
        for id in block_evidence_ids(block) {
            let e = review
                .evidence
                .iter()
                .find(|e| &e.id == id)
                .ok_or_else(|| invalid("unknown block evidence"))?;
            if Some(e.candidate_id.as_str()) != candidate_id {
                return Err(invalid("block evidence candidate mismatch"));
            }
            if matches!(block.content, RichBlockContent::Sources { .. })
                && matches!(e.source, EvidenceSource::Executed { .. })
            {
                return Err(invalid("executed evidence is not a source"));
            }
            if matches!(block.content, RichBlockContent::Evidence { .. })
                && !matches!(e.source, EvidenceSource::Executed { .. })
            {
                return Err(invalid("evidence block requires executed evidence"));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_path(review: &Review, candidate_id: &str, path: &str) -> Result<()> {
    relative(path)?;
    if !review
        .candidates
        .iter()
        .any(|c| c.id == candidate_id && c.files.iter().any(|f| f.path == path))
    {
        return Err(invalid("path absent from candidate"));
    }
    Ok(())
}

pub(crate) fn validate_version(
    c: &Connection,
    review: &Review,
    version: &TourVersion,
) -> Result<()> {
    if version
        .html
        .as_ref()
        .is_some_and(|html| html.trim().is_empty() || html.len() > 1024 * 1024)
    {
        return Err(invalid("invalid tour HTML page"));
    }
    if (!version.blocks.is_empty() || version.html.is_some()) && version.candidate_id.is_none() {
        return Err(invalid("rich tour requires explicit candidate"));
    }
    let candidate_id = if version.candidate_id.is_some() {
        tour_version_candidate_id(version)?
    } else {
        None
    };
    if let Some(id) = candidate_id {
        if !review.candidates.iter().any(|c| c.id == id) {
            return Err(invalid("unknown tour candidate"));
        }
    }
    for anchor in &version.anchors {
        if candidate_id.is_some() && Some(anchor.position.candidate_id.as_str()) != candidate_id {
            return Err(invalid("tour anchor candidate mismatch"));
        }
        crate::reconcile::validate_anchor(c, review, anchor)?;
    }
    validate_blocks(c, review, &version.blocks, candidate_id)
}
