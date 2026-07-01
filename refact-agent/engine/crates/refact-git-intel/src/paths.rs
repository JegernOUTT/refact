pub fn is_test_path(path: &str) -> bool {
    let p = path.to_lowercase().replace('\\', "/");
    let base = p.rsplit('/').next().unwrap_or(&p);
    base.starts_with("test_")
        || base.ends_with("_test.py")
        || base.ends_with("_test.go")
        || base.ends_with(".test.ts")
        || base.ends_with(".test.tsx")
        || base.ends_with(".test.js")
        || base.ends_with(".test.mts")
        || base.ends_with(".test.cts")
        || base.ends_with(".spec.ts")
        || base.ends_with(".spec.js")
        || base.ends_with(".spec.mts")
        || base.ends_with(".spec.cts")
        || p.contains("/test/")
        || p.contains("/tests/")
        || p.contains("/__tests__/")
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
