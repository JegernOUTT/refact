//! Tour pages tag native inserts directly in their HTML:
//! `<refact-code path lines>`, `<refact-diff path [lines]>` and `<refact-evidence ref>`.
//! The server resolves each tag to an anchored block and leaves a `<refact-slot>` in its place.

use std::{collections::HashSet, sync::OnceLock};

use refact_review_core::*;
use regex::Regex;

use super::{anchor_in, parse_lines, resolve_path, valid_id, Outcome};

pub const MAX_PAGE_INSERTS: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub enum InsertKind {
    Code,
    Diff,
    Evidence,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Insert {
    pub id: String,
    pub kind: InsertKind,
    pub path: Option<String>,
    pub lines: Option<(u32, u32)>,
    pub side: Side,
    pub reference: Option<String>,
}

fn pattern(cell: &'static OnceLock<Regex>, source: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(source).expect("static pattern"))
}

fn tag_pattern() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    pattern(
        &CELL,
        r#"(?is)<refact-(code|diff|evidence)\b([^>]*?)/?>(?:\s*</refact-(?:code|diff|evidence)\s*>)?"#,
    )
}

fn attribute_pattern() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    pattern(
        &CELL,
        r#"(?is)([a-z][a-z0-9_-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')"#,
    )
}

fn slot_pattern() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    pattern(
        &CELL,
        r#"(?is)<refact-slot\b[^>]*?\bdata-block\s*=\s*(?:"([^"]*)"|'([^']*)')"#,
    )
}

fn unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Slot ids already present in the page, in document order.
pub fn slot_ids(html: &str) -> Vec<String> {
    slot_pattern()
        .captures_iter(html)
        .filter_map(|caps| caps.get(1).or(caps.get(2)))
        .map(|found| unescape(found.as_str()))
        .collect()
}

fn parse_tag(
    kind: &str,
    attributes: &str,
    used: &mut HashSet<String>,
    counter: &mut usize,
) -> Outcome<(Insert, String)> {
    let name = kind.to_ascii_lowercase();
    let tag = format!("<refact-{name}>");
    let (mut path, mut lines, mut side, mut id) = (None, None, Side::Head, None);
    let (mut reference, mut highlight, mut title) = (None, None, None);
    for caps in attribute_pattern().captures_iter(attributes) {
        let value = unescape(
            caps.get(2)
                .or(caps.get(3))
                .map_or("", |found| found.as_str()),
        );
        match caps[1].to_ascii_lowercase().as_str() {
            "path" | "file" => path = Some(value),
            "lines" | "line" => {
                lines = Some(parse_lines(&value).map_err(|error| format!("{tag}: {error}"))?)
            }
            "side" => {
                side = match value.as_str() {
                    "base" | "old" | "left" => Side::Base,
                    "head" | "new" | "right" => Side::Head,
                    _ => return Err(format!("{tag}: side must be `head` or `base`")),
                }
            }
            "id" => id = Some(value),
            "ref" | "evidence" => reference = Some(value),
            "highlight" => {
                for part in value.split(',') {
                    parse_lines(part).map_err(|error| format!("{tag} highlight: {error}"))?;
                }
                highlight = Some(value);
            }
            "title" | "caption" => title = Some(value),
            other => {
                return Err(format!(
                    "{tag}: unknown attribute `{other}`; use path, lines, side, id, highlight, title (ref on refact-evidence)"
                ))
            }
        }
    }
    let kind = match name.as_str() {
        "code" => InsertKind::Code,
        "diff" => InsertKind::Diff,
        _ => InsertKind::Evidence,
    };
    match kind {
        InsertKind::Code if path.is_none() || lines.is_none() => {
            return Err(format!(
                "{tag} needs `path` and `lines`, e.g. lines=\"12-40\""
            ))
        }
        InsertKind::Diff if path.is_none() => return Err(format!("{tag} needs `path`")),
        InsertKind::Evidence if reference.is_none() => {
            return Err(format!("{tag} needs `ref` with an evidence or source id"))
        }
        _ => {}
    }
    let id = match id {
        Some(id) if !valid_id(&id) => {
            return Err(format!(
                "{tag}: id may use letters, digits, `-`, `_`, `.` (at most 64)"
            ))
        }
        Some(id) if !used.insert(id.clone()) => {
            return Err(format!("{tag}: id `{id}` is used twice on this page"))
        }
        Some(id) => id,
        None => loop {
            *counter += 1;
            let generated = format!("{name}-{counter}");
            if used.insert(generated.clone()) {
                break generated;
            }
        },
    };
    let mut extra = String::new();
    if let Some(highlight) = highlight {
        extra.push_str(&format!(" data-highlight=\"{}\"", escape(&highlight)));
    }
    if let Some(title) = title {
        extra.push_str(&format!(" data-title=\"{}\"", escape(&title)));
    }
    Ok((
        Insert {
            id,
            kind,
            path,
            lines,
            side,
            reference,
        },
        extra,
    ))
}

