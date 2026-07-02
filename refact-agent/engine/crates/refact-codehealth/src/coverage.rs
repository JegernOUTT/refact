use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileCoverage {
    pub path: String,
    pub lines_total: u32,
    pub lines_covered: u32,
    pub branches_total: u32,
    pub branches_covered: u32,
}

impl FileCoverage {
    pub fn line_pct(&self) -> f64 {
        pct(self.lines_covered, self.lines_total)
    }

    pub fn branch_pct(&self) -> f64 {
        pct(self.branches_covered, self.branches_total)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageReport {
    pub format: String,
    pub files: Vec<FileCoverage>,
}

pub fn parse_lcov(text: &str) -> CoverageReport {
    #[derive(Default)]
    struct Current {
        path: Option<String>,
        total_lines: BTreeSet<u32>,
        covered_lines: BTreeSet<u32>,
        branches_total: u32,
        branches_covered: u32,
        explicit_lf: Option<u32>,
        explicit_lh: Option<u32>,
        explicit_brf: Option<u32>,
        explicit_brh: Option<u32>,
    }

    fn flush(cur: &mut Current, files: &mut Vec<FileCoverage>) {
        let Some(path) = cur.path.take() else {
            return;
        };
        files.push(FileCoverage {
            path,
            lines_total: cur.explicit_lf.unwrap_or(cur.total_lines.len() as u32),
            lines_covered: cur.explicit_lh.unwrap_or(cur.covered_lines.len() as u32),
            branches_total: cur.explicit_brf.unwrap_or(cur.branches_total),
            branches_covered: cur.explicit_brh.unwrap_or(cur.branches_covered),
        });
        *cur = Current::default();
    }

    let mut files = Vec::new();
    let mut cur = Current::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == "end_of_record" {
            flush(&mut cur, &mut files);
            continue;
        }
        let Some((tag, rest)) = line.split_once(':') else {
            continue;
        };
        match tag {
            "SF" => cur.path = Some(normalize_path(rest)),
            "DA" => {
                let parts: Vec<_> = rest.split(',').collect();
                if parts.len() >= 2 {
                    if let (Some(line_no), Some(hits)) = (parse_u32(parts[0]), parse_u32(parts[1]))
                    {
                        cur.total_lines.insert(line_no);
                        if hits > 0 {
                            cur.covered_lines.insert(line_no);
                        }
                    }
                }
            }
            "BRDA" => {
                let parts: Vec<_> = rest.split(',').collect();
                if parts.len() == 4 {
                    cur.branches_total += 1;
                    if parse_u32(parts[3]).is_some_and(|hits| hits > 0) {
                        cur.branches_covered += 1;
                    }
                }
            }
            "LF" => cur.explicit_lf = parse_u32(rest),
            "LH" => cur.explicit_lh = parse_u32(rest),
            "BRF" => cur.explicit_brf = parse_u32(rest),
            "BRH" => cur.explicit_brh = parse_u32(rest),
            _ => {}
        }
    }
    flush(&mut cur, &mut files);
    CoverageReport {
        format: "lcov".to_string(),
        files,
    }
}

pub fn parse_cobertura(xml: &str) -> CoverageReport {
    #[derive(Default)]
    struct Bucket {
        total: BTreeSet<u32>,
        covered: BTreeSet<u32>,
        branches_total: u32,
        branches_covered: u32,
    }

    let mut buckets: BTreeMap<String, Bucket> = BTreeMap::new();
    for class in tag_sections(xml, "class") {
        let attrs = tag_attrs(class.open_tag);
        let Some(path) = attr_value(attrs, "filename") else {
            continue;
        };
        let bucket = buckets.entry(normalize_path(&path)).or_default();
        for line_tag in tags_in(class.body, "line") {
            let attrs = tag_attrs(line_tag);
            let Some(line_no) = attr_value(attrs, "number").and_then(|v| parse_u32(&v)) else {
                continue;
            };
            let hits = attr_value(attrs, "hits")
                .and_then(|v| parse_u32(&v))
                .unwrap_or(0);
            bucket.total.insert(line_no);
            if hits > 0 {
                bucket.covered.insert(line_no);
            }
            if attr_value(attrs, "branch").as_deref() == Some("true") {
                if let Some((covered, total)) = parse_condition_coverage(attrs) {
                    bucket.branches_covered += covered;
                    bucket.branches_total += total;
                } else {
                    bucket.branches_total += 2;
                }
            }
        }
    }

    CoverageReport {
        format: "cobertura".to_string(),
        files: buckets
            .into_iter()
            .map(|(path, b)| FileCoverage {
                path,
                lines_total: b.total.len() as u32,
                lines_covered: b.covered.len() as u32,
                branches_total: b.branches_total,
                branches_covered: b.branches_covered,
            })
            .collect(),
    }
}

pub fn parse_clover(xml: &str) -> CoverageReport {
    let mut files = Vec::new();
    for file in tag_sections(xml, "file") {
        let attrs = tag_attrs(file.open_tag);
        let Some(path) = attr_value(attrs, "path").or_else(|| attr_value(attrs, "name")) else {
            continue;
        };
        let mut coverage = FileCoverage {
            path: normalize_path(&path),
            lines_total: 0,
            lines_covered: 0,
            branches_total: 0,
            branches_covered: 0,
        };
        if let Some(metrics) = tags_in(file.body, "metrics").into_iter().next() {
            let attrs = tag_attrs(metrics);
            coverage.lines_total = attr_value(attrs, "statements")
                .and_then(|v| parse_u32(&v))
                .unwrap_or(0);
            coverage.lines_covered = attr_value(attrs, "coveredstatements")
                .and_then(|v| parse_u32(&v))
                .unwrap_or(0);
            coverage.branches_total = attr_value(attrs, "conditionals")
                .and_then(|v| parse_u32(&v))
                .unwrap_or(0);
            coverage.branches_covered = attr_value(attrs, "coveredconditionals")
                .and_then(|v| parse_u32(&v))
                .unwrap_or(0);
        } else {
            let mut total = BTreeSet::new();
            let mut covered = BTreeSet::new();
            for line_tag in tags_in(file.body, "line") {
                let attrs = tag_attrs(line_tag);
                let Some(line_no) = attr_value(attrs, "num").and_then(|v| parse_u32(&v)) else {
                    continue;
                };
                let count = attr_value(attrs, "count")
                    .and_then(|v| parse_u32(&v))
                    .unwrap_or(0);
                total.insert(line_no);
                if count > 0 {
                    covered.insert(line_no);
                }
                if attr_value(attrs, "type").as_deref() == Some("cond") {
                    coverage.branches_total += 2;
                    coverage.branches_covered += (attr_value(attrs, "truecount")
                        .and_then(|v| parse_u32(&v))
                        .unwrap_or(0)
                        > 0) as u32;
                    coverage.branches_covered += (attr_value(attrs, "falsecount")
                        .and_then(|v| parse_u32(&v))
                        .unwrap_or(0)
                        > 0) as u32;
                }
            }
            coverage.lines_total = total.len() as u32;
            coverage.lines_covered = covered.len() as u32;
        }
        files.push(coverage);
    }
    CoverageReport {
        format: "clover".to_string(),
        files,
    }
}

pub fn detect_and_parse(text: &str) -> Option<CoverageReport> {
    let trimmed = text.trim_start();
    let low = trimmed.to_ascii_lowercase();
    if trimmed.starts_with("TN:") || trimmed.starts_with("SF:") || trimmed.contains("\nSF:") {
        return Some(parse_lcov(text));
    }
    if low.contains("<coverage") && (low.contains("cobertura") || low.contains("line-rate")) {
        return Some(parse_cobertura(text));
    }
    if low.contains("<coverage") && (low.contains("clover") || low.contains("generated")) {
        return Some(parse_clover(text));
    }
    None
}

fn pct(covered: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        (covered as f64 / total as f64) * 100.0
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    s.trim().parse().ok()
}

fn normalize_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}

