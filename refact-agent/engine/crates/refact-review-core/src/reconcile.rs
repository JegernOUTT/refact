use crate::*;
use rusqlite::Connection;
fn text(c: &Connection, candidate: &Candidate, path: &str, side: &Side) -> Result<String> {
    let f = candidate
        .files
        .iter()
        .find(|x| x.path == path)
        .ok_or(Error::NotFound)?;
    let b = match side {
        Side::Base => &f.base,
        Side::Head => &f.head,
    }
    .as_ref()
    .ok_or(Error::NotFound)?;
    String::from_utf8(crate::store::blob(c, b)?).map_err(|_| Error::NonText)
}
fn range_hash(lines: &[&str], start: usize, end: usize) -> String {
    content_hash(lines[start..end].join("\n").as_bytes())
}
fn position(
    candidate: &Candidate,
    path: &str,
    side: Side,
    start: u32,
    end: u32,
    txt: &str,
) -> Result<AnchorPosition> {
    let lines: Vec<_> = txt.lines().collect();
    if start == 0 || end < start || end as usize > lines.len() {
        return Err(invalid("anchor line range out of bounds"));
    }
    let s = start as usize - 1;
    let e = end as usize;
    Ok(AnchorPosition {
        candidate_id: candidate.id.clone(),
        path: path.to_owned(),
        side,
        start_line: start,
        end_line: end,
        content_hash: range_hash(&lines, s, e),
        context_before: lines[s.saturating_sub(3)..s]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        context_after: lines[e..(e + 3).min(lines.len())]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    })
}
impl ReviewStore {
    pub fn anchor(
        &self,
        candidate: &Candidate,
        path: &str,
        side: Side,
        start: u32,
        end: u32,
    ) -> Result<Anchor> {
        relative(path)?;
        let c = self
            .connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?;
        let txt = text(&c, candidate, path, &side)?;
        Ok(Anchor {
            position: position(candidate, path, side, start, end, &txt)?,
            status: AnchorStatus::Current,
            stalled_reason: None,
            history: vec![],
        })
    }
}
pub(crate) fn validate_anchor(c: &Connection, r: &Review, anchor: &Anchor) -> Result<()> {
    let p = &anchor.position;
    relative(&p.path)?;
    let candidate = r
        .candidates
        .iter()
        .find(|x| x.id == p.candidate_id)
        .ok_or_else(|| invalid("unknown anchor candidate"))?;
    let txt = text(c, candidate, &p.path, &p.side)?;
    if position(
        candidate,
        &p.path,
        p.side.clone(),
        p.start_line,
        p.end_line,
        &txt,
    )? != *p
    {
        return Err(invalid("anchor does not match immutable snapshot"));
    }
    for old in &anchor.history {
        let candidate = r
            .candidates
            .iter()
            .find(|x| x.id == old.candidate_id)
            .ok_or_else(|| invalid("unknown anchor history candidate"))?;
        let txt = text(c, candidate, &old.path, &old.side)?;
        if position(
            candidate,
            &old.path,
            old.side.clone(),
            old.start_line,
            old.end_line,
            &txt,
        )? != *old
        {
            return Err(invalid("invalid anchor history"));
        }
    }
    Ok(())
}
fn reconcile(
    c: &Connection,
    anchor: &mut Anchor,
    target: &Candidate,
    candidates: &[Candidate],
) -> Result<()> {
    if anchor.position.candidate_id == target.id && anchor.status == AnchorStatus::Current {
        return Ok(());
    }
    let p = &anchor.position;
    let mut target_path = p.path.clone();
    if !target.files.iter().any(|f| f.path == p.path) {
        if let Some(old) = candidates
            .iter()
            .find(|candidate| candidate.id == p.candidate_id)
            .and_then(|candidate| candidate.files.iter().find(|file| file.path == p.path))
        {
            let old_side = match p.side {
                Side::Base => (&old.base, old.base_mode),
                Side::Head => (&old.head, old.head_mode),
            };
            let matches: Vec<_> = target
                .files
                .iter()
                .filter(|file| {
                    let side = match p.side {
                        Side::Base => (&file.base, file.base_mode),
                        Side::Head => (&file.head, file.head_mode),
                    };
                    let existed_before = candidates
                        .iter()
                        .find(|candidate| candidate.id == p.candidate_id)
                        .is_some_and(|candidate| {
                            candidate
                                .files
                                .iter()
                                .any(|old_file| old_file.path == file.path)
                        });
                    !existed_before && old_side.0.is_some() && old_side == side
                })
                .collect();
            if matches.len() == 1 {
                target_path = matches[0].path.clone();
            }
        }
    }
    let txt = match text(c, target, &target_path, &p.side) {
        Ok(t) => t,
        Err(Error::NotFound) | Err(Error::NonText) => {
            anchor.status = AnchorStatus::Stalled;
            anchor.stalled_reason = Some("path/side unavailable or non-text".into());
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let lines: Vec<_> = txt.lines().collect();
    let len = (p.end_line - p.start_line + 1) as usize;
    let mut matches = vec![];
    if len <= lines.len() {
        for s in 0..=lines.len() - len {
            let e = s + len;
            if range_hash(&lines, s, e) != p.content_hash {
                continue;
            }
            let before = &p.context_before;
            let after = &p.context_after;
            if s < before.len() || e + after.len() > lines.len() {
                continue;
            }
            if lines[s - before.len()..s]
                .iter()
                .copied()
                .eq(before.iter().map(String::as_str))
                && lines[e..e + after.len()]
                    .iter()
                    .copied()
                    .eq(after.iter().map(String::as_str))
            {
                matches.push((s, e));
            }
        }
    }
    if matches.len() != 1 {
        anchor.status = AnchorStatus::Stalled;
        anchor.stalled_reason = Some(
            if matches.is_empty() {
                "no exact contextual match"
            } else {
                "ambiguous exact contextual matches"
            }
            .into(),
        );
        return Ok(());
    }
    let (s, e) = matches[0];
    let next = position(
        target,
        &target_path,
        p.side.clone(),
        s as u32 + 1,
        e as u32,
        &txt,
    )?;
    anchor.history.push(anchor.position.clone());
    anchor.position = next;
    anchor.status = AnchorStatus::Current;
    anchor.stalled_reason = None;
    Ok(())
}
pub(crate) fn activate(c: &Connection, r: &mut Review, target: &Candidate) -> Result<()> {
    if r.current_candidate_id.as_ref() == Some(&target.id) {
        return Ok(());
    }
    let candidates = r.candidates.clone();
    for step in &mut r.tour {
        if step.current.candidate_id.is_some() || !step.current.blocks.is_empty() {
            continue;
        }
        let previous = step.current.clone();
        for a in &mut step.current.anchors {
            reconcile(c, a, target, &candidates)?;
        }
        if step.current != previous {
            step.history.push(previous);
            step.current.version += 1;
        }
    }
    for annotation in &mut r.annotations {
        for a in &mut annotation.anchors {
            reconcile(c, a, target, &candidates)?;
        }
        if annotation.applicability == Applicability::Current {
            annotation.applicability = Applicability::NeedsRevalidation;
        }
    }
    for thread in &mut r.threads {
        if thread.target.is_some() {
            continue;
        }
        for a in &mut thread.anchors {
            reconcile(c, a, target, &candidates)?;
        }
    }
    for evidence in &mut r.evidence {
        if matches!(
            evidence.applicability,
            Applicability::Unknown | Applicability::NoLongerApplicable
        ) {
            continue;
        }
        evidence.applicability = if evidence.candidate_id == target.id {
            if matches!(
                &evidence.source,
                EvidenceSource::Source { anchor: Some(anchor), .. }
                    if anchor.status != AnchorStatus::Current
            ) {
                Applicability::NeedsRevalidation
            } else {
                Applicability::Current
            }
        } else {
            Applicability::Historical
        };
    }
    Ok(())
}
