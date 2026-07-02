use crate::biomarkers::{Dimension, Finding, Severity};
use refact_core::path_classifier::is_test_path;
use std::collections::{HashMap, HashSet};

const SMALL_TEAM_MAX_CONTRIBUTORS: u32 = 3;

pub struct CoChangePartner {
    pub path: String,
    pub co_change_count: f64,
}

pub struct FunctionGitFacts {
    pub name: String,
    pub median_age_days: u32,
    pub recent_mod_count: u32,
    pub modification_count: u32,
    pub ccn: u32,
    pub max_nesting: u32,
}

pub struct GitMeta {
    pub file_path: String,
    pub change_entropy: f64,
    pub change_entropy_pct: f64,
    pub commit_count_90d: u32,
    pub commit_count_total: u32,
    pub is_hotspot: bool,
    pub is_stable: bool,
    pub churn_percentile: f64,
    pub lines_added_90d: u32,
    pub lines_deleted_90d: u32,
    pub nloc: u32,
    pub contributor_count: u32,
    pub primary_owner_commit_pct: f64,
    pub primary_owner_name: String,
    pub recent_owner_name: String,
    pub recent_owner_commit_pct: f64,
    pub bus_factor: u32,
    pub prior_defect_count: u32,
    pub repo_active_contributors_90d: Option<u32>,
    pub repo_function_mod_p80: Option<u32>,
    pub co_change_partners: Vec<CoChangePartner>,
    pub top_authors: Vec<(String, u32)>,
    pub functions: Vec<FunctionGitFacts>,
    pub repo_commit_counts: HashMap<String, u32>,
    pub import_edges: HashSet<String>,
}

pub fn git_biomarkers(meta: &GitMeta) -> Vec<Finding> {
    let mut out = Vec::new();
    detect_change_entropy(meta, &mut out);
    detect_churn_risk(meta, &mut out);
    detect_co_change_scatter(meta, &mut out);
    detect_code_age_volatility(meta, &mut out);
    detect_developer_congestion(meta, &mut out);
    detect_function_hotspot(meta, &mut out);
    detect_hidden_coupling(meta, &mut out);
    detect_knowledge_loss(meta, &mut out);
    detect_ownership_risk(meta, &mut out);
    detect_prior_defect(meta, &mut out);
    out.sort_by(|a, b| a.biomarker.cmp(&b.biomarker).then(a.detail.cmp(&b.detail)));
    out
}

fn finding(biomarker: &str, severity: Severity, detail: String) -> Finding {
    Finding {
        biomarker: biomarker.to_string(),
        category: "organizational".to_string(),
        dimension: Dimension::Defect,
        severity,
        line: 1,
        detail,
    }
}

fn detect_change_entropy(meta: &GitMeta, out: &mut Vec<Finding>) {
    if meta.change_entropy > 0.0 && meta.change_entropy_pct >= 0.80 && meta.commit_count_90d >= 3 {
        let severity = if meta.change_entropy_pct >= 0.95 && meta.is_hotspot {
            Severity::Critical
        } else if meta.change_entropy_pct >= 0.90 {
            Severity::High
        } else {
            Severity::Medium
        };
        out.push(finding(
            "change_entropy",
            severity,
            format!(
                "change entropy pct {:.2} across {} commits",
                meta.change_entropy_pct, meta.commit_count_90d
            ),
        ));
    }
}

fn detect_churn_risk(meta: &GitMeta, out: &mut Vec<Finding>) {
    let relative_churn =
        (meta.lines_added_90d + meta.lines_deleted_90d) as f64 / meta.nloc.max(1) as f64;
    if meta.commit_count_90d >= 5 && meta.churn_percentile >= 0.75 && relative_churn >= 1.0 {
        let severity = if relative_churn >= 4.0 && meta.is_hotspot {
            Severity::Critical
        } else if relative_churn >= 2.5 {
            Severity::High
        } else if relative_churn >= 1.5 {
            Severity::Medium
        } else {
            Severity::Low
        };
        out.push(finding(
            "churn_risk",
            severity,
            format!(
                "relative churn {:.2} at percentile {:.2} across {} commits",
                relative_churn, meta.churn_percentile, meta.commit_count_90d
            ),
        ));
    }
}

