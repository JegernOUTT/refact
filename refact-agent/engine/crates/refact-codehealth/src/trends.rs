use crate::biomarkers::{Dimension, Finding, Severity};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

const CATEGORY: &str = "trend";
const DECLINE_DROP_THRESHOLD: f64 = 1.0;
const PREDICTED_DECLINE_MIN_SNAPSHOTS: usize = 4;
const PREDICTED_DECLINE_HORIZON: f64 = 3.0;
const PREDICTED_DECLINE_LIMIT: f64 = 4.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthSnapshot {
    pub ts: i64,
    pub per_file: HashMap<String, f64>,
    pub aggregate: f64,
}

pub fn evaluate_trends(history: &[HealthSnapshot]) -> Vec<Finding> {
    let mut out = Vec::new();
    for path in paths_in_history(history) {
        let series = file_series(history, &path);
        if let Some(finding) = declining_finding(&path, &series) {
            out.push(finding);
        }
        if let Some(finding) = predicted_decline_finding(&path, &series) {
            out.push(finding);
        }
    }
    out.sort_by(|a, b| a.detail.cmp(&b.detail).then(a.biomarker.cmp(&b.biomarker)));
    out
}

pub fn push_snapshot(history: &mut Vec<HealthSnapshot>, snap: HealthSnapshot, max_len: usize) {
    if max_len == 0 {
        history.clear();
        return;
    }
    history.push(snap);
    let overflow = history.len().saturating_sub(max_len);
    if overflow > 0 {
        history.drain(0..overflow);
    }
}

fn paths_in_history(history: &[HealthSnapshot]) -> Vec<String> {
    let mut paths = BTreeSet::new();
    for snap in history {
        paths.extend(snap.per_file.keys().cloned());
    }
    paths.into_iter().collect()
}

fn file_series(history: &[HealthSnapshot], path: &str) -> Vec<(i64, f64)> {
    history
        .iter()
        .filter_map(|snap| snap.per_file.get(path).map(|score| (snap.ts, *score)))
        .collect()
}

fn declining_finding(path: &str, series: &[(i64, f64)]) -> Option<Finding> {
    if series.len() < 4 {
        return None;
    }
    let tail = &series[series.len() - 4..];
    let strictly_dropping = tail[0].1 > tail[1].1 && tail[1].1 > tail[2].1 && tail[2].1 > tail[3].1;
    let total_drop = tail[0].1 - tail[3].1;
    if strictly_dropping && total_drop >= DECLINE_DROP_THRESHOLD {
        Some(finding(
            "health_declining",
            Severity::High,
            format!(
                "{path}: health dropped {:.2} points across 3 consecutive drops ({:.2} -> {:.2} -> {:.2} -> {:.2})",
                total_drop, tail[0].1, tail[1].1, tail[2].1, tail[3].1
            ),
        ))
    } else {
        None
    }
}

fn predicted_decline_finding(path: &str, series: &[(i64, f64)]) -> Option<Finding> {
    if series.len() < PREDICTED_DECLINE_MIN_SNAPSHOTS {
        return None;
    }
    let slope = linear_slope(series);
    if slope >= 0.0 {
        return None;
    }
    let current = series.last()?;
    let projected = current.1 + slope * PREDICTED_DECLINE_HORIZON;
    if projected < PREDICTED_DECLINE_LIMIT {
        Some(finding(
            "predicted_decline",
            Severity::Medium,
            format!(
                "{path}: health trend projects below {:.1} within 3 snapshots ({:.2} -> {:.2})",
                PREDICTED_DECLINE_LIMIT, current.1, projected
            ),
        ))
    } else {
        None
    }
}

fn linear_slope(series: &[(i64, f64)]) -> f64 {
    let n = series.len() as f64;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = series.iter().map(|(_, y)| *y).sum::<f64>() / n;
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for (idx, (_, y)) in series.iter().enumerate() {
        let x = idx as f64;
        numerator += (x - mean_x) * (y - mean_y);
        denominator += (x - mean_x).powi(2);
    }
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

fn finding(biomarker: &str, severity: Severity, detail: String) -> Finding {
    Finding {
        biomarker: biomarker.to_string(),
        category: CATEGORY.to_string(),
        dimension: Dimension::Maintainability,
        severity,
        line: 1,
        detail,
        deduction: None,
    }
}
