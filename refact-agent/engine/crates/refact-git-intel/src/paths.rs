pub fn is_test_path(path: &str) -> bool {
    refact_core::path_classifier::is_test_path(path)
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
}
