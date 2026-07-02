use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeritageKind {
    Extends,
    Implements,
    TraitImpl,
    Derive,
    Mixin,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HeritageRel {
    pub subtype: String,
    pub base: String,
    pub kind: HeritageKind,
}

pub fn extract_heritage(lang: &str, text: &str) -> Vec<HeritageRel> {
    let lang = crate::normalize_lang(lang);
    let Some(_tree) = crate::parse_tree(lang, text) else {
        return Vec::new();
    };

    let mut out = match lang {
        "rust" => extract_rust(text),
        "java" => extract_java_like(text, false),
        "kotlin" => extract_kotlin(text),
        "csharp" => extract_csharp(text),
        "python" => extract_python(text),
        "ruby" => extract_ruby(text),
        "c" | "cpp" => extract_cpp(text),
        "typescript" | "javascript" => extract_ts(text),
        "go" => extract_go(text),
        "php" => extract_php(text),
        "swift" => extract_swift(text),
        "scala" => extract_scala(text),
        _ => Vec::new(),
    };
    dedup(&mut out);
    out
}

fn push(
    out: &mut Vec<HeritageRel>,
    subtype: impl Into<String>,
    base: impl Into<String>,
    kind: HeritageKind,
) {
    let subtype = clean_name(&subtype.into());
    let base = clean_name(&base.into());
    if !subtype.is_empty() && !base.is_empty() {
        out.push(HeritageRel {
            subtype,
            base,
            kind,
        });
    }
}

fn dedup(out: &mut Vec<HeritageRel>) {
    let mut seen = Vec::new();
    out.retain(|rel| {
        let key = (rel.subtype.clone(), rel.base.clone(), rel.kind);
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

fn clean_name(text: &str) -> String {
    let mut s = text
        .trim()
        .trim_matches(';')
        .trim_matches(',')
        .trim()
        .to_string();
    for prefix in [
        "extends ",
        "implements ",
        "public ",
        "protected ",
        "private ",
        "virtual ",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim().to_string();
        }
    }
    if let Some(idx) = s.find('(') {
        s.truncate(idx);
    }
    if let Some(idx) = s.find('<') {
        s.truncate(idx);
    }
    if let Some(idx) = s.find(':') {
        if !s.contains("::") {
            s.truncate(idx);
        }
    }
    s = s.trim().trim_matches('{').trim().to_string();
    s.rsplit("::")
        .next()
        .unwrap_or(&s)
        .rsplit('.')
        .next()
        .unwrap_or(&s)
        .trim()
        .to_string()
}

fn take_ident_after(line: &str, keyword: &str) -> Option<String> {
    let rest = line.split_once(keyword)?.1.trim_start();
    let ident: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    if ident.is_empty() {
        None
    } else {
        Some(ident)
    }
}

fn clause_after<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    line.split_once(keyword).map(|(_, rest)| rest.trim())
}

fn until_any<'a>(text: &'a str, stops: &[&str]) -> &'a str {
    let mut end = text.len();
    for stop in stops {
        if let Some(idx) = text.find(stop) {
            end = end.min(idx);
        }
    }
    &text[..end]
}

fn split_bases(text: &str) -> impl Iterator<Item = &str> {
    text.split(',')
        .flat_map(|part| part.split('+'))
        .map(str::trim)
}

fn parse_derive_attr(text: &str) -> Vec<String> {
    let Some(start) = text.find('(') else {
        return Vec::new();
    };
    let Some(end) = text.rfind(')') else {
        return Vec::new();
    };
    split_bases(&text[start + 1..end]).map(clean_name).collect()
}

