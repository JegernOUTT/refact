use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};

use super::super::converters::{
    convert_command_markdown, convert_skill_package, convert_subagent_with_source_hash,
    read_markdown_file_limited, validate_skill_package_privacy,
};
use super::super::manifest::{
    hash_string, MAX_SCAN_DEPTH, MAX_SCAN_DIRECT_CHILD_DIRS, MAX_SCAN_ENTRIES,
    MAX_SCAN_MARKDOWN_FILES,
};
use super::super::markdown::{
    first_useful_line_or_heading, sanitize_subagent_id, yaml_string, yaml_string_any,
    yaml_string_list_any,
};
use super::super::types::{
    Competitor, ConversionContext, ImportCandidate, ImportIssue, ImportKind, ImportPrivacyFilter,
    ImportScope, ImportStatus, NormalizedSubagent, ToolPolicy,
};

#[cfg(test)]
#[allow(dead_code)]
pub fn collect_global_candidates(
    home_dir: &Path,
    refact_config_dir: &Path,
) -> (Vec<ImportCandidate>, Vec<ImportIssue>) {
    collect_global_candidates_with_filter(
        home_dir,
        refact_config_dir,
        &ImportPrivacyFilter::allow_all(),
    )
}

pub(crate) fn collect_global_candidates_with_filter(
    home_dir: &Path,
    refact_config_dir: &Path,
    filter: &ImportPrivacyFilter,
) -> (Vec<ImportCandidate>, Vec<ImportIssue>) {
    let source_root = home_dir.join(".claude");
    let staging_root = refact_config_dir
        .join("imports")
        .join("staging")
        .join("claude");
    collect_candidates_with_filter(&source_root, ImportScope::Global, &staging_root, filter)
}

#[cfg(test)]
#[allow(dead_code)]
pub fn collect_project_candidates(
    workspace_root: &Path,
) -> (Vec<ImportCandidate>, Vec<ImportIssue>) {
    collect_project_candidates_with_filter(workspace_root, &ImportPrivacyFilter::allow_all())
}

pub(crate) fn collect_project_candidates_with_filter(
    workspace_root: &Path,
    filter: &ImportPrivacyFilter,
) -> (Vec<ImportCandidate>, Vec<ImportIssue>) {
    let source_root = workspace_root.join(".claude");
    let staging_root = workspace_root
        .join(".refact")
        .join("imports")
        .join("staging")
        .join("claude");
    collect_candidates_with_filter(
        &source_root,
        ImportScope::Project {
            root: workspace_root.to_path_buf(),
        },
        &staging_root,
        filter,
    )
}

#[cfg(test)]
#[allow(dead_code)]
pub fn collect_candidates(
    source_root: &Path,
    scope: ImportScope,
    staging_root: &Path,
) -> (Vec<ImportCandidate>, Vec<ImportIssue>) {
    collect_candidates_with_filter(
        source_root,
        scope,
        staging_root,
        &ImportPrivacyFilter::allow_all(),
    )
}

pub(crate) fn collect_candidates_with_filter(
    source_root: &Path,
    scope: ImportScope,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
) -> (Vec<ImportCandidate>, Vec<ImportIssue>) {
    let mut candidates = Vec::new();
    let mut issues = Vec::new();
    let context = ConversionContext {
        competitor: Competitor::ClaudeCode,
        scope,
        source_root: source_root.to_path_buf(),
    };
    match super::scan_root_allowed(&context, source_root) {
        Ok(true) => {}
        Ok(false) => return (candidates, issues),
        Err(message) => {
            issues.push(super::skipped_root_issue(
                &context,
                None,
                source_root,
                message,
            ));
            return (candidates, issues);
        }
    }

    collect_skill_candidates(
        &context,
        &source_root.join("skills"),
        staging_root,
        filter,
        &mut candidates,
        &mut issues,
    );
    collect_command_candidates(
        &context,
        &source_root.join("commands"),
        filter,
        &mut candidates,
        &mut issues,
    );
    collect_agent_candidates(
        &context,
        &source_root.join("agents"),
        filter,
        &mut candidates,
        &mut issues,
    );

    (candidates, issues)
}

