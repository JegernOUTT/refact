pub fn is_test_path(path: &str) -> bool {
    refact_core::path_classifier::is_test_path(path)
}

pub fn normalize_separators(path: &str) -> String {
    path.trim().replace('\\', "/")
}

pub fn repo_relative<'a>(path: &'a str, repo_root: &str) -> Option<&'a str> {
    let path = path.trim();
    let normalized_path = normalize_separators(path);
    let root = normalize_repo_root(repo_root);
    if root.is_empty() {
        return None;
    }
    if normalized_path == root {
        return Some("");
    }
    let remainder = normalized_path.strip_prefix(&root)?;
    let suffix = if root.ends_with('/') {
        remainder
    } else {
        remainder.strip_prefix('/')?
    };
    let suffix_start = normalized_path.len().checked_sub(suffix.len())?;
    Some(&path[suffix_start..])
}

pub fn repo_relative_or_basename(path: &str, repo_root: &str) -> String {
    if let Some(relative) = repo_relative(path, repo_root) {
        return normalize_separators(relative);
    }

    let normalized = normalize_separators(path);
    if normalized.is_empty() || !is_absolute_path(&normalized) {
        return normalized;
    }

    basename(&normalized)
        .map(str::to_string)
        .unwrap_or(normalized)
}

pub fn paths_refer_to_same_file(a: &str, b: &str, repo_root: &str) -> bool {
    repo_relative_or_basename(a, repo_root) == repo_relative_or_basename(b, repo_root)
}

fn normalize_repo_root(repo_root: &str) -> String {
    let mut root = normalize_separators(repo_root);
    while root.len() > 1 && root.ends_with('/') {
        root.pop();
    }
    root
}

fn is_absolute_path(path: &str) -> bool {
    path.starts_with('/') || path.as_bytes().get(1).is_some_and(|byte| *byte == b':')
}

fn basename(path: &str) -> Option<&str> {
    path.rsplit('/').find(|segment| !segment.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_test_paths_including_mts_cts() {
        assert!(is_test_path("src/test_foo.py"));
        assert!(is_test_path("src/foo_test.py"));
        assert!(is_test_path("src/foo_test.go"));
        assert!(is_test_path("src/foo.test.ts"));
        assert!(is_test_path("src/foo.test.tsx"));
        assert!(is_test_path("src/foo.test.jsx"));
        assert!(is_test_path("src/foo.spec.tsx"));
        assert!(is_test_path("src/foo.spec.jsx"));
        assert!(is_test_path(r"pkg\\tests\\foo.rs"));
        assert!(is_test_path("src/foo.test.mts"));
        assert!(is_test_path("src/foo.test.cts"));
        assert!(is_test_path("src/foo.spec.mts"));
        assert!(is_test_path("src/foo.spec.cts"));
        assert!(is_test_path("pkg/tests/foo.rs"));
        assert!(is_test_path("pkg/__tests__/foo.ts"));
        assert!(!is_test_path("src/main.rs"));
        assert!(!is_test_path("src/contest.rs"));
    }

    #[test]
    fn normalizes_separators_and_trims_edges() {
        assert_eq!(
            normalize_separators(r"  pkg\\tests\\foo.rs  "),
            "pkg//tests//foo.rs"
        );
    }

    #[test]
    fn repo_relative_strips_root_prefix_with_boundary() {
        assert_eq!(
            repo_relative("/repo/src/lib.rs", "/repo"),
            Some("src/lib.rs")
        );
        assert_eq!(
            repo_relative("/repo/src/lib.rs", "/repo/"),
            Some("src/lib.rs")
        );
        assert_eq!(
            repo_relative(r"C:\repo\src\lib.rs", r"C:\repo\"),
            Some(r"src\lib.rs")
        );
        assert_eq!(repo_relative("/repo2/src/lib.rs", "/repo"), None);
        assert_eq!(repo_relative("src/lib.rs", "/repo"), None);
    }

    #[test]
    fn repo_relative_or_basename_maps_join_paths() {
        let cases = [
            ("abs-under-root", "/repo/src/lib.rs", "/repo", "src/lib.rs"),
            ("already-relative", "src/lib.rs", "/repo", "src/lib.rs"),
            (
                "backslash-mix",
                r"C:\repo\src\lib.rs",
                r"C:\repo\",
                "src/lib.rs",
            ),
            (
                "trailing-slash-root",
                "/repo/src/lib.rs",
                "/repo/",
                "src/lib.rs",
            ),
            (
                "basename-fallback",
                "/elsewhere/src/lib.rs",
                "/repo",
                "lib.rs",
            ),
        ];

        for (name, path, root, expected) in cases {
            assert_eq!(repo_relative_or_basename(path, root), expected, "{name}");
        }
    }

    #[test]
    fn paths_refer_to_same_file_compares_join_keys() {
        assert!(paths_refer_to_same_file(
            "/repo/src/lib.rs",
            "src/lib.rs",
            "/repo"
        ));
        assert!(paths_refer_to_same_file(
            r"C:\repo\src\lib.rs",
            "src/lib.rs",
            r"C:\repo"
        ));
        assert!(paths_refer_to_same_file(
            "/elsewhere/a/lib.rs",
            "/other/b/lib.rs",
            "/repo"
        ));
        assert!(!paths_refer_to_same_file(
            "/repo/src/lib.rs",
            "other/lib.rs",
            "/repo"
        ));
    }
}
