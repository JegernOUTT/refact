use std::collections::{HashMap, HashSet};

use refact_core::path_classifier;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedPage {
    pub page_id: String,
    pub title: String,
    pub page_type: String,
    pub target_path: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WikiLink {
    pub anchor: String,
    pub target_page_id: String,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backlink {
    pub source_page_id: String,
    pub source_title: String,
    pub source_page_type: String,
    pub anchor: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkIndex {
    by_path: HashMap<String, String>,
    by_basename: HashMap<String, String>,
    by_symbol_qname: HashMap<String, String>,
    by_target: HashMap<String, String>,
}

impl LinkIndex {
    pub fn build(pages: &[GeneratedPage]) -> LinkIndex {
        let mut index = LinkIndex::default();

        for page in pages {
            if page.target_path.is_empty() {
                continue;
            }

            index
                .by_target
                .insert(page.target_path.clone(), page.page_id.clone());

            if matches!(
                page.page_type.as_str(),
                "file_page" | "api_contract" | "infra_page"
            ) {
                index
                    .by_path
                    .insert(page.target_path.clone(), page.page_id.clone());
                index
                    .by_basename
                    .entry(basename(&page.target_path).to_string())
                    .or_insert_with(|| page.page_id.clone());
            }

            if page.page_type == "symbol_spotlight" && page.target_path.contains("::") {
                index
                    .by_symbol_qname
                    .entry(page.target_path.clone())
                    .or_insert_with(|| page.page_id.clone());
                if let Some(short_name) = page.target_path.rsplit("::").next() {
                    index
                        .by_symbol_qname
                        .entry(short_name.to_string())
                        .or_insert_with(|| page.page_id.clone());
                }
            }
        }

        index
    }

    pub fn resolve(&self, ref_: &str) -> Option<&String> {
        if is_file_ref(ref_) {
            if let Some(page_id) = self.by_path.get(ref_) {
                return Some(page_id);
            }
        }

        if let Some(page_id) = self.by_target.get(ref_) {
            return Some(page_id);
        }

        if let Some(page_id) = self.by_path.get(ref_) {
            return Some(page_id);
        }

        if !path_classifier::has_path_separator(ref_) && ref_.contains('.') {
            if let Some(page_id) = self.by_basename.get(ref_) {
                return Some(page_id);
            }
        }

        self.by_symbol_qname.get(ref_)
    }
}

pub fn extract_backtick_refs(text: &str) -> Vec<String> {
    let stripped = strip_fenced_code_blocks(text);
    let mut refs = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = stripped[cursor..].find('`') {
        let start = cursor + relative_start;
        if is_preceded_by_backtick(&stripped, start) || is_followed_by_backtick(&stripped, start) {
            cursor = start + 1;
            continue;
        }

        let inner_start = start + 1;
        let mut search_from = inner_start;
        let mut found_end = None;

        while let Some(relative_end) = stripped[search_from..].find('`') {
            let end = search_from + relative_end;
            if !is_preceded_by_backtick(&stripped, end) && !is_followed_by_backtick(&stripped, end)
            {
                found_end = Some(end);
                break;
            }
            search_from = end + 1;
        }

        let Some(end) = found_end else {
            break;
        };

        let token = stripped[inner_start..end].trim();
        if is_valid_ref_token(token) {
            refs.push(token.to_string());
        }

        cursor = end + 1;
    }

    refs
}

pub fn resolve_wiki_links(
    page: &GeneratedPage,
    index: &LinkIndex,
    max_links_per_page: usize,
) -> Vec<WikiLink> {
    let text = strip_fenced_code_blocks(&page.content);
    let mut links = Vec::new();
    let mut seen = HashSet::new();

    for ref_ in extract_backtick_refs(&text) {
        if links.len() >= max_links_per_page {
            break;
        }

        let Some(target_page_id) = index.resolve(&ref_) else {
            continue;
        };

        if target_page_id == &page.page_id || !seen.insert(target_page_id.clone()) {
            continue;
        }

        let kind = if is_file_ref(&ref_) { "file" } else { "symbol" };
        links.push(WikiLink {
            anchor: ref_,
            target_page_id: target_page_id.clone(),
            kind: kind.to_string(),
        });
    }

    links
}

pub fn attach_wiki_links_and_backlinks(
    pages: &[GeneratedPage],
) -> (
    HashMap<String, Vec<WikiLink>>,
    HashMap<String, Vec<Backlink>>,
) {
    const BACKLINK_CAP: usize = 25;

    let index = LinkIndex::build(pages);
    let page_ids: HashSet<String> = pages.iter().map(|page| page.page_id.clone()).collect();
    let mut forward = HashMap::new();

    for page in pages {
        forward.insert(page.page_id.clone(), resolve_wiki_links(page, &index, 50));
    }

    let mut backlinks: HashMap<String, Vec<Backlink>> = HashMap::new();
    let mut seen_sources: HashMap<String, HashSet<String>> = HashMap::new();

    for page in pages {
        let Some(links) = forward.get(&page.page_id) else {
            continue;
        };

        for link in links {
            if !page_ids.contains(&link.target_page_id) {
                continue;
            }

            let target_seen = seen_sources.entry(link.target_page_id.clone()).or_default();
            if target_seen.contains(&page.page_id) {
                continue;
            }

            let target_backlinks = backlinks.entry(link.target_page_id.clone()).or_default();
            if target_backlinks.len() >= BACKLINK_CAP {
                continue;
            }

            target_seen.insert(page.page_id.clone());
            target_backlinks.push(Backlink {
                source_page_id: page.page_id.clone(),
                source_title: page.title.clone(),
                source_page_type: page.page_type.clone(),
                anchor: link.anchor.clone(),
            });
        }
    }

    (forward, backlinks)
}

fn strip_fenced_code_blocks(text: &str) -> String {
    let mut result = String::new();
    let mut cursor = 0;

    while let Some(relative_start) = text[cursor..].find("```") {
        let start = cursor + relative_start;
        result.push_str(&text[cursor..start]);
        let after_start = start + 3;

        if let Some(relative_end) = text[after_start..].find("```") {
            cursor = after_start + relative_end + 3;
        } else {
            cursor = text.len();
            break;
        }
    }

    result.push_str(&text[cursor..]);
    result
}

fn basename(path: &str) -> &str {
    path_classifier::basename(path)
}

fn is_preceded_by_backtick(text: &str, index: usize) -> bool {
    index > 0 && text[..index].ends_with('`')
}

fn is_followed_by_backtick(text: &str, index: usize) -> bool {
    text[index + 1..].starts_with('`')
}

fn is_valid_ref_token(token: &str) -> bool {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !matches!(first, 'A'..='Z' | 'a'..='z' | '_' | '/' | '.') {
        return false;
    }

    chars.all(|ch| matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '.' | '/' | '-'))
}

fn is_file_ref(ref_: &str) -> bool {
    path_classifier::has_path_separator(ref_)
        || [".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".rs", ".java"]
            .iter()
            .any(|suffix| ref_.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(
        page_id: &str,
        title: &str,
        page_type: &str,
        target_path: &str,
        content: &str,
    ) -> GeneratedPage {
        GeneratedPage {
            page_id: page_id.to_string(),
            title: title.to_string(),
            page_type: page_type.to_string(),
            target_path: target_path.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn resolve_priority_prefers_path_for_path_like_refs() {
        let pages = vec![
            page("target", "Target", "overview", "same.rs", ""),
            page("path", "Path", "file_page", "same.rs", ""),
            page("base", "Base", "file_page", "dir/base.rs", ""),
            page("symbol", "Symbol", "symbol_spotlight", "crate::Thing", ""),
        ];
        let index = LinkIndex::build(&pages);

        assert_eq!(index.resolve("same.rs"), Some(&"path".to_string()));
        assert_eq!(index.resolve("base.rs"), Some(&"base".to_string()));
        assert_eq!(index.resolve("Thing"), Some(&"symbol".to_string()));
    }
}