fn extract_rust(text: &str) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    let mut pending_derives = Vec::new();
    let mut derive_attr = String::new();

    for raw in text.lines() {
        let line = raw.trim();
        if !derive_attr.is_empty() {
            derive_attr.push(' ');
            derive_attr.push_str(line);
            if line.contains(')') {
                pending_derives.extend(parse_derive_attr(&derive_attr));
                derive_attr.clear();
            }
            continue;
        }

        if line.starts_with("#[derive") {
            if line.contains(')') {
                pending_derives.extend(parse_derive_attr(line));
            } else {
                derive_attr.push_str(line);
            }
            continue;
        }

        if line.starts_with("#[") {
            continue;
        }

        if line.starts_with("struct ") || line.starts_with("enum ") {
            let keyword = if line.starts_with("struct ") {
                "struct"
            } else {
                "enum"
            };
            if let Some(name) = take_ident_after(line, keyword) {
                for base in pending_derives.drain(..) {
                    push(&mut out, name.clone(), base, HeritageKind::Derive);
                }
            }
        } else if !line.is_empty() {
            pending_derives.clear();
        }

        if line.starts_with("impl") && line.contains(" for ") {
            let body = line.trim_start_matches("impl").trim();
            if let Some((trait_part, type_part)) = body.split_once(" for ") {
                let ty = until_any(type_part, &["{", "where"]);
                push(&mut out, ty, trait_part, HeritageKind::TraitImpl);
            }
        }

        if line.starts_with("trait ") && line.contains(':') {
            if let Some(name) = take_ident_after(line, "trait") {
                if let Some(bounds) = line
                    .split_once(':')
                    .map(|(_, b)| until_any(b, &["{", "where", ";"]))
                {
                    for base in split_bases(bounds) {
                        push(&mut out, name.clone(), base, HeritageKind::Extends);
                    }
                }
            }
        }
    }
    out
}

fn extract_java_like(text: &str, interface_extends_is_implements: bool) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        for keyword in ["class", "interface", "record", "enum"] {
            let Some(name) = take_ident_after(line, keyword) else {
                continue;
            };
            if let Some(ext) = clause_after(line, " extends ") {
                let clause = until_any(ext, &[" implements ", " permits ", "{"]);
                let kind = if keyword == "interface" && interface_extends_is_implements {
                    HeritageKind::Implements
                } else {
                    HeritageKind::Extends
                };
                for base in split_bases(clause) {
                    push(&mut out, name.clone(), base, kind);
                }
            }
            if let Some(imp) = clause_after(line, " implements ") {
                let clause = until_any(imp, &["{"]);
                for base in split_bases(clause) {
                    push(&mut out, name.clone(), base, HeritageKind::Implements);
                }
            }
        }
    }
    out
}

fn extract_kotlin(text: &str) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = take_ident_after(line, "class") {
            if let Some(rest) = line.split_once(':').map(|(_, r)| until_any(r, &["{"])) {
                for base in split_bases(rest) {
                    let kind = if base.contains('(') {
                        HeritageKind::Extends
                    } else {
                        HeritageKind::Implements
                    };
                    push(&mut out, name.clone(), base, kind);
                }
            }
        }
        if let Some(name) = take_ident_after(line, "interface") {
            if let Some(rest) = line.split_once(':').map(|(_, r)| until_any(r, &["{"])) {
                for base in split_bases(rest) {
                    push(&mut out, name.clone(), base, HeritageKind::Extends);
                }
            }
        }
    }
    out
}

fn extract_csharp(text: &str) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        for keyword in ["class", "interface", "record", "struct"] {
            let Some(name) = take_ident_after(line, keyword) else {
                continue;
            };
            if let Some(rest) = line.split_once(':').map(|(_, r)| until_any(r, &["{"])) {
                for (idx, base) in split_bases(rest).enumerate() {
                    let kind = if keyword == "interface" || idx > 0 {
                        HeritageKind::Implements
                    } else {
                        HeritageKind::Extends
                    };
                    push(&mut out, name.clone(), base, kind);
                }
            }
        }
    }
    out
}

fn extract_python(text: &str) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("class ") {
            let Some((name, after_name)) = rest.split_once('(') else {
                continue;
            };
            let Some((bases, _)) = after_name.split_once(')') else {
                continue;
            };
            for base in split_bases(bases) {
                push(&mut out, name, base, HeritageKind::Extends);
            }
        }
    }
    out
}