fn detect_co_change_scatter(meta: &GitMeta, out: &mut Vec<Finding>) {
    let scatter = meta
        .co_change_partners
        .iter()
        .filter(|p| p.co_change_count >= 2.0)
        .count();
    if scatter >= 8 && meta.commit_count_90d >= 3 {
        let severity = if scatter > 15 {
            Severity::High
        } else {
            Severity::Medium
        };
        out.push(finding(
            "co_change_scatter",
            severity,
            format!("co-changes scatter across {scatter} partners"),
        ));
    }
}

fn detect_code_age_volatility(meta: &GitMeta, out: &mut Vec<Finding>) {
    for function in &meta.functions {
        if function.median_age_days >= 365 && function.recent_mod_count > 2 {
            let severity = if function.median_age_days >= 730 && function.recent_mod_count > 5 {
                Severity::Critical
            } else if function.median_age_days >= 730 || function.recent_mod_count > 5 {
                Severity::High
            } else {
                Severity::Medium
            };
            out.push(finding(
                "code_age_volatility",
                severity,
                format!(
                    "{} is {} days old with {} recent modifications",
                    function.name, function.median_age_days, function.recent_mod_count
                ),
            ));
        }
    }
}

fn detect_developer_congestion(meta: &GitMeta, out: &mut Vec<Finding>) {
    let share = pct_to_share(meta.primary_owner_commit_pct);
    if meta.contributor_count >= 5 && meta.commit_count_90d >= 6 && share < 0.5 {
        let severity = if meta.contributor_count > 10 && meta.commit_count_90d > 20 {
            Severity::High
        } else if meta.contributor_count > 7 || meta.commit_count_90d > 12 {
            Severity::Medium
        } else {
            Severity::Low
        };
        out.push(finding(
            "developer_congestion",
            severity,
            format!(
                "{} contributors touched the file across {} commits with primary owner share {:.2}",
                meta.contributor_count, meta.commit_count_90d, share
            ),
        ));
    }
}

fn detect_function_hotspot(meta: &GitMeta, out: &mut Vec<Finding>) {
    let Some(p80) = meta.repo_function_mod_p80 else {
        return;
    };
    if p80 == 0 {
        return;
    }
    for function in &meta.functions {
        if function.modification_count >= p80 && (function.ccn >= 10 || function.max_nesting >= 3) {
            let severity = if function.modification_count >= 3 * p80 && function.ccn >= 20 {
                Severity::Critical
            } else if function.modification_count >= 2 * p80 || function.ccn >= 15 {
                Severity::High
            } else {
                Severity::Medium
            };
            out.push(finding(
                "function_hotspot",
                severity,
                format!(
                    "{} changed {} times with CCN {} and nesting {}",
                    function.name, function.modification_count, function.ccn, function.max_nesting
                ),
            ));
        }
    }
}

fn detect_hidden_coupling(meta: &GitMeta, out: &mut Vec<Finding>) {
    if meta.co_change_partners.is_empty() || meta.commit_count_total < 5 {
        return;
    }
    let file_is_test = is_test_path(&meta.file_path);
    let mut candidates = Vec::new();
    for partner in &meta.co_change_partners {
        if partner.path == meta.file_path {
            continue;
        }
        let partner_total = meta
            .repo_commit_counts
            .get(&partner.path)
            .copied()
            .unwrap_or(0);
        if partner_total < 5 {
            continue;
        }
        let correlation =
            partner.co_change_count / meta.commit_count_total.min(partner_total) as f64;
        if correlation <= 0.5 {
            continue;
        }
        if file_is_test ^ is_test_path(&partner.path) {
            continue;
        }
        if meta.import_edges.contains(&partner.path) {
            continue;
        }
        candidates.push((partner.path.as_str(), correlation));
    }
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (path, correlation) in candidates.into_iter().take(3) {
        let severity = if correlation >= 0.8 {
            Severity::Critical
        } else if correlation >= 0.65 {
            Severity::High
        } else {
            Severity::Medium
        };
        out.push(finding(
            "hidden_coupling",
            severity,
            format!("{path} co-changes with correlation {correlation:.2} without an import edge"),
        ));
    }
}