fn collect_skill_candidates(
    context: &ConversionContext,
    skills_root: &Path,
    staging_root: &Path,
    filter: &ImportPrivacyFilter,
    candidates: &mut Vec<ImportCandidate>,
    issues: &mut Vec<ImportIssue>,
) {
    match super::scan_root_allowed(context, skills_root) {
        Ok(true) => {}
        Ok(false) => return,
        Err(message) => {
            issues.push(super::skipped_root_issue(
                context,
                Some(ImportKind::Skill),
                skills_root,
                message,
            ));
            return;
        }
    }
    let entries = match direct_child_dirs(skills_root) {
        Ok(entries) => entries,
        Err(err) => {
            issues.push(issue(
                context,
                ImportKind::Skill,
                skills_root,
                format!("failed to read skills directory: {err}"),
            ));
            return;
        }
    };

    for skill_dir in entries {
        let skill_md = skill_dir.join("SKILL.md");
        if !is_regular_file(&skill_md) {
            continue;
        }
        if let Err(message) = validate_skill_package_privacy(&skill_dir, filter) {
            issues.push(super::privacy_skip_issue(
                context,
                ImportKind::Skill,
                &skill_dir,
                message,
            ));
            continue;
        }
        if let Err(issue) = read_valid_markdown(context, ImportKind::Skill, &skill_md, filter) {
            issues.push(issue);
            continue;
        }
        match convert_skill_package(context, &skill_dir, staging_root) {
            Ok(candidate) => candidates.push(candidate),
            Err(err) => issues.push(err.into_issue()),
        }
    }
}

fn collect_command_candidates(
    context: &ConversionContext,
    commands_root: &Path,
    filter: &ImportPrivacyFilter,
    candidates: &mut Vec<ImportCandidate>,
    issues: &mut Vec<ImportIssue>,
) {
    for path in markdown_files(context, ImportKind::Command, commands_root, issues) {
        let (_, _, content) = match read_valid_markdown(context, ImportKind::Command, &path, filter)
        {
            Ok(parsed) => parsed,
            Err(issue) => {
                issues.push(issue);
                continue;
            }
        };
        let name = relative_stem_name(commands_root, &path);
        match convert_command_markdown(context, &path, &content, Some(&name)) {
            Ok(candidate) => candidates.push(candidate),
            Err(err) => issues.push(err.into_issue()),
        }
    }
}

fn collect_agent_candidates(
    context: &ConversionContext,
    agents_root: &Path,
    filter: &ImportPrivacyFilter,
    candidates: &mut Vec<ImportCandidate>,
    issues: &mut Vec<ImportIssue>,
) {
    for path in markdown_files(context, ImportKind::Subagent, agents_root, issues) {
        let (frontmatter, body, content) =
            match read_valid_markdown(context, ImportKind::Subagent, &path, filter) {
                Ok(parsed) => parsed,
                Err(issue) => {
                    issues.push(issue);
                    continue;
                }
            };
        let input = normalized_agent(agents_root, &path, &frontmatter, &body);
        match convert_subagent_with_source_hash(context, &path, &input, hash_string(&content)) {
            Ok(candidate) => candidates.push(candidate),
            Err(err) => issues.push(err.into_issue()),
        }
    }
}

