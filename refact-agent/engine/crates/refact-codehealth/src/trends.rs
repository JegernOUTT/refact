use serde::{Deserialize, Serialize};

const DECLINE_THRESHOLD: f64 = 0.5;
const DECLINE_LOOKBACK: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HealthSnapshot {
    pub ts: i64,
    pub hotspot_health: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrendKind {
    Declining,
    PredictedDecline,
    Improving,
    Stable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendAlert {
    pub kind: TrendKind,
    pub detail: String,
}

pub fn analyze_trend(history: &[HealthSnapshot]) -> Option<TrendAlert> {
    if history.is_empty() {
        return None;
    }

    if let Some(alert) = declining(history) {
        return Some(alert);
    }

    if history.len() >= 3 {
        let tail = &history[history.len() - 3..];
        if tail[0].hotspot_health > tail[1].hotspot_health
            && tail[1].hotspot_health > tail[2].hotspot_health
        {
            return Some(TrendAlert {
                kind: TrendKind::PredictedDecline,
                detail: format!(
                    "Hotspot health declined for the last 3 snapshots ({:.2} -> {:.2} -> {:.2}).",
                    tail[0].hotspot_health, tail[1].hotspot_health, tail[2].hotspot_health
                ),
            });
        }
        if tail[0].hotspot_health < tail[1].hotspot_health
            && tail[1].hotspot_health < tail[2].hotspot_health
        {
            return Some(TrendAlert {
                kind: TrendKind::Improving,
                detail: format!(
                    "Hotspot health improved for the last 3 snapshots ({:.2} -> {:.2} -> {:.2}).",
                    tail[0].hotspot_health, tail[1].hotspot_health, tail[2].hotspot_health
                ),
            });
        }
    }

    Some(TrendAlert {
        kind: TrendKind::Stable,
        detail: "Hotspot health is stable in the recent history.".to_string(),
    })
}

fn declining(history: &[HealthSnapshot]) -> Option<TrendAlert> {
    if history.len() <= DECLINE_LOOKBACK {
        return None;
    }
    let current = history.last()?;
    let baseline = &history[history.len() - 1 - DECLINE_LOOKBACK];
    let delta = current.hotspot_health - baseline.hotspot_health;
    if delta <= -DECLINE_THRESHOLD {
        Some(TrendAlert {
            kind: TrendKind::Declining,
            detail: format!(
                "Hotspot health dropped {:.2} points vs. snapshot {} ago ({:.2} -> {:.2}).",
                delta.abs(),
                DECLINE_LOOKBACK,
                baseline.hotspot_health,
                current.hotspot_health
            ),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(ts: i64, hotspot_health: f64) -> HealthSnapshot {
        HealthSnapshot { ts, hotspot_health }
    }

    #[test]
    fn three_strict_drops_yield_predicted_decline() {
        let history = vec![snap(1, 8.0), snap(2, 7.0), snap(3, 6.0)];

        let alert = analyze_trend(&history).unwrap();
        assert_eq!(alert.kind, TrendKind::PredictedDecline);
    }

    #[test]
    fn current_half_point_lower_than_five_ago_yields_declining() {
        let history = vec![
            snap(1, 9.0),
            snap(2, 8.9),
            snap(3, 8.8),
            snap(4, 8.7),
            snap(5, 8.6),
            snap(6, 8.4),
        ];

        let alert = analyze_trend(&history).unwrap();
        assert_eq!(alert.kind, TrendKind::Declining);
    }
}
