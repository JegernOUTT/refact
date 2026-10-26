use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use super::opencode::{scan_compatible_roots_with_filter, CompatibleScanRoots, OpenCodeScan};
use super::super::manifest::{MAX_SCAN_DEPTH, MAX_SCAN_ENTRIES, MAX_UNSUPPORTED_RULE_REPORTS};
use super::super::types::{
    Competitor, ConversionContext, ImportIssue, ImportKind, ImportPrivacyFilter, ImportScope,
    ImportStatus,
};
#[cfg(test)]
use super::super::types::ImportSummary;
#[cfg(test)]
use super::super::writer::write_candidates;

pub type KiloScan = OpenCodeScan;

#[cfg(test)]
pub fn scan_project_root(workspace_root: &Path) -> KiloScan {
    scan_project_root_with_filter(workspace_root, &ImportPrivacyFilter::allow_all())
}

pub(crate) fn scan_project_root_with_filter(
    workspace_root: &Path,
    filter: &ImportPrivacyFilter,
) -> KiloScan {
    let staging_root = workspace_root
        .join(".refact")
        .join("imports")
        .join("staging")
        .join("kilo");
    scan_project_root_with_staging_and_filter(workspace_root, &staging_root, filter)
}

#[cfg(test)]
pub fn scan_project_root_with_staging(workspace_root: &Path, staging_root: &Path) -> KiloScan {
    scan_project_root_with_staging_and_filter(
        workspace_root,
        staging_root,
        &ImportPrivacyFilter::allow_all(),
    )
}

pub(crate) fn scan_project_root_with_staging_and_filter(
    workspace_root: &Path,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
) -> KiloScan {
    let context = ConversionContext {
        competitor: Competitor::KiloCode,
        scope: ImportScope::Project {
            root: workspace_root.to_path_buf(),
        },
        source_root: workspace_root.to_path_buf(),
    };
    let mut scan = scan_compatible_roots_with_filter(
        &context,
        CompatibleScanRoots {
            display_name: "Kilo Code",
            skill_roots: vec![
                workspace_root.join(".kilo").join("skills"),
                workspace_root.join(".kilocode").join("skills"),
            ],
            command_roots: vec![
                workspace_root.join(".kilo").join("commands"),
                workspace_root.join(".kilocode").join("workflows"),
            ],
            agent_roots: vec![
                workspace_root.join(".kilo").join("agents"),
                workspace_root.join(".kilo").join("agent"),
            ],
            config_files: vec![
                workspace_root.join("kilo.json"),
                workspace_root.join("kilo.jsonc"),
            ],
        },
        staging_root,
        filter,
    );
    report_project_rule_files(&mut scan, &context, workspace_root);
    scan
}

#[cfg(test)]
#[allow(dead_code)]
pub fn scan_global_root(
    home_dir: &Path,
    config_root: &Path,
    refact_config_root: &Path,
) -> KiloScan {
    scan_global_root_with_filter(
        home_dir,
        config_root,
        refact_config_root,
        &ImportPrivacyFilter::allow_all(),
    )
}

pub(crate) fn scan_global_root_with_filter(
    home_dir: &Path,
    config_root: &Path,
    refact_config_root: &Path,
    filter: &ImportPrivacyFilter,
) -> KiloScan {
    let staging_root = refact_config_root
        .join("imports")
        .join("staging")
        .join("kilo");
    scan_global_root_with_staging_and_filter(home_dir, config_root, &staging_root, filter)
}

#[cfg(test)]
pub fn scan_global_root_with_staging(
    home_dir: &Path,
    config_root: &Path,
    staging_root: &Path,
) -> KiloScan {
    scan_global_root_with_staging_and_filter(
        home_dir,
        config_root,
        staging_root,
        &ImportPrivacyFilter::allow_all(),
    )
}

pub(crate) fn scan_global_root_with_staging_and_filter(
    home_dir: &Path,
    config_root: &Path,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
) -> KiloScan {
    let mut scan = KiloScan::default();
    append_scan(
        &mut scan,
        scan_global_config_root(&config_root.join("kilo"), staging_root, filter),
    );
    append_scan(
        &mut scan,
        scan_global_home_kilo_root(&home_dir.join(".kilo"), staging_root, filter),
    );
    append_scan(
        &mut scan,
        scan_global_legacy_root(&home_dir.join(".kilocode"), staging_root, filter),
    );
    scan
}

#[cfg(test)]
pub async fn import_project_root(workspace_root: &Path) -> ImportSummary {
    let scan = scan_project_root(workspace_root);
    let mut summary = issues_summary(&scan);
    let write_summary = write_candidates(&workspace_root.join(".refact"), &scan.candidates).await;
    summary.merge(write_summary);
    summary
}

#[cfg(test)]
#[allow(dead_code)]
pub async fn import_global_root(
    home_dir: &Path,
    config_root: &Path,
    refact_config_root: &Path,
) -> ImportSummary {
    let scan = scan_global_root(home_dir, config_root, refact_config_root);
    let mut summary = issues_summary(&scan);
    let write_summary = write_candidates(refact_config_root, &scan.candidates).await;
    summary.merge(write_summary);
    summary
}