fn normalized_agent(
    agents_root: &Path,
    source_path: &Path,
    frontmatter: &YamlValue,
    body: &str,
) -> NormalizedSubagent {
    let relative_name = relative_stem_name(agents_root, source_path);
    let frontmatter_name = yaml_string(frontmatter, "name");
    let frontmatter_name_valid =
        !frontmatter_name.is_empty() && !sanitize_subagent_id(&frontmatter_name).is_empty();
    let id = if frontmatter_name_valid {
        frontmatter_name.clone()
    } else {
        relative_name.clone()
    };
    let description = first_non_empty_string(&[
        yaml_string(frontmatter, "description"),
        yaml_string_any(frontmatter, &["whenToUse", "when_to_use", "when-to-use"]),
        first_useful_line_or_heading(body).unwrap_or_default(),
    ]);
    let allowed_tools =
        yaml_string_list_any(frontmatter, &["tools", "allowed-tools", "allowed_tools"]);
    let denied_tools = yaml_string_list_any(
        frontmatter,
        &[
            "denied-tools",
            "denied_tools",
            "disallowed-tools",
            "disallowed_tools",
        ],
    );
    let tool_policy = ToolPolicy {
        allowed: if allowed_tools.is_empty() {
            None
        } else {
            Some(allowed_tools)
        },
        denied: denied_tools,
    };
    let model = non_empty(yaml_string(frontmatter, "model"));
    let max_steps = yaml_usize_any(frontmatter, &["maxTurns", "max_turns", "max-turns"]);
    let effort = yaml_string_any(
        frontmatter,
        &["effort", "reasoning_effort", "reasoning-effort"],
    );
    let mut metadata = JsonMap::new();
    insert_string(
        &mut metadata,
        "source_relative_path",
        &relative_file_name(agents_root, source_path),
    );
    insert_string(&mut metadata, "claude_effort", &effort);

    NormalizedSubagent {
        id,
        title: if frontmatter_name_valid {
            frontmatter_name
        } else {
            relative_name
        },
        description,
        prompt: body.to_string(),
        tool_policy,
        max_steps,
        model,
        metadata: JsonValue::Object(metadata),
    }
}

fn markdown_files(
    context: &ConversionContext,
    kind: ImportKind,
    root: &Path,
    issues: &mut Vec<ImportIssue>,
) -> Vec<PathBuf> {
    match super::scan_root_allowed(context, root) {
        Ok(true) => {}
        Ok(false) => return Vec::new(),
        Err(message) => {
            issues.push(super::skipped_root_issue(
                context,
                Some(kind),
                root,
                message,
            ));
            return Vec::new();
        }
    }
    let mut paths = Vec::new();
    let mut depth_capped = false;
    let mut file_capped = false;
    let mut entry_capped = false;
    let mut entry_count = 0usize;
    let mut entries = walkdir::WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .max_depth(MAX_SCAN_DEPTH + 1)
        .into_iter();
    while let Some(entry) = entries.next() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                let path = err
                    .path()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| root.to_path_buf());
                issues.push(issue(
                    context,
                    kind,
                    &path,
                    format!("failed to scan directory entry: {err}"),
                ));
                continue;
            }
        };
        entry_count += 1;
        if entry_count > MAX_SCAN_ENTRIES {
            entry_capped = true;
            break;
        }
        if entry.depth() > MAX_SCAN_DEPTH {
            depth_capped = true;
            if entry.file_type().is_dir() {
                entries.skip_current_dir();
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        if paths.len() >= MAX_SCAN_MARKDOWN_FILES {
            file_capped = true;
            break;
        }
        paths.push(entry.path().to_path_buf());
    }
    if depth_capped {
        issues.push(issue(
            context,
            kind,
            root,
            format!("markdown scan reached {MAX_SCAN_DEPTH} depth limit"),
        ));
    }
    if file_capped {
        issues.push(issue(
            context,
            kind,
            root,
            format!("markdown scan capped after {MAX_SCAN_MARKDOWN_FILES} markdown files"),
        ));
    }
    if entry_capped {
        issues.push(issue(
            context,
            kind,
            root,
            format!("markdown scan capped after {MAX_SCAN_ENTRIES} filesystem entries"),
        ));
    }
    paths
}

fn read_valid_markdown(
    context: &ConversionContext,
    kind: ImportKind,
    path: &Path,
    filter: &ImportPrivacyFilter,
) -> Result<(YamlValue, String, String), ImportIssue> {
    super::check_privacy(filter, context, kind, path)?;
    let content = read_markdown_file_limited(path).map_err(|err| {
        issue(
            context,
            kind,
            path,
            format!("failed to read markdown file: {err}"),
        )
    })?;
    let (frontmatter, body) = parse_markdown_frontmatter_strict(&content)
        .map_err(|err| issue(context, kind, path, format!("invalid frontmatter: {err}")))?;
    Ok((frontmatter, body, content))
}