struct TagSection<'a> {
    open_tag: &'a str,
    body: &'a str,
}

fn tag_sections<'a>(text: &'a str, name: &str) -> Vec<TagSection<'a>> {
    let mut out = Vec::new();
    let mut pos = 0;
    let close_pat = format!("</{name}>");
    while let Some(start) = find_tag_start(text, name, pos) {
        let Some(open_end_rel) = text[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel + 1;
        let open_tag = &text[start..open_end];
        if open_tag.ends_with("/>") {
            out.push(TagSection { open_tag, body: "" });
            pos = open_end;
            continue;
        }
        let Some(close_rel) = text[open_end..].find(&close_pat) else {
            break;
        };
        let close = open_end + close_rel;
        out.push(TagSection {
            open_tag,
            body: &text[open_end..close],
        });
        pos = close + close_pat.len();
    }
    out
}

fn tags_in<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(start) = find_tag_start(text, name, pos) {
        let Some(end_rel) = text[start..].find('>') else {
            break;
        };
        let end = start + end_rel + 1;
        out.push(&text[start..end]);
        pos = end;
    }
    out
}

fn find_tag_start(text: &str, name: &str, from: usize) -> Option<usize> {
    let open_pat = format!("<{name}");
    let mut pos = from;
    while let Some(rel) = text[pos..].find(&open_pat) {
        let start = pos + rel;
        let after = start + open_pat.len();
        if text[after..]
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || c == '>' || c == '/')
        {
            return Some(start);
        }
        pos = after;
    }
    None
}