fn scan_global_config_root(
    config_kilo_root: &Path,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
) -> KiloScan {
    let context = ConversionContext {
        competitor: Competitor::KiloCode,
        scope: ImportScope::Global,
        source_root: config_kilo_root.to_path_buf(),
    };
    let mut scan = scan_compatible_roots_with_filter(
        &context,
        CompatibleScanRoots {
            display_name: "Kilo Code",
            skill_roots: vec![config_kilo_root.join("skills")],
            command_roots: vec![config_kilo_root.join("commands")],
            agent_roots: vec![config_kilo_root.join("agents")],
            config_files: vec![
                config_kilo_root.join("kilo.json"),
                config_kilo_root.join("kilo.jsonc"),
            ],
        },
        staging_root,
        filter,
    );
    report_global_rule_files(&mut scan, &context, config_kilo_root);
    scan
}

fn scan_global_home_kilo_root(
    home_kilo_root: &Path,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
) -> KiloScan {
    let context = ConversionContext {
        competitor: Competitor::KiloCode,
        scope: ImportScope::Global,
        source_root: home_kilo_root.to_path_buf(),
    };
    let mut scan = scan_compatible_roots_with_filter(
        &context,
        CompatibleScanRoots {
            display_name: "Kilo Code",
            skill_roots: vec![home_kilo_root.join("skills")],
            command_roots: vec![home_kilo_root.join("commands")],
            agent_roots: vec![home_kilo_root.join("agents")],
            config_files: Vec::new(),
        },
        staging_root,
        filter,
    );
    report_global_rule_files(&mut scan, &context, home_kilo_root);
    scan
}

fn scan_global_legacy_root(
    legacy_root: &Path,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
) -> KiloScan {
    let context = ConversionContext {
        competitor: Competitor::KiloCode,
        scope: ImportScope::Global,
        source_root: legacy_root.to_path_buf(),
    };
    let mut scan = scan_compatible_roots_with_filter(
        &context,
        CompatibleScanRoots {
            display_name: "Kilo Code",
            skill_roots: vec![legacy_root.join("skills")],
            command_roots: vec![legacy_root.join("workflows")],
            agent_roots: Vec::new(),
            config_files: Vec::new(),
        },
        staging_root,
        filter,
    );
    report_global_rule_files(&mut scan, &context, legacy_root);
    scan
}

fn append_scan(scan: &mut KiloScan, mut other: KiloScan) {
    scan.candidates.append(&mut other.candidates);
    scan.issues.append(&mut other.issues);
}

#[cfg(test)]
fn issues_summary(scan: &KiloScan) -> ImportSummary {
    let mut summary = ImportSummary::default();
    for issue in &scan.issues {
        summary.add_issue(issue.clone());
    }
    summary
}

fn report_project_rule_files(
    scan: &mut KiloScan,
    context: &ConversionContext,
    workspace_root: &Path,
) {
    let mut limiter = RuleReportLimiter::default();
    for path in [
        workspace_root.join("AGENTS.md"),
        workspace_root.join("AGENT.md"),
        workspace_root.join("CLAUDE.md"),
        workspace_root.join("CONTEXT.md"),
    ] {
        if is_regular_file(&path) {
            limiter.push_rule(scan, context, &path);
        }
    }
    for root in [
        workspace_root.join(".kilo"),
        workspace_root.join(".kilocode"),
    ] {
        if limiter.is_capped() {
            return;
        }
        match super::project_scan_root_allowed(&root, workspace_root) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(message) => {
                scan.issues.push(super::skipped_root_issue(
                    context,
                    Some(ImportKind::UnsupportedRules),
                    &root,
                    message,
                ));
                continue;
            }
        }
        report_rule_root(scan, context, &root.join("rules"), &mut limiter);
        report_prefixed_rule_dirs(scan, context, &root, "rules-", &mut limiter);
    }
}

fn report_global_rule_files(scan: &mut KiloScan, context: &ConversionContext, source_root: &Path) {
    let mut limiter = RuleReportLimiter::default();
    for path in [source_root.join("AGENTS.md"), source_root.join("AGENT.md")] {
        if is_regular_file(&path) {
            limiter.push_rule(scan, context, &path);
        }
    }
    report_rule_root(scan, context, &source_root.join("rules"), &mut limiter);
    report_prefixed_rule_dirs(scan, context, source_root, "rules-", &mut limiter);
}

