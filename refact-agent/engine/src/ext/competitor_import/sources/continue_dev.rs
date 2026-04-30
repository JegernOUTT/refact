use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::{Mapping, Value as YamlValue};
use walkdir::WalkDir;

use super::super::converters::{convert_command_markdown, convert_skill_package, convert_subagent};
use super::super::markdown::{first_useful_line_or_heading, yaml_string};
use super::super::types::{
    Competitor, ConversionContext, ConversionError, ImportCandidate, ImportIssue, ImportKind,
    ImportScope, ImportStatus, ImportSummary, NormalizedSubagent, ToolPolicy,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContinueScanResult {
    pub candidates: Vec<ImportCandidate>,
    pub summary: ImportSummary,
}

impl ContinueScanResult {
    fn add_candidate(&mut self, candidate: ImportCandidate) {
        self.summary.record_candidate(&candidate);
        self.candidates.push(candidate);
    }

    fn add_issue(&mut self, issue: ImportIssue) {
        self.summary.add_issue(issue);
    }
}

struct ParsedMarkdown {
    frontmatter: YamlValue,
    body: String,
}

pub fn scan_global_root(home_dir: &Path, staging_root: &Path) -> ContinueScanResult {
    scan_continue_root(
        &home_dir.join(".continue"),
        ImportScope::Global,
        staging_root,
    )
}

pub fn scan_project_root(workspace_root: &Path, staging_root: &Path) -> ContinueScanResult {
    let scope = ImportScope::Project {
        root: workspace_root.to_path_buf(),
    };
    let mut result = scan_continue_root(
        &workspace_root.join(".continue"),
        scope.clone(),
        staging_root,
    );
    report_workspace_rule_files(workspace_root, &scope, &mut result);
    result
}

pub fn scan_continue_root(
    source_root: &Path,
    scope: ImportScope,
    staging_root: &Path,
) -> ContinueScanResult {
    let context = ConversionContext {
        competitor: Competitor::ContinueDev,
        scope,
        source_root: source_root.to_path_buf(),
    };
    let mut result = ContinueScanResult::default();
    if !source_root.is_dir() {
        return result;
    }

    scan_skills(&context, staging_root, &mut result);
    scan_prompts(&context, &mut result);
    scan_checks(&context, &mut result);
    report_continue_rule_files(&context, &mut result);
    result
}

fn scan_skills(context: &ConversionContext, staging_root: &Path, result: &mut ContinueScanResult) {
    let skills_root = context.source_root.join("skills");
    for skill_md in collect_named_files_at_depth(&skills_root, "SKILL.md", 2) {
        match read_parsed_markdown(&skill_md) {
            Ok(_) => {
                let Some(skill_dir) = skill_md.parent() else {
                    continue;
                };
                match convert_skill_package(context, skill_dir, staging_root) {
                    Ok(candidate) => result.add_candidate(candidate),
                    Err(err) => result.add_issue(err.into_issue()),
                }
            }
            Err(message) => result.add_issue(error_issue(
                context,
                ImportKind::Skill,
                &skill_md,
                format!("invalid Continue skill markdown: {message}"),
            )),
        }
    }
}

fn scan_prompts(context: &ConversionContext, result: &mut ContinueScanResult) {
    let prompts_root = context.source_root.join("prompts");
    for prompt_path in collect_markdown_files(&prompts_root) {
        match read_parsed_markdown(&prompt_path) {
            Ok((content, parsed)) => {
                if !yaml_bool_true(&parsed.frontmatter, "invokable") {
                    result.add_issue(unsupported_issue(
                        context,
                        ImportKind::Command,
                        &prompt_path,
                        "Continue prompt is not invokable; add invokable: true to import it as a command",
                    ));
                    continue;
                }
                let name = yaml_string(&parsed.frontmatter, "name");
                let fallback_name = relative_stem_name(&prompt_path, &prompts_root);
                let explicit_name = first_non_empty(&[name.as_str(), fallback_name.as_str()]);
                match convert_command_markdown(
                    context,
                    &prompt_path,
                    &content,
                    Some(&explicit_name),
                ) {
                    Ok(candidate) => result.add_candidate(candidate),
                    Err(err) => result.add_issue(err.into_issue()),
                }
            }
            Err(message) => result.add_issue(error_issue(
                context,
                ImportKind::Command,
                &prompt_path,
                format!("invalid Continue prompt markdown: {message}"),
            )),
        }
    }
}

fn scan_checks(context: &ConversionContext, result: &mut ContinueScanResult) {
    let checks_root = context.source_root.join("checks");
    for check_path in collect_markdown_files(&checks_root) {
        match read_parsed_markdown(&check_path) {
            Ok((_, parsed)) => {
                let fallback_name = relative_stem_name(&check_path, &checks_root);
                let name = yaml_string(&parsed.frontmatter, "name");
                let title = first_non_empty(&[name.as_str(), fallback_name.as_str()]);
                let frontmatter_description = yaml_string(&parsed.frontmatter, "description");
                let useful_line = first_useful_line_or_heading(&parsed.body).unwrap_or_default();
                let description = first_non_empty(&[
                    frontmatter_description.as_str(),
                    name.as_str(),
                    useful_line.as_str(),
                    title.as_str(),
                ]);
                let model = optional_yaml_string(&parsed.frontmatter, "model");
                let prompt = if parsed.body.trim().is_empty() {
                    description.clone()
                } else {
                    parsed.body.trim().to_string()
                };
                let input = NormalizedSubagent {
                    id: title.clone(),
                    title,
                    description,
                    prompt,
                    tool_policy: ToolPolicy::missing(),
                    max_steps: Some(10),
                    model,
                    metadata: serde_json::json!({"source": "continue_check"}),
                };
                match convert_subagent(context, &check_path, &input) {
                    Ok(candidate) => result.add_candidate(candidate),
                    Err(err) => result.add_issue(err.into_issue()),
                }
            }
            Err(message) => result.add_issue(error_issue(
                context,
                ImportKind::Subagent,
                &check_path,
                format!("invalid Continue check markdown: {message}"),
            )),
        }
    }
}

fn report_continue_rule_files(context: &ConversionContext, result: &mut ContinueScanResult) {
    let rules_root = context.source_root.join("rules");
    for path in collect_markdown_files(&rules_root) {
        result.add_issue(unsupported_issue(
            context,
            ImportKind::UnsupportedRules,
            &path,
            "Continue rules are report-only in v1",
        ));
    }
    let root_rules = context.source_root.join("rules.md");
    if root_rules.is_file() {
        result.add_issue(unsupported_issue(
            context,
            ImportKind::UnsupportedRules,
            &root_rules,
            "Continue rules are report-only in v1",
        ));
    }
}

fn report_workspace_rule_files(
    workspace_root: &Path,
    scope: &ImportScope,
    result: &mut ContinueScanResult,
) {
    for file_name in ["rules.md", "AGENTS.md", "AGENT.md", "CLAUDE.md"] {
        let path = workspace_root.join(file_name);
        if path.is_file() {
            result.add_issue(ImportIssue {
                competitor: Some(Competitor::ContinueDev),
                kind: Some(ImportKind::UnsupportedRules),
                scope: Some(scope.clone()),
                path: Some(path),
                status: ImportStatus::Unsupported,
                message: "Continue rules and instruction files are report-only in v1".to_string(),
            });
        }
    }
}

fn read_parsed_markdown(path: &Path) -> Result<(String, ParsedMarkdown), String> {
    let content = fs::read_to_string(path).map_err(|err| format!("failed to read file: {err}"))?;
    let parsed = parse_markdown_with_errors(&content)?;
    Ok((content, parsed))
}

fn parse_markdown_with_errors(content: &str) -> Result<ParsedMarkdown, String> {
    let normalized = content.replace("\r\n", "\n");
    let empty = YamlValue::Mapping(Mapping::new());
    if !normalized.starts_with("---\n") {
        return Ok(ParsedMarkdown {
            frontmatter: empty,
            body: content.to_string(),
        });
    }

    let rest = &normalized[4..];
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches('\n');
        if line_without_newline.trim() == "---" {
            let frontmatter_text = &rest[..offset];
            let body_start = offset + line.len();
            let body = rest.get(body_start..).unwrap_or_default().to_string();
            let frontmatter = parse_frontmatter_mapping(frontmatter_text)?;
            return Ok(ParsedMarkdown { frontmatter, body });
        }
        offset += line.len();
    }

    if rest.trim() == "---" {
        return Ok(ParsedMarkdown {
            frontmatter: empty,
            body: String::new(),
        });
    }

    Err("missing closing frontmatter marker".to_string())
}

