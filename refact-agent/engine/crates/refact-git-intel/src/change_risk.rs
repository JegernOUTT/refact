//! Kamei-style just-in-time change-risk scoring.
//!
//! This module is intentionally self-contained: it ships the offline-calibrated
//! RepoWise logistic constants and scores only the runtime-available change
//! features.

use serde::{Deserialize, Serialize};

use crate::ChangeFeatures;

/// Kamei just-in-time change metrics for one commit or change range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RiskInputs {
    /// Lines added.
    pub la: f64,
    /// Lines deleted.
    pub ld: f64,
    /// Number of files touched.
    pub nf: f64,
    /// Number of distinct directories touched.
    pub nd: f64,
    /// Number of distinct top-level subsystems touched.
    pub ns: f64,
    /// Shannon entropy of the per-file churn distribution.
    pub entropy: f64,
    /// Author's prior commit count / experience.
    pub exp: f64,
}

// Calibrated offline (2026-05-30) on a 7-repo, 5-language slice of the corpus
// (clap, pydantic, fd, gin, fastify, bat, chi; AG-SZZ bug-inducing commits as
// labels, gap_days=120 right-censoring, pooled L2-logistic over the genuinely
// runtime-available features). Generalization (leave-one-repo-out pooled OOF,
// n=4102, 662 inducing): model AUC 0.772 vs churn-only baseline 0.766.
//
// Ported verbatim from RepoWise `change_risk/model.py` _CONSTANTS. Order matters:
// features / log1p / mean / std / coef are parallel arrays.
const LOG1P: [bool; 7] = [true, true, true, true, true, false, true];
const MEAN: [f64; 7] = [2.443, 1.723, 1.006, 0.795, 0.806, 0.537, 3.011];
const STD: [f64; 7] = [1.414, 1.380, 0.465, 0.237, 0.285, 0.776, 2.270];
const COEF: [f64; 7] = [1.1241, 0.0151, -0.1103, -0.0310, -0.0672, 0.1483, -0.0702];
const INTERCEPT: f64 = -0.3797;

/// Score a change as a calibrated bug-inducing probability in `[0, 1]`.
///
/// The model is an L2-logistic regression over standardized features:
/// `sigmoid(intercept + Σ coef_i * z_i)`, where selected features first receive
/// the same `log1p` compression used during calibration.
pub fn score_change(inputs: &RiskInputs) -> f64 {
    let raw = [
        inputs.la,
        inputs.ld,
        inputs.nf,
        inputs.nd,
        inputs.ns,
        inputs.entropy,
        inputs.exp,
    ];

    let logit = raw.iter().enumerate().fold(INTERCEPT, |acc, (i, value)| {
        let x = if LOG1P[i] { value.ln_1p() } else { *value };
        let std = if STD[i] == 0.0 { 1.0 } else { STD[i] };
        let z = (x - MEAN[i]) / std;
        acc + COEF[i] * z
    });

    sigmoid(logit)
}

/// Bucket a calibrated probability into absolute tercile risk bands.
pub fn risk_band(p: f64) -> &'static str {
    if p < 0.33 {
        "low"
    } else if p < 0.66 {
        "medium"
    } else {
        "high"
    }
}

/// Lift the compact git-intel change features into the full Kamei risk vector.
pub fn from_change_features(cf: &ChangeFeatures, nd: f64, ns: f64, exp: f64) -> RiskInputs {
    RiskInputs {
        la: cf.la as f64,
        ld: cf.ld as f64,
        nf: cf.nf as f64,
        nd,
        ns,
        entropy: cf.entropy,
        exp,
    }
}

fn sigmoid(z: f64) -> f64 {
    if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        let e = z.exp();
        e / (1.0 + e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_scattered_low_experience_scores_higher_than_tiny_focused_experienced() {
        let large_scattered = RiskInputs {
            la: 900.0,
            ld: 250.0,
            nf: 24.0,
            nd: 10.0,
            ns: 5.0,
            entropy: 4.2,
            exp: 0.0,
        };
        let tiny_focused = RiskInputs {
            la: 2.0,
            ld: 0.0,
            nf: 1.0,
            nd: 1.0,
            ns: 1.0,
            entropy: 0.0,
            exp: 500.0,
        };

        assert!(score_change(&large_scattered) > score_change(&tiny_focused));
    }

    #[test]
    fn score_change_output_is_always_within_probability_bounds() {
        let cases = [
            RiskInputs {
                la: 0.0,
                ld: 0.0,
                nf: 0.0,
                nd: 0.0,
                ns: 0.0,
                entropy: 0.0,
                exp: 0.0,
            },
            RiskInputs {
                la: 50.0,
                ld: 10.0,
                nf: 3.0,
                nd: 2.0,
                ns: 1.0,
                entropy: 1.2,
                exp: 25.0,
            },
            RiskInputs {
                la: 1_000_000.0,
                ld: 1_000_000.0,
                nf: 10_000.0,
                nd: 1_000.0,
                ns: 100.0,
                entropy: 10.0,
                exp: 100_000.0,
            },
        ];

        for inputs in &cases {
            let score = score_change(inputs);
            assert!((0.0..=1.0).contains(&score));
        }
    }

    #[test]
    fn risk_band_maps_high_and_low_scores() {
        assert_eq!(risk_band(0.95), "high");
        assert_eq!(risk_band(0.05), "low");
    }

    #[test]
    fn from_change_features_lifts_fields_correctly() {
        let cf = ChangeFeatures {
            la: 12,
            ld: 5,
            nf: 3,
            entropy: 1.5,
        };

        let inputs = from_change_features(&cf, 2.0, 1.0, 42.0);

        assert_eq!(inputs.la, 12.0);
        assert_eq!(inputs.ld, 5.0);
        assert_eq!(inputs.nf, 3.0);
        assert_eq!(inputs.nd, 2.0);
        assert_eq!(inputs.ns, 1.0);
        assert_eq!(inputs.entropy, 1.5);
        assert_eq!(inputs.exp, 42.0);
    }
}
