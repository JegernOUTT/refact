use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use super::opencode::{scan_compatible_roots_with_filter, CompatibleScanRoots, OpenCodeScan};
use super::super::types::{
    Competitor, ConversionContext, ImportIssue, ImportKind, ImportPrivacyFilter, ImportScope,
    ImportStatus, ImportSummary,
};
use super::super::writer::write_candidates;

pub type KiloScan = OpenCodeScan;

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

pub async fn import_project_root(workspace_root: &Path) -> ImportSummary {
    let scan = scan_project_root(workspace_root);
    let mut summary = issues_summary(&scan);
    let write_summary = write_candidates(&workspace_root.join(".refact"), &scan.candidates).await;
    summary.merge(write_summary);
    summary
}

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
    for path in [
        workspace_root.join("AGENTS.md"),
        workspace_root.join("AGENT.md"),
        workspace_root.join("CLAUDE.md"),
        workspace_root.join("CONTEXT.md"),
    ] {
        if is_regular_file(&path) {
            scan.issues.push(unsupported_rule_issue(context, &path));
        }
    }
    for root in [
        workspace_root.join(".kilo"),
        workspace_root.join(".kilocode"),
    ] {
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
        report_rule_root(scan, context, &root.join("rules"));
        report_prefixed_rule_dirs(scan, context, &root, "rules-");
    }
}

fn report_global_rule_files(scan: &mut KiloScan, context: &ConversionContext, source_root: &Path) {
    for path in [source_root.join("AGENTS.md"), source_root.join("AGENT.md")] {
        if is_regular_file(&path) {
            scan.issues.push(unsupported_rule_issue(context, &path));
        }
    }
    report_rule_root(scan, context, &source_root.join("rules"));
    report_prefixed_rule_dirs(scan, context, source_root, "rules-");
}

fn report_prefixed_rule_dirs(
    scan: &mut KiloScan,
    context: &ConversionContext,
    root: &Path,
    prefix: &str,
) {
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
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with(prefix) {
            report_rule_root(scan, context, &path);
        }
    }
}