fn parse_frontmatter_mapping(frontmatter_text: &str) -> Result<YamlValue, String> {
    if frontmatter_text.trim().is_empty() {
        return Ok(YamlValue::Mapping(Mapping::new()));
    }
    match serde_yaml::from_str::<YamlValue>(frontmatter_text) {
        Ok(YamlValue::Null) => Ok(YamlValue::Mapping(Mapping::new())),
        Ok(value @ YamlValue::Mapping(_)) => Ok(value),
        Ok(_) => Err("frontmatter must be a mapping".to_string()),
        Err(err) => Err(err.to_string()),
    }
}

fn collect_markdown_files(root: &Path) -> Vec<PathBuf> {
    collect_files(root, |path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    })
}

fn collect_named_files_at_depth(root: &Path, file_name: &str, depth: usize) -> Vec<PathBuf> {
    collect_files(root, |path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == file_name)
            && path
                .strip_prefix(root)
                .map(|relative| relative.components().count() == depth)
                .unwrap_or(false)
    })
}

fn collect_files(root: &Path, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    if !root.is_dir() {
        return Vec::new();
    }
    WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| keep(path))
        .collect()
}

fn optional_yaml_string(frontmatter: &YamlValue, key: &str) -> Option<String> {
    let value = yaml_string(frontmatter, key);
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn yaml_bool_true(frontmatter: &YamlValue, key: &str) -> bool {
    match frontmatter.get(key) {
        Some(YamlValue::Bool(value)) => *value,
        Some(YamlValue::String(value)) => value.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn relative_stem_name(path: &Path, root: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
}

fn first_non_empty(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

fn error_issue(
    context: &ConversionContext,
    kind: ImportKind,
    path: &Path,
    message: impl Into<String>,
) -> ImportIssue {
    ConversionError::new(context, kind, path.to_path_buf(), message).into_issue()
}

fn unsupported_issue(
    context: &ConversionContext,
    kind: ImportKind,
    path: &Path,
    message: impl Into<String>,
) -> ImportIssue {
    ImportIssue {
        competitor: Some(context.competitor),
        kind: Some(kind),
        scope: Some(context.scope.clone()),
        path: Some(path.to_path_buf()),
        status: ImportStatus::Unsupported,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml_configs::customization_types::SubagentConfig;
    use super::super::super::types::ImportArtifact;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn continue_skill_imports_as_skill() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let skill_dir = workspace.join(".continue/skills/foo");
        write(
            &skill_dir.join("SKILL.md"),
            "---\nname: foo\ndescription: Foo skill\n---\nUse foo.",
        );
        write(&skill_dir.join("notes.txt"), "notes");

        let result = scan_project_root(&workspace, &temp.path().join("staging"));

        assert!(result.summary.errors.is_empty());
        assert_eq!(result.candidates.len(), 1);
        let candidate = &result.candidates[0];
        assert_eq!(candidate.kind, ImportKind::Skill);
        assert_eq!(candidate.destination_path, PathBuf::from("skills/foo"));
    }

    #[test]
    fn invokable_continue_prompt_imports_as_command() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let prompt_path = workspace.join(".continue/prompts/review.md");
        write(
            &prompt_path,
            "---\nname: Review Code\ndescription: Review changes\ninvokable: true\n---\nReview the code.",
        );

        let result = scan_project_root(&workspace, &temp.path().join("staging"));

        assert!(result.summary.errors.is_empty());
        assert_eq!(result.candidates.len(), 1);
        let candidate = &result.candidates[0];
        assert_eq!(candidate.kind, ImportKind::Command);
        assert_eq!(
            candidate.destination_path,
            PathBuf::from("commands/review-code.md")
        );
        let ImportArtifact::FileContent { content } = &candidate.artifact else {
            panic!("expected file content");
        };
        assert!(!content.contains("invokable"));
        assert!(!content.contains("name:"));
    }

    #[test]
    fn non_invokable_continue_prompt_is_unsupported() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let prompt_path = workspace.join(".continue/prompts/rule.md");
        write(&prompt_path, "---\ndescription: Rule\n---\nAlways do this.");

        let result = scan_project_root(&workspace, &temp.path().join("staging"));

        assert!(result.candidates.is_empty());
        assert!(result.summary.errors.is_empty());
        assert_eq!(result.summary.issues.len(), 1);
        assert_eq!(result.summary.issues[0].status, ImportStatus::Unsupported);
        assert_eq!(result.summary.issues[0].kind, Some(ImportKind::Command));
    }

    #[test]
    fn continue_check_imports_as_valid_subagent() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let check_path = workspace.join(".continue/checks/security.md");
        write(
            &check_path,
            "---\nname: Security Review\ndescription: Review security issues\n---\n# Security check\nFind vulnerabilities.",
        );

        let result = scan_project_root(&workspace, &temp.path().join("staging"));

        assert!(result.summary.errors.is_empty());
        assert_eq!(result.candidates.len(), 1);
        let candidate = &result.candidates[0];
        assert_eq!(candidate.kind, ImportKind::Subagent);
        assert_eq!(
            candidate.destination_path,
            PathBuf::from("subagents/security-review.yaml")
        );
        let ImportArtifact::FileContent { content } = &candidate.artifact else {
            panic!("expected file content");
        };
        let config: SubagentConfig = serde_yaml::from_str(content).unwrap();
        assert_eq!(config.id, "security-review");
        assert_eq!(config.description, "Review security issues");
        assert_eq!(config.subchat.max_steps, Some(10));
        assert_eq!(config.tools, strings(&["tree", "cat", "search_pattern"]));
    }

    #[test]
    fn continue_rules_are_reported_unsupported_without_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        write(
            &workspace.join(".continue/rules/security.md"),
            "# Security rules",
        );
        write(&workspace.join("rules.md"), "# Codebase rules");

        let result = scan_project_root(&workspace, &temp.path().join("staging"));

        assert!(result.candidates.is_empty());
        assert!(result.summary.errors.is_empty());
        assert_eq!(result.summary.issues.len(), 2);
        assert!(result
            .summary
            .issues
            .iter()
            .all(|issue| issue.status == ImportStatus::Unsupported));
        assert!(result
            .summary
            .issues
            .iter()
            .all(|issue| issue.kind == Some(ImportKind::UnsupportedRules)));
    }

    #[test]
    fn invalid_check_frontmatter_reports_error_and_continues() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        write(
            &workspace.join(".continue/checks/bad.md"),
            "---\nname: [bad\n---\n# Bad check",
        );
        write(
            &workspace.join(".continue/checks/good.md"),
            "---\nname: Good Check\n---\n# Good check",
        );

        let result = scan_project_root(&workspace, &temp.path().join("staging"));

        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.summary.errors.len(), 1);
        assert_eq!(result.summary.errors[0].status, ImportStatus::Error);
        assert_eq!(result.summary.errors[0].kind, Some(ImportKind::Subagent));
        assert_eq!(
            result.candidates[0].destination_path,
            PathBuf::from("subagents/good-check.yaml")
        );
    }
}
