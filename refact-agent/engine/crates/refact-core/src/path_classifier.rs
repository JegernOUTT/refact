pub const CATEGORY_CODE: &str = "code";
pub const CATEGORY_CONFIG: &str = "config";
pub const CATEGORY_DOC: &str = "doc";
pub const CATEGORY_DATA: &str = "data";
pub const CATEGORY_PIPELINE: &str = "pipeline";

const DOC_SUFFIXES: &[&str] = &[".md", ".mdx", ".rst", ".txt", ".adoc"];
const CONFIG_LANGUAGES: &[&str] = &["yaml", "toml", "json", "ini", "properties", "hcl"];
const DATA_DIR_TOKENS: &[&str] = &[
    "migrations",
    "versions",
    "models",
    "schema",
    "schemas",
    "entities",
];
const DATA_SUFFIXES: &[&str] = &[".sql", ".prisma", ".graphql", ".proto"];
const PIPELINE_PATH_HINTS: &[&str] = &[
    ".github/workflows/",
    ".gitlab-ci",
    "jenkinsfile",
    "azure-pipelines",
    ".circleci/",
    "/pipelines/",
    "/etl/",
];
const GLUE_STEMS: &[&str] = &["index", "mod"];
const CONVENTIONAL_ENTRY_STEMS: &[&str] = &[
    "main",
    "index",
    "app",
    "server",
    "cli",
    "bootstrap",
    "entry",
];

pub fn normalize_separators(path: &str) -> String {
    path.replace('\\', "/")
}

pub fn lower_normalized_path(path: &str) -> String {
    normalize_separators(path).to_lowercase()
}

pub fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("")
}

pub fn path_stem_lowercase(path: &str) -> String {
    let name = basename(path);
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_lowercase()
}

pub fn suffix_lowercase(path: &str) -> String {
    let name = basename(path);
    match name.rfind('.') {
        Some(index) if index < name.len() - 1 => name[index..].to_lowercase(),
        _ => String::new(),
    }
}

pub fn path_components(path: &str) -> impl Iterator<Item = &str> {
    path.split(['/', '\\']).filter(|part| !part.is_empty())
}

pub fn parent_segments_lowercase(path: &str) -> Vec<String> {
    let mut parts: Vec<&str> = path_components(path).collect();
    parts.pop();
    parts.into_iter().map(|part| part.to_lowercase()).collect()
}

pub fn entry_point_depth(path: &str) -> usize {
    path_components(path).count().saturating_sub(1)
}

pub fn has_path_separator(path: &str) -> bool {
    path.contains('/') || path.contains('\\')
}

pub fn is_glue_stem(stem: &str) -> bool {
    GLUE_STEMS.contains(&stem)
}

pub fn is_glue_leaf(path: &str) -> bool {
    is_glue_stem(&path_stem_lowercase(path)) && entry_point_depth(path) > 1
}

pub fn default_conventional_entry_stems() -> &'static [&'static str] {
    CONVENTIONAL_ENTRY_STEMS
}

pub fn is_conventional_entry(path: &str) -> bool {
    let stem = path_stem_lowercase(path);
    !is_glue_leaf(path) && CONVENTIONAL_ENTRY_STEMS.contains(&stem.as_str())
}

pub fn is_test_path(path: &str) -> bool {
    let normalized = lower_normalized_path(path);
    let base = basename(&normalized);
    base.starts_with("test_")
        || base.ends_with("_test.py")
        || base.ends_with("_test.go")
        || base.ends_with("_test.rs")
        || base.ends_with(".test.ts")
        || base.ends_with(".test.tsx")
        || base.ends_with(".test.js")
        || base.ends_with(".test.jsx")
        || base.ends_with(".test.mts")
        || base.ends_with(".test.cts")
        || base.ends_with(".test.rs")
        || base.ends_with(".spec.ts")
        || base.ends_with(".spec.tsx")
        || base.ends_with(".spec.js")
        || base.ends_with(".spec.jsx")
        || base.ends_with(".spec.mts")
        || base.ends_with(".spec.cts")
        || normalized.contains("/test/")
        || normalized.contains("/tests/")
        || normalized.contains("/__tests__/")
}

pub fn file_category(path: &str, language: &str, is_config: bool) -> &'static str {
    let suffix = suffix_lowercase(path);

    if DOC_SUFFIXES.contains(&suffix.as_str()) {
        return CATEGORY_DOC;
    }

    let lower_path = lower_normalized_path(path);
    if PIPELINE_PATH_HINTS
        .iter()
        .any(|hint| lower_path.contains(hint))
    {
        return CATEGORY_PIPELINE;
    }

    let parent_segments = parent_segments_lowercase(path);
    if DATA_SUFFIXES.contains(&suffix.as_str())
        || parent_segments
            .iter()
            .any(|segment| DATA_DIR_TOKENS.contains(&segment.as_str()))
    {
        return CATEGORY_DATA;
    }

    let lower_language = language.to_lowercase();
    if is_config || CONFIG_LANGUAGES.contains(&lower_language.as_str()) {
        return CATEGORY_CONFIG;
    }

    CATEGORY_CODE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_and_depth_are_separator_safe() {
        assert_eq!(basename(r"src\nested\main.rs"), "main.rs");
        assert_eq!(entry_point_depth(r"src\nested\main.rs"), 2);
        assert_eq!(path_stem_lowercase(r"Src\MAIN.RS"), "main");
    }

    #[test]
    fn test_path_classifier_covers_ts_js_variants() {
        assert!(is_test_path(r"src\Button.spec.tsx"));
        assert!(is_test_path("src/Button.spec.jsx"));
        assert!(is_test_path("src/Button.test.jsx"));
        assert!(is_test_path("src/Button.test.mts"));
        assert!(is_test_path("src/Button.spec.cts"));
        assert!(!is_test_path("src/contest.ts"));
    }

    #[test]
    fn conventional_entry_is_separator_safe() {
        assert!(is_conventional_entry(r"src\main.rs"));
        assert!(!is_conventional_entry(r"pkg\sub\index.ts"));
    }

    #[test]
    fn file_category_hints_are_separator_safe() {
        assert_eq!(
            file_category(r".github\workflows\ci.yml", "", false),
            CATEGORY_PIPELINE
        );
        assert_eq!(
            file_category(r"db\migrations\0001.py", "", false),
            CATEGORY_DATA
        );
        assert_eq!(file_category("README.md", "", false), CATEGORY_DOC);
    }
}
