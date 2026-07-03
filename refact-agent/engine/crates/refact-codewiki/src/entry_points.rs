use std::cmp::Ordering;
use std::collections::HashSet;

use refact_core::path_classifier;

pub fn entry_point_depth(path: &str) -> usize {
    path_classifier::entry_point_depth(path)
}

pub fn is_glue_leaf(path: &str) -> bool {
    path_classifier::is_glue_leaf(path)
}

fn name_bucket(path: &str, conventional_stems: &HashSet<String>) -> u8 {
    let stem = path_classifier::path_stem_lowercase(path);
    if path_classifier::is_glue_stem(&stem) {
        2
    } else if conventional_stems.contains(&stem) {
        0
    } else {
        1
    }
}

pub fn entry_point_rank_key(
    path: &str,
    pagerank: f64,
    betweenness: f64,
    conventional_stems: &HashSet<String>,
) -> (u8, usize, f64, String) {
    (
        name_bucket(path, conventional_stems),
        entry_point_depth(path),
        -(pagerank + betweenness),
        path.to_string(),
    )
}

pub fn rank_entry_points(
    candidates: &[(String, f64, f64)],
    conventional_stems: &HashSet<String>,
) -> Vec<String> {
    let mut ranked = candidates.to_vec();
    ranked.sort_by(|left, right| {
        let left_key = entry_point_rank_key(&left.0, left.1, left.2, conventional_stems);
        let right_key = entry_point_rank_key(&right.0, right.1, right.2, conventional_stems);
        compare_rank_keys(&left_key, &right_key)
    });
    ranked.into_iter().map(|(path, _, _)| path).collect()
}

pub fn default_conventional_stems() -> HashSet<String> {
    path_classifier::default_conventional_entry_stems()
        .iter()
        .copied()
        .map(String::from)
        .collect()
}

fn compare_rank_keys(
    left: &(u8, usize, f64, String),
    right: &(u8, usize, f64, String),
) -> Ordering {
    left.0
        .cmp(&right.0)
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.2.total_cmp(&right.2))
        .then_with(|| left.3.cmp(&right.3))
}

pub fn is_conventional_entry(path: &str) -> bool {
    path_classifier::is_conventional_entry(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_conventional_entry_matches_stems_but_not_glue_leaves() {
        assert!(is_conventional_entry("src/main.rs"));
        assert!(is_conventional_entry(r"src\\main.rs"));
        assert!(is_conventional_entry("index.js"));
        assert!(!is_conventional_entry("pkg/sub/index.ts"));
        assert!(!is_conventional_entry("app/__init__.py"));
        assert!(!is_conventional_entry("src/helpers.rs"));
    }

    #[test]
    fn depth_uses_posix_parts() {
        assert_eq!(entry_point_depth("main.py"), 0);
        assert_eq!(entry_point_depth("src/main.py"), 1);
        assert_eq!(entry_point_depth("a/b/c.py"), 2);
        assert_eq!(entry_point_depth("/a//b/c.py"), 2);
        assert_eq!(entry_point_depth(r"a\\b\\c.py"), 2);
    }

    #[test]
    fn ranks_by_bucket_depth_centrality_and_path() {
        let conventional = default_conventional_stems();
        let candidates = vec![
            ("pkg/sub/index.ts".to_string(), 100.0, 100.0),
            ("src/util.ts".to_string(), 0.0, 0.0),
            ("main.py".to_string(), 0.0, 0.0),
        ];

        assert_eq!(
            rank_entry_points(&candidates, &conventional),
            vec![
                "main.py".to_string(),
                "src/util.ts".to_string(),
                "pkg/sub/index.ts".to_string(),
            ]
        );
    }

    #[test]
    fn glue_stems_take_precedence_over_conventional_stems() {
        let conventional = default_conventional_stems();
        let key = entry_point_rank_key("pkg/sub/index.ts", 0.0, 0.0, &conventional);

        assert_eq!(key.0, 2);
    }

    #[test]
    fn glue_leaf_requires_deep_depth() {
        assert!(!is_glue_leaf("index.js"));
        assert!(is_glue_leaf("a/b/index.js"));
        assert!(is_glue_leaf(r"a\\b\\index.js"));
    }

    #[test]
    fn centrality_only_breaks_equal_bucket_and_depth_ties() {
        let conventional = default_conventional_stems();
        let candidates = vec![
            ("zeta.ts".to_string(), 0.0, 0.0),
            ("alpha.ts".to_string(), 1.0, 1.0),
            ("src/main.py".to_string(), 100.0, 100.0),
        ];

        assert_eq!(
            rank_entry_points(&candidates, &conventional),
            vec![
                "src/main.py".to_string(),
                "alpha.ts".to_string(),
                "zeta.ts".to_string(),
            ]
        );
    }

    #[test]
    fn stable_sort_preserves_equal_rank_keys() {
        let conventional = default_conventional_stems();
        let candidates = vec![
            ("same.ts".to_string(), 1.0, 1.0),
            ("same.ts".to_string(), 1.0, 1.0),
        ];

        assert_eq!(
            rank_entry_points(&candidates, &conventional),
            vec!["same.ts".to_string(), "same.ts".to_string()]
        );
    }
}