fn parse_markdown_frontmatter_strict(content: &str) -> Result<(YamlValue, String), String> {
    if !content.starts_with("---") {
        return Ok((empty_frontmatter(), content.to_string()));
    }
    let after_dashes = &content[3..];
    let rest = if let Some(rest) = after_dashes.strip_prefix("\r\n") {
        rest
    } else if let Some(rest) = after_dashes.strip_prefix('\n') {
        rest
    } else {
        return Ok((empty_frontmatter(), content.to_string()));
    };
    let (frontmatter_str, body) = if let Some(after_close) = rest.strip_prefix("---") {
        ("", trim_leading_newline(after_close).to_string())
    } else {
        let end_marker = "\n---";
        let Some(end_pos) = rest.find(end_marker) else {
            return Err("missing closing delimiter".to_string());
        };
        let frontmatter_str = &rest[..end_pos];
        let after_end = &rest[end_pos + end_marker.len()..];
        (frontmatter_str, trim_leading_newline(after_end).to_string())
    };
    if frontmatter_str.trim().is_empty() {
        return Ok((empty_frontmatter(), body));
    }
    let value =
        serde_yaml::from_str::<YamlValue>(frontmatter_str).map_err(|err| err.to_string())?;
    if value.is_null() {
        return Ok((empty_frontmatter(), body));
    }
    if value.as_mapping().is_none() {
        return Err("frontmatter must be a YAML mapping".to_string());
    }
    Ok((value, body))
}

fn trim_leading_newline(value: &str) -> &str {
    value
        .strip_prefix("\r\n")
        .or_else(|| value.strip_prefix('\n'))
        .unwrap_or(value)
}

fn empty_frontmatter() -> YamlValue {
    YamlValue::Mapping(Mapping::new())
}

fn relative_stem_name(root: &Path, source_path: &Path) -> String {
    let relative = source_path.strip_prefix(root).unwrap_or(source_path);
    let mut stem = relative.to_path_buf();
    stem.set_extension("");
    path_to_slash_name(&stem)
}

fn relative_file_name(root: &Path, source_path: &Path) -> String {
    let relative = source_path.strip_prefix(root).unwrap_or(source_path);
    path_to_slash_name(relative)
}

fn path_to_slash_name(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn yaml_usize_any(frontmatter: &YamlValue, keys: &[&str]) -> Option<usize> {
    for key in keys {
        let Some(value) = frontmatter.get(*key) else {
            continue;
        };
        match value {
            YamlValue::Number(number) => {
                if let Some(value) = number
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                {
                    return Some(value);
                }
            }
            YamlValue::String(value) => {
                if let Ok(value) = value.trim().parse::<usize>() {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn first_non_empty_string(values: &[String]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn insert_string(metadata: &mut JsonMap<String, JsonValue>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        metadata.insert(key.to_string(), JsonValue::String(value.trim().to_string()));
    }
}

fn direct_child_dirs(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    let mut entry_count = 0usize;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        entry_count += 1;
        if entry_count > MAX_SCAN_ENTRIES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("direct child scan capped after {MAX_SCAN_ENTRIES} filesystem entries"),
            ));
        }
        if entry_count > MAX_SCAN_DIRECT_CHILD_DIRS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("direct child scan capped after {MAX_SCAN_DIRECT_CHILD_DIRS} entries"),
            ));
        }
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn issue(
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
        status: ImportStatus::Error,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::competitor_import::markdown::{parse_markdown_frontmatter, yaml_string_list};
    use crate::ext::competitor_import::{run_global_import_with_paths, run_project_import_with_paths};
    use crate::yaml_configs::customization_types::SubagentConfig;

    fn write_file(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}