fn report_rule_root(scan: &mut KiloScan, context: &ConversionContext, rule_root: &Path) {
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
    for entry in walkdir::WalkDir::new(rule_root)
        .follow_links(false)
        .sort_by_file_name()
        .min_depth(1)
    {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.file_type().is_file() {
            scan.issues
                .push(unsupported_rule_issue(context, entry.path()));
        }
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

    #[test]
    fn kilo_skill_package_imports_as_skill() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join(".kilo/skills/foo/SKILL.md"),
            "---\nname: foo\ndescription: Foo skill\n---\nUse foo.",
        );

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Skill, "foo");
        assert_eq!(candidate.destination_path, PathBuf::from("skills/foo"));
        assert_eq!(candidate.competitor, Competitor::KiloCode);
    }

    #[test]
    fn kilo_command_imports_as_command() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join(".kilo/commands/review.md"),
            "---\ndescription: Review code\nagent: code\nmodel: claude\nsubtask: true\n---\nReview the diff.",
        );

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Command, "review");
        assert_eq!(
            candidate.destination_path,
            PathBuf::from("commands/review.md")
        );
        let (frontmatter, body) = parse_markdown_frontmatter(file_content(candidate));
        assert_eq!(yaml_string(&frontmatter, "description"), "Review code");
        assert_eq!(yaml_string(&frontmatter, "model"), "claude");
        assert!(frontmatter.get("agent").is_none());
        assert!(frontmatter.get("subtask").is_none());
        assert_eq!(body, "Review the diff.");
        assert!(candidate.metadata.get("competitor_fields").is_some());
    }

    #[test]
    fn legacy_workflow_imports_as_command() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join(".kilocode/workflows/deploy.md"),
            "Deploy to staging.",
        );

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Command, "deploy");
        assert_eq!(
            candidate.destination_path,
            PathBuf::from("commands/deploy.md")
        );
        assert!(scan
            .candidates
            .iter()
            .all(|candidate| candidate.kind != ImportKind::Subagent));
    }

    #[test]
    fn markdown_agent_imports_as_schema_valid_subagent() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join(".kilo/agents/docs-writer.md"),
            "---\ndescription: Writes docs\nmode: subagent\nmodel: claude\npermission:\n  bash: deny\nsteps: 7\n---\nWrite clear documentation.",
        );

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Subagent, "docs-writer");
        let config = subagent_config(candidate);
        assert_eq!(config.id, "docs-writer");
        assert_eq!(config.description, "Writes docs");
        assert_eq!(config.subchat.max_steps, Some(7));
        assert_eq!(config.subchat.model.as_deref(), Some("claude"));
        assert!(!config.tools.contains(&"shell".to_string()));
    }

    #[test]
    fn kilo_json_agent_entry_imports_best_effort() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join("kilo.json"),
            &serde_json::json!({
                "agent": {
                    "test-gen": {
                        "description": "Generates tests",
                        "mode": "subagent",
                        "prompt": "Write comprehensive tests.",
                        "permission": { "edit": "allow", "bash": "deny" },
                        "steps": 5,
                        "model": "claude"
                    }
                }
            })
            .to_string(),
        );

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Subagent, "test-gen");
        let config = subagent_config(candidate);
        assert_eq!(config.id, "test-gen");
        assert_eq!(config.description, "Generates tests");
        assert_eq!(config.subchat.max_steps, Some(5));
        assert_eq!(config.subchat.model.as_deref(), Some("claude"));
        assert!(config.tools.contains(&"apply_patch".to_string()));
        assert!(!config.tools.contains(&"shell".to_string()));
    }

    #[test]
    fn invalid_kilo_jsonc_is_non_fatal_for_markdown_imports() {
        let temp = tempfile::tempdir().unwrap();
        write(&temp.path().join("kilo.jsonc"), "{ agent: [ }");
        write(
            &temp.path().join(".kilo/commands/review.md"),
            "Review anyway.",
        );

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        find_candidate(&scan, ImportKind::Command, "review");
        assert_eq!(scan.issues.len(), 1);
        assert_eq!(scan.issues[0].status, ImportStatus::Error);
    }

    #[cfg(unix)]
    #[test]
    fn project_symlinked_kilo_roots_outside_workspace_are_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let outside_kilo = temp.path().join("outside-kilo");
        let outside_legacy = temp.path().join("outside-kilocode");
        write(
            &outside_kilo.join("skills/foo/SKILL.md"),
            "---\nname: foo\n---\nUse foo.",
        );
        write(&outside_kilo.join("commands/review.md"), "Review.");
        write(&outside_kilo.join("agents/reviewer.md"), "Review.");
        write(&outside_legacy.join("workflows/deploy.md"), "Deploy.");
        fs::create_dir_all(&workspace).unwrap();
        std::os::unix::fs::symlink(&outside_kilo, workspace.join(".kilo")).unwrap();
        std::os::unix::fs::symlink(&outside_legacy, workspace.join(".kilocode")).unwrap();
        let staging = temp.path().join("staging");

        let scan = scan_project_root_with_staging(&workspace, &staging);

        assert!(scan.candidates.is_empty());
        assert!(scan
            .issues
            .iter()
            .any(|issue| issue.status == ImportStatus::Unsupported));
        assert!(!staging.exists());
    }

    #[test]
    fn rule_like_files_are_reported_without_import_candidates() {
        let temp = tempfile::tempdir().unwrap();
        write(&temp.path().join(".kilo/rules/formatting.md"), "# Rules");
        write(
            &temp.path().join(".kilocode/rules/security.md"),
            "# Security",
        );
        write(&temp.path().join("AGENTS.md"), "# Instructions");

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        assert!(scan.candidates.is_empty());
        assert_eq!(scan.issues.len(), 3);
        assert!(scan
            .issues
            .iter()
            .all(|issue| issue.kind == Some(ImportKind::UnsupportedRules)));
    }

    #[test]
    fn global_scan_includes_config_home_and_legacy_roots_without_deduping() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let config = temp.path().join("config");
        write(
            &config.join("kilo/commands/review.md"),
            "Review from config.",
        );
        write(&home.join(".kilo/commands/review.md"), "Review from home.");
        write(&home.join(".kilocode/workflows/deploy.md"), "Deploy.");

        let scan = scan_global_root_with_staging(&home, &config, &temp.path().join("staging"));

        assert_eq!(
            scan.candidates
                .iter()
                .filter(|candidate| candidate.dest_name == "review")
                .count(),
            2
        );
        find_candidate(&scan, ImportKind::Command, "deploy");
        assert!(scan
            .candidates
            .iter()
            .all(|candidate| candidate.scope == ImportScope::Global));
    }

    #[tokio::test]
    async fn import_project_root_writes_candidates_through_shared_writer() {
        let temp = tempfile::tempdir().unwrap();
        write(&temp.path().join(".kilo/commands/review.md"), "Review.");

        let summary = import_project_root(temp.path()).await;

        assert_eq!(summary.outcomes.len(), 1);
        assert_eq!(
            fs::read_to_string(temp.path().join(".refact/commands/review.md")).unwrap(),
            "Review."
        );
    }
}