fn tag_attrs(tag: &str) -> &str {
    let trimmed = tag
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_end_matches('/');
    let Some(idx) = trimmed.find(char::is_whitespace) else {
        return "";
    };
    trimmed[idx..].trim_start()
}

fn attr_value(attrs: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let mut search_from = 0;
    let idx = loop {
        let found = search_from + attrs[search_from..].find(&needle)?;
        if found == 0
            || attrs[..found]
                .chars()
                .last()
                .is_some_and(char::is_whitespace)
        {
            break found + needle.len();
        }
        search_from = found + needle.len();
    };
    let rest = attrs[idx..].trim_start();
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let after = &rest[quote.len_utf8()..];
        let end = after.find(quote)?;
        Some(xml_unescape(&after[..end]))
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        Some(xml_unescape(&rest[..end]))
    }
}

fn xml_unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn parse_condition_coverage(attrs: &str) -> Option<(u32, u32)> {
    let val = attr_value(attrs, "condition-coverage")?;
    let open = val.find('(')?;
    let close = val[open..].find(')')? + open;
    let inner = &val[open + 1..close];
    let (covered, total) = inner.split_once('/')?;
    Some((parse_u32(covered)?, parse_u32(total)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lcov_blob() {
        let report = parse_lcov(
            "TN:\nSF:src/lib.rs\nDA:1,1\nDA:2,0\nLF:2\nLH:1\nBRF:2\nBRH:1\nend_of_record\n",
        );
        assert_eq!(report.format, "lcov");
        assert_eq!(report.files[0].lines_total, 2);
        assert_eq!(report.files[0].lines_covered, 1);
        assert_eq!(report.files[0].branches_total, 2);
        assert_eq!(report.files[0].branches_covered, 1);
        assert_eq!(report.files[0].line_pct(), 50.0);
    }

    #[test]
    fn parses_cobertura_xml() {
        let xml = r#"<coverage line-rate="0.5"><packages><package><classes><class filename="pkg/a.py"><lines><line number="1" hits="2"/><line number="2" hits="0" branch="true" condition-coverage="50% (1/2)"/></lines></class></classes></package></packages></coverage>"#;
        let report = parse_cobertura(xml);
        assert_eq!(report.files[0].path, "pkg/a.py");
        assert_eq!(report.files[0].lines_total, 2);
        assert_eq!(report.files[0].lines_covered, 1);
        assert_eq!(report.files[0].branches_total, 2);
        assert_eq!(report.files[0].branches_covered, 1);
    }

    #[test]
    fn lcov_branch_hits_must_be_numeric_and_positive() {
        let report = parse_lcov(
            "SF:src/lib.rs\nBRDA:1,0,0,1\nBRDA:2,0,0,0\nBRDA:3,0,0,-\nBRDA:4,0,0,abc\nend_of_record\n",
        );
        assert_eq!(report.files[0].branches_total, 4);
        assert_eq!(report.files[0].branches_covered, 1);
    }

    #[test]
    fn cobertura_branch_without_condition_coverage_is_uncovered() {
        let xml = r#"<coverage line-rate="1"><packages><package><classes><class filename="pkg/a.py"><lines><line number="1" hits="3" branch="true"/></lines></class></classes></package></packages></coverage>"#;
        let report = parse_cobertura(xml);
        assert_eq!(report.files[0].lines_total, 1);
        assert_eq!(report.files[0].lines_covered, 1);
        assert_eq!(report.files[0].branches_total, 2);
        assert_eq!(report.files[0].branches_covered, 0);
    }

    #[test]
    fn parses_clover_xml() {
        let xml = r#"<coverage generated="1"><project><file path="src/app.ts"><metrics statements="10" coveredstatements="7" conditionals="4" coveredconditionals="2" /></file></project></coverage>"#;
        let report = parse_clover(xml);
        assert_eq!(report.format, "clover");
        assert_eq!(report.files[0].path, "src/app.ts");
        assert_eq!(report.files[0].lines_total, 10);
        assert_eq!(report.files[0].lines_covered, 7);
    }
}