fn report_prefixed_rule_dirs(
    scan: &mut KiloScan,
    context: &ConversionContext,
    root: &Path,
    prefix: &str,
    limiter: &mut RuleReportLimiter,
) {
    if limiter.is_capped() {
        return;
    }
    match super::scan_root_allowed(context, root) {
        Ok(true) => {}
        Ok(false) => return,
        Err(message) => {
            scan.issues.push(super::skipped_root_issue(
                context,
                Some(ImportKind::UnsupportedRules),
                root,
                message,
            ));
            return;
        }
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return,
        Err(_) => return,
    };
    let mut entry_count = 0usize;
    for entry in entries.filter_map(Result::ok) {
        if limiter.is_capped() {
            return;
        }
        entry_count += 1;
        if entry_count > MAX_SCAN_ENTRIES {
            limiter.push_entry_cap(scan, context, root);
            return;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with(prefix) {
            report_rule_root(scan, context, &path, limiter);
        }
    }
}

fn report_rule_root(
    scan: &mut KiloScan,
    context: &ConversionContext,
    rule_root: &Path,
    limiter: &mut RuleReportLimiter,
) {
    if limiter.is_capped() {
        return;
    }
    match super::scan_root_allowed(context, rule_root) {
        Ok(true) => {}
        Ok(false) => return,
        Err(message) => {
            scan.issues.push(super::skipped_root_issue(
                context,
                Some(ImportKind::UnsupportedRules),
                rule_root,
                message,
            ));
            return;
        }
    }
    let mut entry_count = 0usize;
    let mut entries = walkdir::WalkDir::new(rule_root)
        .follow_links(false)
        .sort_by_file_name()
        .min_depth(1)
        .max_depth(MAX_SCAN_DEPTH + 1)
        .into_iter();
    while let Some(entry) = entries.next() {
        let Ok(entry) = entry else {
            continue;
        };
        entry_count += 1;
        if entry_count > MAX_SCAN_ENTRIES {
            limiter.push_entry_cap(scan, context, rule_root);
            break;
        }
        if entry.depth() > MAX_SCAN_DEPTH {
            limiter.push_cap(scan, context, rule_root);
            if entry.file_type().is_dir() {
                entries.skip_current_dir();
            }
            break;
        }
        if entry.file_type().is_file() {
            limiter.push_rule(scan, context, entry.path());
            if limiter.is_capped() {
                break;
            }
        }
    }
}

#[derive(Default)]
struct RuleReportLimiter {
    reported: usize,
    capped: bool,
}

impl RuleReportLimiter {
    fn is_capped(&self) -> bool {
        self.capped
    }

    fn push_rule(&mut self, scan: &mut KiloScan, context: &ConversionContext, path: &Path) {
        if self.reported >= MAX_UNSUPPORTED_RULE_REPORTS {
            self.push_cap(scan, context, path);
            return;
        }
        scan.issues.push(unsupported_rule_issue(context, path));
        self.reported += 1;
    }

    fn push_cap(&mut self, scan: &mut KiloScan, context: &ConversionContext, path: &Path) {
        if self.capped {
            return;
        }
        self.capped = true;
        scan.issues.push(unsupported_rule_cap_issue(context, path));
    }

    fn push_entry_cap(&mut self, scan: &mut KiloScan, context: &ConversionContext, path: &Path) {
        if self.capped {
            return;
        }
        self.capped = true;
        scan.issues.push(ImportIssue {
            competitor: Some(context.competitor),
            kind: Some(ImportKind::UnsupportedRules),
            scope: Some(context.scope.clone()),
            path: Some(path.to_path_buf()),
            status: ImportStatus::Unsupported,
            message: format!(
                "Kilo Code rules scan capped after {MAX_SCAN_ENTRIES} filesystem entries"
            ),
        });
    }
}

fn unsupported_rule_issue(context: &ConversionContext, path: &Path) -> ImportIssue {
    ImportIssue {
        competitor: Some(context.competitor),
        kind: Some(ImportKind::UnsupportedRules),
        scope: Some(context.scope.clone()),
        path: Some(path.to_path_buf()),
        status: ImportStatus::Unsupported,
        message: "Kilo Code rules and instruction files are report-only in v1".to_string(),
    }
}

fn unsupported_rule_cap_issue(context: &ConversionContext, path: &Path) -> ImportIssue {
    ImportIssue {
        competitor: Some(context.competitor),
        kind: Some(ImportKind::UnsupportedRules),
        scope: Some(context.scope.clone()),
        path: Some(path.to_path_buf()),
        status: ImportStatus::Unsupported,
        message: format!(
            "Kilo Code rules scan capped after {MAX_UNSUPPORTED_RULE_REPORTS} reports"
        ),
    }
}

fn is_regular_file(path: &Path) -> bool {
    super::regular_file_exists(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::ext::competitor_import::markdown::{parse_markdown_frontmatter, yaml_string};
    use crate::ext::competitor_import::types::{ImportArtifact, ImportCandidate};
    use crate::yaml_configs::customization_types::SubagentConfig;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn file_content(candidate: &ImportCandidate) -> &str {
        match &candidate.artifact {
            ImportArtifact::FileContent { content } => content,
            _ => panic!("expected file content"),
        }
    }

    fn subagent_config(candidate: &ImportCandidate) -> SubagentConfig {
        serde_yaml::from_str(file_content(candidate)).unwrap()
    }

    fn find_candidate<'a>(
        scan: &'a KiloScan,
        kind: ImportKind,
        dest_name: &str,
    ) -> &'a ImportCandidate {
        scan.candidates
            .iter()
            .find(|candidate| candidate.kind == kind && candidate.dest_name == dest_name)
            .unwrap_or_else(|| panic!("missing {kind:?} candidate {dest_name}"))
    }
}
