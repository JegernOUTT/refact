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