fn detect_knowledge_loss(meta: &GitMeta, out: &mut Vec<Finding>) {
    if meta.is_stable {
        return;
    }
    let is_hotspot_eff = meta.is_hotspot || meta.commit_count_90d >= 8;
    if meta.commit_count_90d < 1 && !is_hotspot_eff {
        return;
    }
    if meta.bus_factor != 1 || meta.primary_owner_name.is_empty() {
        return;
    }
    let recent_share = pct_to_share(meta.recent_owner_commit_pct);
    let primary_gone =
        meta.primary_owner_name != meta.recent_owner_name && !meta.recent_owner_name.is_empty();
    let recent_quiet = recent_share < 0.2;
    if primary_gone || recent_quiet {
        let mut severity = if is_hotspot_eff {
            Severity::High
        } else if primary_gone && recent_quiet {
            Severity::Medium
        } else {
            Severity::Low
        };
        let mut detail = format!(
            "primary owner {} has recent owner {} with recent share {:.2}",
            meta.primary_owner_name, meta.recent_owner_name, recent_share
        );
        if is_small_team(meta) && !is_hotspot_eff && severity != Severity::Low {
            severity = Severity::Low;
            detail.push_str(" (informational: small team)");
        }
        out.push(finding("knowledge_loss", severity, detail));
    }
}

fn detect_ownership_risk(meta: &GitMeta, out: &mut Vec<Finding>) {
    let total: u32 = meta.top_authors.iter().map(|(_, count)| *count).sum();
    if total < 5 {
        return;
    }
    let total_f = total as f64;
    let minor_contributors = meta
        .top_authors
        .iter()
        .filter(|(_, count)| *count as f64 / total_f < 0.05)
        .count();
    let top_owner_share = meta
        .top_authors
        .iter()
        .map(|(_, count)| *count as f64 / total_f)
        .fold(0.0, f64::max);
    if minor_contributors >= 3 || top_owner_share < 0.4 {
        let mut severity = if minor_contributors >= 6 && meta.is_hotspot {
            Severity::Critical
        } else if minor_contributors >= 5 || (minor_contributors >= 3 && meta.is_hotspot) {
            Severity::High
        } else if minor_contributors > 3 {
            Severity::Medium
        } else {
            Severity::Low
        };
        let mut detail = format!(
            "{minor_contributors} minor contributors with top owner share {top_owner_share:.2}"
        );
        if is_small_team(meta)
            && !meta.is_hotspot
            && severity_rank(severity) > severity_rank(Severity::Low)
        {
            severity = Severity::Low;
            detail.push_str(" (informational: small team)");
        }
        out.push(finding("ownership_risk", severity, detail));
    }
}

fn detect_prior_defect(meta: &GitMeta, out: &mut Vec<Finding>) {
    if meta.prior_defect_count < 1 {
        return;
    }
    let severity = if meta.prior_defect_count >= 5 {
        Severity::Critical
    } else if meta.prior_defect_count >= 3 {
        if meta.is_hotspot {
            Severity::Critical
        } else {
            Severity::High
        }
    } else if meta.prior_defect_count >= 2 {
        Severity::Medium
    } else {
        Severity::Low
    };
    out.push(finding(
        "prior_defect",
        severity,
        format!(
            "{} prior defects in window_days=180",
            meta.prior_defect_count
        ),
    ));
}

fn pct_to_share(value: f64) -> f64 {
    if value > 1.0 {
        value / 100.0
    } else {
        value
    }
}

