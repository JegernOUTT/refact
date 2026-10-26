use std::path::{Path, PathBuf};

use serde_yaml::{Mapping, Value as YamlValue};
use walkdir::WalkDir;

use super::super::converters::{
    convert_command_markdown, convert_skill_package, convert_subagent_with_source_hash,
    read_markdown_file_limited, validate_skill_package_privacy,
};
use super::super::manifest::{
    hash_string, MAX_SCAN_DEPTH, MAX_SCAN_ENTRIES, MAX_SCAN_MARKDOWN_FILES,
    MAX_UNSUPPORTED_RULE_REPORTS,
};
use super::super::markdown::{first_useful_line_or_heading, yaml_string};
use super::super::types::{
    Competitor, ConversionContext, ConversionError, ImportCandidate, ImportIssue, ImportKind,
    ImportPrivacyFilter, ImportScope, ImportStatus, ImportSummary, NormalizedSubagent, ToolPolicy,
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

#[cfg(test)]
#[allow(dead_code)]
pub fn scan_global_root(home_dir: &Path, staging_root: &Path) -> ContinueScanResult {
    scan_global_root_with_filter(home_dir, staging_root, &ImportPrivacyFilter::allow_all())
}

pub(crate) fn scan_global_root_with_filter(
    home_dir: &Path,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
) -> ContinueScanResult {
    scan_continue_root_with_filter(
        &home_dir.join(".continue"),
        ImportScope::Global,
        staging_root,
        filter,
    )
}

#[cfg(test)]
#[allow(dead_code)]
pub fn scan_project_root(workspace_root: &Path, staging_root: &Path) -> ContinueScanResult {
    scan_project_root_with_filter(
        workspace_root,
        staging_root,
        &ImportPrivacyFilter::allow_all(),
    )
}

pub(crate) fn scan_project_root_with_filter(
    workspace_root: &Path,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
) -> ContinueScanResult {
    let scope = ImportScope::Project {
        root: workspace_root.to_path_buf(),
    };
    let mut result = scan_continue_root_with_filter(
        &workspace_root.join(".continue"),
        scope.clone(),
        staging_root,
        filter,
    );
    report_workspace_rule_files(workspace_root, &scope, &mut result);
    result
}

#[cfg(test)]
#[allow(dead_code)]
pub fn scan_continue_root(
    source_root: &Path,
    scope: ImportScope,
    staging_root: &Path,
) -> ContinueScanResult {
    scan_continue_root_with_filter(
        source_root,
        scope,
        staging_root,
        &ImportPrivacyFilter::allow_all(),
    )
}

pub(crate) fn scan_continue_root_with_filter(
    source_root: &Path,
    scope: ImportScope,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
) -> ContinueScanResult {
    let context = ConversionContext {
        competitor: Competitor::ContinueDev,
        scope,
        source_root: source_root.to_path_buf(),
    };
    let mut result = ContinueScanResult::default();
    match super::scan_root_allowed(&context, source_root) {
        Ok(true) => {}
        Ok(false) => return result,
        Err(message) => {
            result.add_issue(super::skipped_root_issue(
                &context,
                None,
                source_root,
                message,
            ));
            return result;
        }
    }

    scan_skills(&context, staging_root, filter, &mut result);
    scan_prompts(&context, filter, &mut result);
    scan_checks(&context, filter, &mut result);
    report_continue_rule_files(&context, &mut result);
    result
}

fn scan_skills(
    context: &ConversionContext,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
    result: &mut ContinueScanResult,
) {
    let skills_root = context.source_root.join("skills");
    match super::scan_root_allowed(context, &skills_root) {
        Ok(true) => {}
        Ok(false) => return,
        Err(message) => {
            result.add_issue(super::skipped_root_issue(
                context,
                Some(ImportKind::Skill),
                &skills_root,
                message,
            ));
            return;
        }
    }
    let skill_files = collect_named_files_at_depth(&skills_root, "SKILL.md", 2);
    report_scan_caps(
        context,
        ImportKind::Skill,
        &skills_root,
        &skill_files,
        "Continue skills",
        MAX_SCAN_MARKDOWN_FILES,
        result,
    );
    for skill_md in skill_files.paths {
        let Some(skill_dir) = skill_md.parent() else {
            continue;
        };
        if let Err(message) = validate_skill_package_privacy(skill_dir, filter) {
            result.add_issue(super::privacy_skip_issue(
                context,
                ImportKind::Skill,
                skill_dir,
                message,
            ));
            continue;
        }
        match read_parsed_markdown(&skill_md, filter, context, ImportKind::Skill, "skill") {
            Ok(_) => match convert_skill_package(context, skill_dir, staging_root) {
                Ok(candidate) => result.add_candidate(candidate),
                Err(err) => result.add_issue(err.into_issue()),
            },
            Err(issue) => result.add_issue(issue),
        }
    }
}

fn scan_prompts(
    context: &ConversionContext,
    filter: &ImportPrivacyFilter,
    result: &mut ContinueScanResult,
) {
    let prompts_root = context.source_root.join("prompts");
    match super::scan_root_allowed(context, &prompts_root) {
        Ok(true) => {}
        Ok(false) => return,
        Err(message) => {
            result.add_issue(super::skipped_root_issue(
                context,
                Some(ImportKind::Command),
                &prompts_root,
                message,
            ));
            return;
        }
    }
    let prompt_files = collect_markdown_files(&prompts_root, MAX_SCAN_MARKDOWN_FILES);
    report_scan_caps(
        context,
        ImportKind::Command,
        &prompts_root,
        &prompt_files,
        "Continue prompts",
        MAX_SCAN_MARKDOWN_FILES,
        result,
    );
    for prompt_path in prompt_files.paths {
        match read_parsed_markdown(&prompt_path, filter, context, ImportKind::Command, "prompt") {
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
            Err(issue) => result.add_issue(issue),
        }
    }
}

fn scan_checks(
    context: &ConversionContext,
    filter: &ImportPrivacyFilter,
    result: &mut ContinueScanResult,
) {
    let checks_root = context.source_root.join("checks");
    match super::scan_root_allowed(context, &checks_root) {
        Ok(true) => {}
        Ok(false) => return,
        Err(message) => {
            result.add_issue(super::skipped_root_issue(
                context,
                Some(ImportKind::Subagent),
                &checks_root,
                message,
            ));
            return;
        }
    }
    let check_files = collect_markdown_files(&checks_root, MAX_SCAN_MARKDOWN_FILES);
    report_scan_caps(
        context,
        ImportKind::Subagent,
        &checks_root,
        &check_files,
        "Continue checks",
        MAX_SCAN_MARKDOWN_FILES,
        result,
    );
    for check_path in check_files.paths {
        match read_parsed_markdown(&check_path, filter, context, ImportKind::Subagent, "check") {
            Ok((content, parsed)) => {
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
                match convert_subagent_with_source_hash(
                    context,
                    &check_path,
                    &input,
                    hash_string(&content),
                ) {
                    Ok(candidate) => result.add_candidate(candidate),
                    Err(err) => result.add_issue(err.into_issue()),
                }
            }
            Err(issue) => result.add_issue(issue),
        }
    }
}

fn report_continue_rule_files(context: &ConversionContext, result: &mut ContinueScanResult) {
    let rules_root = context.source_root.join("rules");
    match super::scan_root_allowed(context, &rules_root) {
        Ok(true) => {
            let rule_files = collect_markdown_files(&rules_root, MAX_UNSUPPORTED_RULE_REPORTS);
            for path in &rule_files.paths {
                result.add_issue(unsupported_issue(
                    context,
                    ImportKind::UnsupportedRules,
                    path,
                    "Continue rules are report-only in v1",
                ));
            }
            report_rule_scan_caps(context, &rules_root, &rule_files, result);
        }
        Ok(false) => {}
        Err(message) => result.add_issue(super::skipped_root_issue(
            context,
            Some(ImportKind::UnsupportedRules),
            &rules_root,
            message,
        )),
    }
    let root_rules = context.source_root.join("rules.md");
    if super::regular_file_exists(&root_rules) {
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
        if super::regular_file_exists(&path) {
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

fn read_parsed_markdown(
    path: &Path,
    filter: &ImportPrivacyFilter,
    context: &ConversionContext,
    kind: ImportKind,
    label: &str,
) -> Result<(String, ParsedMarkdown), ImportIssue> {
    super::check_privacy(filter, context, kind, path)?;
    let content = read_markdown_file_limited(path).map_err(|err| {
        error_issue(
            context,
            kind,
            path,
            format!("invalid Continue {label} markdown: failed to read file: {err}"),
        )
    })?;
    let parsed = parse_markdown_with_errors(&content).map_err(|message| {
        error_issue(
            context,
            kind,
            path,
            format!("invalid Continue {label} markdown: {message}"),
        )
    })?;
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

#[derive(Debug, Default)]
struct CollectedFiles {
    paths: Vec<PathBuf>,
    depth_capped: bool,
    file_capped: bool,
    entry_capped: bool,
}

fn collect_markdown_files(root: &Path, max_files: usize) -> CollectedFiles {
    collect_files(root, max_files, |path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    })
}

fn collect_named_files_at_depth(root: &Path, file_name: &str, depth: usize) -> CollectedFiles {
    collect_files(root, MAX_SCAN_MARKDOWN_FILES, |path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == file_name)
            && path
                .strip_prefix(root)
                .map(|relative| relative.components().count() == depth)
                .unwrap_or(false)
    })
}

fn collect_files(root: &Path, max_files: usize, keep: impl Fn(&Path) -> bool) -> CollectedFiles {
    if !super::regular_dir_exists(root) {
        return CollectedFiles::default();
    }
    let mut collected = CollectedFiles::default();
    let mut entry_count = 0usize;
    let mut entries = WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .max_depth(MAX_SCAN_DEPTH + 1)
        .into_iter();
    while let Some(entry) = entries.next() {
        let Ok(entry) = entry else {
            continue;
        };
        entry_count += 1;
        if entry_count > MAX_SCAN_ENTRIES {
            collected.entry_capped = true;
            break;
        }
        if entry.depth() > MAX_SCAN_DEPTH {
            collected.depth_capped = true;
            if entry.file_type().is_dir() {
                entries.skip_current_dir();
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        if !keep(&path) {
            continue;
        }
        if collected.paths.len() >= max_files {
            collected.file_capped = true;
            break;
        }
        collected.paths.push(path);
    }
    collected
}

fn report_scan_caps(
    context: &ConversionContext,
    kind: ImportKind,
    root: &Path,
    files: &CollectedFiles,
    label: &str,
    max_files: usize,
    result: &mut ContinueScanResult,
) {
    if files.depth_capped {
        result.add_issue(error_issue(
            context,
            kind,
            root,
            format!("{label} scan reached {MAX_SCAN_DEPTH} depth limit"),
        ));
    }
    if files.file_capped {
        result.add_issue(error_issue(
            context,
            kind,
            root,
            format!("{label} scan capped after {max_files} markdown files"),
        ));
    }
    if files.entry_capped {
        result.add_issue(error_issue(
            context,
            kind,
            root,
            format!("{label} scan capped after {MAX_SCAN_ENTRIES} filesystem entries"),
        ));
    }
}

fn report_rule_scan_caps(
    context: &ConversionContext,
    root: &Path,
    files: &CollectedFiles,
    result: &mut ContinueScanResult,
) {
    if files.depth_capped || files.file_capped || files.entry_capped {
        result.add_issue(unsupported_issue(
            context,
            ImportKind::UnsupportedRules,
            root,
            format!("Continue rules scan capped after {MAX_UNSUPPORTED_RULE_REPORTS} reports"),
        ));
    }
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
    use std::fs;
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
}