fn extract_ruby(text: &str) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("class ") {
            let name = until_any(rest, &["<", "\n"]).trim();
            let name = clean_name(name);
            if let Some(base) = line.split_once('<').map(|(_, b)| b) {
                push(&mut out, name.clone(), base, HeritageKind::Extends);
            }
            if !name.is_empty() {
                stack.push(name);
            }
            continue;
        }
        if line == "end" {
            stack.pop();
            continue;
        }
        if let Some(class_name) = stack.last().cloned() {
            for method in ["include", "prepend", "extend"] {
                if let Some(rest) = line.strip_prefix(method) {
                    for base in split_bases(rest) {
                        push(&mut out, class_name.clone(), base, HeritageKind::Mixin);
                    }
                }
            }
        }
    }
    out
}

fn extract_cpp(text: &str) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        for keyword in ["class", "struct"] {
            let Some(name) = take_ident_after(line, keyword) else {
                continue;
            };
            if let Some(rest) = line.split_once(':').map(|(_, r)| until_any(r, &["{"])) {
                for base in split_bases(rest) {
                    push(&mut out, name.clone(), base, HeritageKind::Extends);
                }
            }
        }
    }
    out
}

fn extract_ts(text: &str) -> Vec<HeritageRel> {
    extract_java_like(text, false)
}

fn extract_go(text: &str) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    let mut current: Option<(String, String)> = None;

    for line in text.lines().map(str::trim) {
        if let Some((name, body)) = current.as_mut() {
            if let Some((before_close, _)) = line.split_once('}') {
                body.push('\n');
                body.push_str(before_close);
                for base in body.lines().flat_map(split_bases) {
                    if !base.contains('(') {
                        push(&mut out, name.clone(), base, HeritageKind::Implements);
                    }
                }
                current = None;
            } else {
                body.push('\n');
                body.push_str(line);
            }
            continue;
        }

        if line.starts_with("type ") && line.contains(" interface") {
            if let Some(name) = take_ident_after(line, "type") {
                if let Some(body) = line
                    .split_once('{')
                    .and_then(|(_, r)| r.split_once('}').map(|(b, _)| b))
                {
                    for base in body.lines().flat_map(split_bases) {
                        if !base.contains('(') {
                            push(&mut out, name.clone(), base, HeritageKind::Implements);
                        }
                    }
                } else if let Some((_, body_start)) = line.split_once('{') {
                    current = Some((name, body_start.to_string()));
                }
            }
        }
    }
    out
}

fn extract_php(text: &str) -> Vec<HeritageRel> {
    extract_java_like(text, false)
}

fn extract_swift(text: &str) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        for keyword in ["class", "struct", "enum", "protocol"] {
            let Some(name) = take_ident_after(line, keyword) else {
                continue;
            };
            if let Some(rest) = line.split_once(':').map(|(_, r)| until_any(r, &["{"])) {
                for (idx, base) in split_bases(rest).enumerate() {
                    let kind = if keyword == "class" && idx == 0 {
                        HeritageKind::Extends
                    } else {
                        HeritageKind::Implements
                    };
                    push(&mut out, name.clone(), base, kind);
                }
            }
        }
    }
    out
}