fn is_small_team(meta: &GitMeta) -> bool {
    meta.repo_active_contributors_90d
        .map_or(false, |n| n <= SMALL_TEAM_MAX_CONTRIBUTORS)
}

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Info => 0,
        Severity::Low => 1,
        Severity::Medium => 2,
        Severity::High => 3,
        Severity::Critical => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_meta() -> GitMeta {
        GitMeta {
            file_path: "src/main.rs".to_string(),
            change_entropy: 0.0,
            change_entropy_pct: 0.0,
            commit_count_90d: 0,
            commit_count_total: 0,
            is_hotspot: false,
            is_stable: false,
            churn_percentile: 0.0,
            lines_added_90d: 0,
            lines_deleted_90d: 0,
            nloc: 100,
            contributor_count: 0,
            primary_owner_commit_pct: 1.0,
            primary_owner_name: String::new(),
            recent_owner_name: String::new(),
            recent_owner_commit_pct: 1.0,
            bus_factor: 0,
            prior_defect_count: 0,
            repo_active_contributors_90d: None,
            repo_function_mod_p80: None,
            co_change_partners: Vec::new(),
            top_authors: Vec::new(),
            functions: Vec::new(),
            repo_commit_counts: HashMap::new(),
            import_edges: HashSet::new(),
        }
    }

    fn only(meta: GitMeta, biomarker: &str) -> Vec<Finding> {
        git_biomarkers(&meta)
            .into_iter()
            .filter(|f| f.biomarker == biomarker)
            .collect()
    }

    #[test]
    fn change_entropy_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        assert!(only(m, "change_entropy").is_empty());
        m = base_meta();
        m.change_entropy = 1.0;
        m.change_entropy_pct = 0.80;
        m.commit_count_90d = 3;
        assert_eq!(only(m, "change_entropy")[0].severity, Severity::Medium);
        m = base_meta();
        m.change_entropy = 1.0;
        m.change_entropy_pct = 0.90;
        m.commit_count_90d = 3;
        assert_eq!(only(m, "change_entropy")[0].severity, Severity::High);
        m = base_meta();
        m.change_entropy = 1.0;
        m.change_entropy_pct = 0.95;
        m.commit_count_90d = 3;
        m.is_hotspot = true;
        assert_eq!(only(m, "change_entropy")[0].severity, Severity::Critical);
    }

    #[test]
    fn churn_risk_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        assert!(only(m, "churn_risk").is_empty());
        for (churn, hotspot, severity) in [
            (100, false, Severity::Low),
            (150, false, Severity::Medium),
            (250, false, Severity::High),
            (400, true, Severity::Critical),
        ] {
            m = base_meta();
            m.commit_count_90d = 5;
            m.churn_percentile = 0.75;
            m.lines_added_90d = churn;
            m.nloc = 100;
            m.is_hotspot = hotspot;
            assert_eq!(only(m, "churn_risk")[0].severity, severity);
        }
    }

    #[test]
    fn co_change_scatter_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        m.commit_count_90d = 3;
        m.co_change_partners = (0..7)
            .map(|i| CoChangePartner {
                path: format!("p{i}"),
                co_change_count: 2.0,
            })
            .collect();
        assert!(only(m, "co_change_scatter").is_empty());
        m = base_meta();
        m.commit_count_90d = 3;
        m.co_change_partners = (0..8)
            .map(|i| CoChangePartner {
                path: format!("p{i}"),
                co_change_count: 2.0,
            })
            .collect();
        assert_eq!(only(m, "co_change_scatter")[0].severity, Severity::Medium);
        m = base_meta();
        m.commit_count_90d = 3;
        m.co_change_partners = (0..16)
            .map(|i| CoChangePartner {
                path: format!("p{i}"),
                co_change_count: 2.0,
            })
            .collect();
        assert_eq!(only(m, "co_change_scatter")[0].severity, Severity::High);
    }

    #[test]
    fn code_age_volatility_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        m.functions.push(FunctionGitFacts {
            name: "old".to_string(),
            median_age_days: 365,
            recent_mod_count: 2,
            modification_count: 0,
            ccn: 0,
            max_nesting: 0,
        });
        assert!(only(m, "code_age_volatility").is_empty());
        for (age, mods, severity) in [
            (365, 3, Severity::Medium),
            (730, 3, Severity::High),
            (365, 6, Severity::High),
            (730, 6, Severity::Critical),
        ] {
            m = base_meta();
            m.functions.push(FunctionGitFacts {
                name: "old".to_string(),
                median_age_days: age,
                recent_mod_count: mods,
                modification_count: 0,
                ccn: 0,
                max_nesting: 0,
            });
            assert_eq!(only(m, "code_age_volatility")[0].severity, severity);
        }
    }

    #[test]
    fn developer_congestion_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        m.contributor_count = 5;
        m.commit_count_90d = 6;
        m.primary_owner_commit_pct = 0.4;
        assert_eq!(only(m, "developer_congestion")[0].severity, Severity::Low);
        for (contributors, commits, severity) in [
            (5, 7, Severity::Low),
            (8, 7, Severity::Medium),
            (5, 13, Severity::Medium),
            (11, 21, Severity::High),
        ] {
            m = base_meta();
            m.contributor_count = contributors;
            m.commit_count_90d = commits;
            m.primary_owner_commit_pct = 40.0;
            assert_eq!(only(m, "developer_congestion")[0].severity, severity);
        }
    }

    #[test]
    fn function_hotspot_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        m.repo_function_mod_p80 = Some(0);
        m.functions.push(FunctionGitFacts {
            name: "f".to_string(),
            median_age_days: 0,
            recent_mod_count: 0,
            modification_count: 10,
            ccn: 20,
            max_nesting: 0,
        });
        assert!(only(m, "function_hotspot").is_empty());
        for (mods, ccn, nesting, severity) in [
            (5, 10, 0, Severity::Medium),
            (10, 10, 0, Severity::High),
            (5, 15, 0, Severity::High),
            (15, 20, 0, Severity::Critical),
            (5, 0, 3, Severity::Medium),
        ] {
            m = base_meta();
            m.repo_function_mod_p80 = Some(5);
            m.functions.push(FunctionGitFacts {
                name: "f".to_string(),
                median_age_days: 0,
                recent_mod_count: 0,
                modification_count: mods,
                ccn,
                max_nesting: nesting,
            });
            assert_eq!(only(m, "function_hotspot")[0].severity, severity);
        }
    }

    #[test]
    fn hidden_coupling_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        m.commit_count_total = 4;
        m.co_change_partners.push(CoChangePartner {
            path: "src/a.rs".to_string(),
            co_change_count: 5.0,
        });
        assert!(only(m, "hidden_coupling").is_empty());
        for (count, severity) in [
            (6.0, Severity::Medium),
            (7.0, Severity::High),
            (8.0, Severity::Critical),
        ] {
            m = base_meta();
            m.commit_count_total = 10;
            m.repo_commit_counts.insert("src/a.rs".to_string(), 10);
            m.co_change_partners.push(CoChangePartner {
                path: "src/a.rs".to_string(),
                co_change_count: count,
            });
            assert_eq!(only(m, "hidden_coupling")[0].severity, severity);
        }
        m = base_meta();
        m.commit_count_total = 10;
        m.repo_commit_counts.insert("src/a.rs".to_string(), 10);
        m.import_edges.insert("src/a.rs".to_string());
        m.co_change_partners.push(CoChangePartner {
            path: "src/a.rs".to_string(),
            co_change_count: 8.0,
        });
        assert!(only(m, "hidden_coupling").is_empty());
    }

    #[test]
    fn hidden_coupling_keeps_tests_with_tests_across_js_ts_variants() {
        let mut m = base_meta();
        m.file_path = "src/foo.spec.tsx".to_string();
        m.commit_count_total = 10;
        m.repo_commit_counts
            .insert("src/foo.test.jsx".to_string(), 10);
        m.co_change_partners.push(CoChangePartner {
            path: "src/foo.test.jsx".to_string(),
            co_change_count: 8.0,
        });

        assert_eq!(only(m, "hidden_coupling")[0].severity, Severity::Critical);
    }

    #[test]
    fn knowledge_loss_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        m.bus_factor = 1;
        m.commit_count_90d = 1;
        assert!(only(m, "knowledge_loss").is_empty());
        for (gone, share, hotspot, severity) in [
            (false, 0.1, false, Severity::Low),
            (true, 0.1, false, Severity::Medium),
            (true, 0.3, true, Severity::High),
        ] {
            m = base_meta();
            m.bus_factor = 1;
            m.commit_count_90d = 1;
            m.primary_owner_name = "alice".to_string();
            m.recent_owner_name = if gone { "bob" } else { "alice" }.to_string();
            m.recent_owner_commit_pct = share;
            m.is_hotspot = hotspot;
            assert_eq!(only(m, "knowledge_loss")[0].severity, severity);
        }
        m = base_meta();
        m.bus_factor = 1;
        m.commit_count_90d = 1;
        m.primary_owner_name = "alice".to_string();
        m.recent_owner_name = "bob".to_string();
        m.recent_owner_commit_pct = 0.1;
        m.repo_active_contributors_90d = Some(3);
        let finding = only(m, "knowledge_loss").remove(0);
        assert_eq!(finding.severity, Severity::Low);
        assert!(finding.detail.contains("small team"));
    }

    #[test]
    fn ownership_risk_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        m.top_authors = vec![("a".to_string(), 4)];
        assert!(only(m, "ownership_risk").is_empty());
        m = base_meta();
        m.top_authors = vec![
            ("a".to_string(), 60),
            ("b".to_string(), 30),
            ("c".to_string(), 4),
            ("d".to_string(), 3),
            ("e".to_string(), 3),
        ];
        assert_eq!(only(m, "ownership_risk")[0].severity, Severity::Low);
        m = base_meta();
        m.top_authors = vec![
            ("a".to_string(), 84),
            ("b".to_string(), 4),
            ("c".to_string(), 4),
            ("d".to_string(), 4),
            ("e".to_string(), 4),
        ];
        assert_eq!(only(m, "ownership_risk")[0].severity, Severity::Medium);

        m = base_meta();
        m.top_authors = vec![
            ("a".to_string(), 76),
            ("b".to_string(), 4),
            ("c".to_string(), 4),
            ("d".to_string(), 4),
            ("e".to_string(), 4),
            ("f".to_string(), 4),
            ("g".to_string(), 4),
        ];
        assert_eq!(only(m, "ownership_risk")[0].severity, Severity::High);
        m = base_meta();
        m.top_authors = vec![
            ("a".to_string(), 76),
            ("b".to_string(), 4),
            ("c".to_string(), 4),
            ("d".to_string(), 4),
            ("e".to_string(), 4),
            ("f".to_string(), 4),
            ("g".to_string(), 4),
        ];
        m.is_hotspot = true;
        assert_eq!(only(m, "ownership_risk")[0].severity, Severity::Critical);
    }

    #[test]
    fn prior_defect_fire_silent_and_severity_ladder() {
        let mut m = base_meta();
        assert!(only(m, "prior_defect").is_empty());
        for (count, hotspot, severity) in [
            (1, false, Severity::Low),
            (2, false, Severity::Medium),
            (3, false, Severity::High),
            (3, true, Severity::Critical),
            (5, false, Severity::Critical),
        ] {
            m = base_meta();
            m.prior_defect_count = count;
            m.is_hotspot = hotspot;
            assert_eq!(only(m, "prior_defect")[0].severity, severity);
        }
    }
}