/// Replaces every insert tag with a slot and returns the inserts in document order.
pub fn rewrite(html: &str, taken: &HashSet<String>) -> Outcome<(String, Vec<Insert>)> {
    let mut inserts = Vec::new();
    let mut used = taken.clone();
    let mut counter = 0;
    let mut failure = None;
    let rewritten = tag_pattern()
        .replace_all(html, |caps: &regex::Captures| {
            if failure.is_some() {
                return caps[0].to_string();
            }
            match parse_tag(&caps[1], &caps[2], &mut used, &mut counter) {
                Ok((insert, extra)) => {
                    let slot = format!(
                        "<refact-slot data-block=\"{}\"{extra}></refact-slot>",
                        escape(&insert.id)
                    );
                    inserts.push(insert);
                    slot
                }
                Err(error) => {
                    failure = Some(error);
                    caps[0].to_string()
                }
            }
        })
        .into_owned();
    match failure {
        Some(error) => Err(error),
        None => Ok((rewritten, inserts)),
    }
}

/// Snapshot paths an insert quotes, for provenance.
pub fn quoted_paths(review: &Review, candidate: &Candidate, inserts: &[Insert]) -> Vec<String> {
    inserts
        .iter()
        .filter_map(|insert| insert.path.as_deref())
        .filter_map(|path| resolve_path(review, candidate, path).ok())
        .collect()
}