fn extract_scala(text: &str) -> Vec<HeritageRel> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        for keyword in ["class", "trait", "object"] {
            let Some(name) = take_ident_after(line, keyword) else {
                continue;
            };
            if let Some(ext) = clause_after(line, " extends ") {
                let parts: Vec<&str> = until_any(ext, &["{"]).split(" with ").collect();
                for (idx, base) in parts.iter().enumerate() {
                    let kind = if idx == 0 {
                        HeritageKind::Extends
                    } else {
                        HeritageKind::Implements
                    };
                    push(&mut out, name.clone(), *base, kind);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(rels: &[HeritageRel], subtype: &str, base: &str, kind: HeritageKind) -> bool {
        rels.iter()
            .any(|rel| rel.subtype == subtype && rel.base == base && rel.kind == kind)
    }

    #[test]
    fn extracts_rust_heritage() {
        let src = "#[derive(Clone, Debug)]\nstruct S;\nimpl Display for S {}\ntrait A: B {}";
        let rels = extract_heritage("rust", src);
        assert!(has(&rels, "S", "Clone", HeritageKind::Derive));
        assert!(has(&rels, "S", "Debug", HeritageKind::Derive));
        assert!(has(&rels, "S", "Display", HeritageKind::TraitImpl));
        assert!(has(&rels, "A", "B", HeritageKind::Extends));
    }

    #[test]
    fn extracts_rust_multiline_derives_across_attributes() {
        let src = "#[derive(\nClone,\nDebug,\n)]\n#[repr(C)]\nstruct S;";
        let rels = extract_heritage("rust", src);
        assert!(has(&rels, "S", "Clone", HeritageKind::Derive));
        assert!(has(&rels, "S", "Debug", HeritageKind::Derive));
    }

    #[test]
    fn extracts_java_heritage() {
        let rels = extract_heritage(
            "java",
            "class Dog extends Animal implements Pet, Runnable {}",
        );
        assert!(has(&rels, "Dog", "Animal", HeritageKind::Extends));
        assert!(has(&rels, "Dog", "Pet", HeritageKind::Implements));
        assert!(has(&rels, "Dog", "Runnable", HeritageKind::Implements));
    }

    #[test]
    fn extracts_python_heritage() {
        let rels = extract_heritage("python", "class C(A, B):\n    pass\n");
        assert!(has(&rels, "C", "A", HeritageKind::Extends));
        assert!(has(&rels, "C", "B", HeritageKind::Extends));
    }

    #[test]
    fn extracts_ruby_heritage() {
        let rels = extract_heritage("ruby", "class C < Base\n include M\nend");
        assert!(has(&rels, "C", "Base", HeritageKind::Extends));
        assert!(has(&rels, "C", "M", HeritageKind::Mixin));
    }

    #[test]
    fn extracts_typescript_heritage() {
        let rels = extract_heritage("typescript", "class C extends A implements I {}");
        assert!(has(&rels, "C", "A", HeritageKind::Extends));
        assert!(has(&rels, "C", "I", HeritageKind::Implements));
    }

    #[test]
    fn accepted_aliases_extract_heritage() {
        let cases = [
            (
                "py",
                "class C(A):\n    pass\n",
                "C",
                "A",
                HeritageKind::Extends,
            ),
            ("cs", "class C : A {}", "C", "A", HeritageKind::Extends),
            ("rb", "class C < A\nend", "C", "A", HeritageKind::Extends),
            (
                "c++",
                "class C : public A {};",
                "C",
                "A",
                HeritageKind::Extends,
            ),
            (
                "cc",
                "class C : public A {};",
                "C",
                "A",
                HeritageKind::Extends,
            ),
            (
                "cxx",
                "class C : public A {};",
                "C",
                "A",
                HeritageKind::Extends,
            ),
        ];

        for (lang, src, subtype, base, kind) in cases {
            let rels = extract_heritage(lang, src);
            assert!(has(&rels, subtype, base, kind), "{lang} got {rels:?}");
        }
    }

    #[test]
    fn typescript_interface_extends_is_extends() {
        let rels = extract_heritage("typescript", "interface Child extends Parent {}");
        assert!(has(&rels, "Child", "Parent", HeritageKind::Extends));
    }

    #[test]
    fn extracts_go_multiline_interface_embedding() {
        let rels = extract_heritage(
            "go",
            "type Reader interface {\n    io.Reader\n    Close() error\n}\n",
        );
        assert!(has(&rels, "Reader", "Reader", HeritageKind::Implements));
    }
}