pub fn block(
    store: &ReviewStore,
    review: &Review,
    candidate: &Candidate,
    insert: &Insert,
) -> Outcome<RichBlock> {
    let label = |path: &str| match insert.kind {
        InsertKind::Diff => format!("<refact-diff path=\"{path}\">"),
        _ => format!("<refact-code path=\"{path}\">"),
    };
    let located = |path: &str| {
        resolve_path(review, candidate, path).map_err(|error| format!("{}: {error}", label(path)))
    };
    let content = match insert.kind {
        InsertKind::Code => {
            let given = insert.path.as_deref().unwrap_or_default();
            let path = located(given)?;
            let (start, end) = insert.lines.unwrap_or((1, 1));
            RichBlockContent::Code {
                anchor: anchor_in(store, candidate, &path, insert.side.clone(), start, end)
                    .map_err(|error| format!("{}: {error}", label(given)))?,
            }
        }
        InsertKind::Diff => {
            let given = insert.path.as_deref().unwrap_or_default();
            let path = located(given)?;
            let changed = super::changed_files(candidate)
                .iter()
                .any(|(changed, _)| *changed == path);
            if !changed {
                return Err(format!(
                    "{}: this file is unchanged in the comparison; show it with <refact-code>",
                    label(given)
                ));
            }
            let anchors = match insert.lines {
                Some((start, end)) => {
                    vec![
                        anchor_in(store, candidate, &path, insert.side.clone(), start, end)
                            .map_err(|error| format!("{}: {error}", label(given)))?,
                    ]
                }
                None => vec![],
            };
            RichBlockContent::Diff {
                candidate_id: candidate.id.clone(),
                path,
                anchors,
            }
        }
        InsertKind::Evidence => {
            let id = insert.reference.clone().unwrap_or_default();
            let evidence = review
                .evidence
                .iter()
                .find(|evidence| evidence.id == id && evidence.candidate_id == candidate.id)
                .ok_or_else(|| {
                    let known: Vec<&str> = review
                        .evidence
                        .iter()
                        .filter(|evidence| evidence.candidate_id == candidate.id)
                        .map(|evidence| evidence.id.as_str())
                        .take(20)
                        .collect();
                    format!(
                        "<refact-evidence ref=\"{id}\">: this review has no such source or evidence. They belong to the PR they were recorded for: register one here with review_source, or drop the tag. {}",
                        if known.is_empty() {
                            "Nothing is registered here yet.".to_string()
                        } else {
                            format!("Known ids: {}", known.join(", "))
                        }
                    )
                })?;
            if matches!(evidence.source, EvidenceSource::Executed { .. }) {
                RichBlockContent::Evidence {
                    evidence_ids: vec![id],
                }
            } else {
                RichBlockContent::Sources {
                    evidence_ids: vec![id],
                    relation: SourceRelation::Supports,
                }
            }
        }
    };
    Ok(RichBlock {
        id: insert.id.clone(),
        revision: 1,
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_become_slots_with_generated_and_explicit_ids() {
        let html = r#"<p>a</p><refact-code path="src/a.rs" lines="3-9" highlight="4,6-7"></refact-code>
<refact-diff path='src/b.rs' id="the-diff" title="A &amp; B"/><refact-evidence ref="ev-1"></refact-evidence>"#;
        let (rewritten, inserts) = rewrite(html, &HashSet::new()).unwrap();
        assert_eq!(
            inserts
                .iter()
                .map(|insert| insert.id.as_str())
                .collect::<Vec<_>>(),
            ["code-1", "the-diff", "evidence-2"]
        );
        assert_eq!(inserts[0].lines, Some((3, 9)));
        assert_eq!(inserts[1].kind, InsertKind::Diff);
        assert_eq!(inserts[2].reference.as_deref(), Some("ev-1"));
        assert!(!rewritten.contains("<refact-code") && !rewritten.contains("<refact-diff"));
        assert!(rewritten
            .contains(r#"<refact-slot data-block="code-1" data-highlight="4,6-7"></refact-slot>"#));
        assert!(rewritten.contains(r#"data-block="the-diff" data-title="A &amp; B""#));
        assert_eq!(slot_ids(&rewritten), ["code-1", "the-diff", "evidence-2"]);
    }

    #[test]
    fn generated_ids_skip_slots_carried_from_the_previous_version() {
        let taken = HashSet::from(["code-1".to_string()]);
        let (_, inserts) = rewrite(r#"<refact-code path="a" lines="1">"#, &taken).unwrap();
        assert_eq!(inserts[0].id, "code-2");
    }

    #[test]
    fn errors_name_the_tag_and_the_missing_or_unknown_attribute() {
        for (html, expected) in [
            (r#"<refact-code path="a">"#, "needs `path` and `lines`"),
            (r#"<refact-diff lines="1-2">"#, "needs `path`"),
            (
                r#"<refact-code path="a" lines="0">"#,
                "lines must look like",
            ),
            (
                r#"<refact-code path="a" lines="1" colour="red">"#,
                "unknown attribute `colour`",
            ),
            (
                r#"<refact-code path="a" lines="1" id="x"><refact-diff path="b" id="x">"#,
                "used twice",
            ),
            (r#"<refact-evidence>"#, "needs `ref`"),
        ] {
            let error = rewrite(html, &HashSet::new()).unwrap_err();
            assert!(error.contains(expected), "{html}: {error}");
            assert!(error.starts_with("<refact-"), "{error}");
        }
    }
}
