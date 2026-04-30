use std::fs;
use std::io::{ErrorKind, Result as IoResult};
use std::path::{Path, PathBuf};

use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::Value as YamlValue;

use super::super::converters::{convert_command_markdown, convert_skill_package, convert_subagent};
use super::super::markdown::{
    parse_markdown_frontmatter, render_markdown_with_frontmatter, set_yaml_string,
    set_yaml_string_list, yaml_string,
};
use super::super::types::{
    Competitor, ConversionContext, ImportCandidate, ImportIssue, ImportKind, ImportScope,
    ImportStatus, ImportSummary, NormalizedSubagent, ToolPolicy,
};
use super::super::writer::write_candidates;

#[derive(Debug, Default)]
pub struct OpenCodeScan {
    pub candidates: Vec<ImportCandidate>,
    pub issues: Vec<ImportIssue>,
}

impl OpenCodeScan {
    pub fn to_summary(&self) -> ImportSummary {
        let mut summary = ImportSummary::default();
        for candidate in &self.candidates {
            summary.record_candidate(candidate);
        }
        for issue in &self.issues {
            summary.add_issue(issue.clone());
        }
        summary
    }

    fn issues_summary(&self) -> ImportSummary {
        let mut summary = ImportSummary::default();
        for issue in &self.issues {
            summary.add_issue(issue.clone());
        }
        summary
    }

    fn push_candidate_result(
        &mut self,
        result: Result<ImportCandidate, super::super::types::ConversionError>,
    ) {
        match result {
            Ok(candidate) => self.candidates.push(candidate),
            Err(err) => self.issues.push(err.into_issue()),
        }
    }

    fn push_issue(&mut self, issue: ImportIssue) {
        self.issues.push(issue);
    }
}

pub(super) struct CompatibleScanRoots {
    pub display_name: &'static str,
    pub skill_roots: Vec<PathBuf>,
    pub command_roots: Vec<PathBuf>,
    pub agent_roots: Vec<PathBuf>,
    pub config_files: Vec<PathBuf>,
}

pub(super) fn scan_compatible_roots(
    context: &ConversionContext,
    roots: CompatibleScanRoots,
    staging_root: &Path,
) -> OpenCodeScan {
    let mut scan = OpenCodeScan::default();
    for skills_root in roots.skill_roots {
        scan_skills(
            &mut scan,
            context,
            &skills_root,
            staging_root,
            roots.display_name,
        );
    }
    for command_root in roots.command_roots {
        scan_markdown_commands(&mut scan, context, &command_root, roots.display_name);
    }
    for agent_root in roots.agent_roots {
        scan_markdown_agents(&mut scan, context, &agent_root, roots.display_name);
    }
    scan_config_files(&mut scan, context, &roots.config_files, roots.display_name);
    scan
}

pub fn scan_project_root(workspace_root: &Path) -> OpenCodeScan {
    let staging_root = workspace_root
        .join(".refact")
        .join("imports")
        .join("staging")
        .join("opencode");
    scan_project_root_with_staging(workspace_root, &staging_root)
}

pub fn scan_project_root_with_staging(workspace_root: &Path, staging_root: &Path) -> OpenCodeScan {
    let context = ConversionContext {
        competitor: Competitor::OpenCode,
        scope: ImportScope::Project {
            root: workspace_root.to_path_buf(),
        },
        source_root: workspace_root.to_path_buf(),
    };
    let opencode_root = workspace_root.join(".opencode");
    let config_files = [
        workspace_root.join("opencode.json"),
        workspace_root.join("opencode.jsonc"),
    ];
    scan_root(&context, &opencode_root, &config_files, staging_root)
}

pub fn scan_global_root(config_root: &Path, refact_config_root: &Path) -> OpenCodeScan {
    let staging_root = refact_config_root
        .join("imports")
        .join("staging")
        .join("opencode");
    scan_global_root_with_staging(config_root, &staging_root)
}

pub fn scan_global_root_with_staging(config_root: &Path, staging_root: &Path) -> OpenCodeScan {
    let context = ConversionContext {
        competitor: Competitor::OpenCode,
        scope: ImportScope::Global,
        source_root: config_root.to_path_buf(),
    };
    let config_files = [
        config_root.join("opencode.json"),
        config_root.join("opencode.jsonc"),
        config_root.join("config.json"),
    ];
    scan_root(&context, config_root, &config_files, staging_root)
}

pub async fn import_project_root(workspace_root: &Path) -> ImportSummary {
    let scan = scan_project_root(workspace_root);
    let mut summary = scan.issues_summary();
    let write_summary = write_candidates(&workspace_root.join(".refact"), &scan.candidates).await;
    summary.merge(write_summary);
    summary
}

pub async fn import_global_root(config_root: &Path, refact_config_root: &Path) -> ImportSummary {
    let scan = scan_global_root(config_root, refact_config_root);
    let mut summary = scan.issues_summary();
    let write_summary = write_candidates(refact_config_root, &scan.candidates).await;
    summary.merge(write_summary);
    summary
}

fn scan_root(
    context: &ConversionContext,
    opencode_root: &Path,
    config_files: &[PathBuf],
    staging_root: &Path,
) -> OpenCodeScan {
    scan_compatible_roots(
        context,
        CompatibleScanRoots {
            display_name: "OpenCode",
            skill_roots: vec![opencode_root.join("skills")],
            command_roots: vec![
                opencode_root.join("commands"),
                opencode_root.join("command"),
            ],
            agent_roots: vec![opencode_root.join("agents"), opencode_root.join("agent")],
            config_files: config_files.to_vec(),
        },
        staging_root,
    )
}

fn scan_skills(
    scan: &mut OpenCodeScan,
    context: &ConversionContext,
    skills_root: &Path,
    staging_root: &Path,
    display_name: &str,
) {
    let skill_dirs = match direct_child_dirs(skills_root) {
        Ok(skill_dirs) => skill_dirs,
        Err(err) => {
            scan.push_issue(error_issue(
                context,
                ImportKind::Skill,
                skills_root,
                format!("failed to scan {display_name} skills directory: {err}"),
            ));
            return;
        }
    };
    for skill_dir in skill_dirs {
        if is_regular_file(&skill_dir.join("SKILL.md")) {
            scan.push_candidate_result(convert_skill_package(context, &skill_dir, staging_root));
        }
    }
}

fn scan_markdown_commands(
    scan: &mut OpenCodeScan,
    context: &ConversionContext,
    command_root: &Path,
    display_name: &str,
) {
    let command_paths = match recursive_markdown_files(command_root) {
        Ok(command_paths) => command_paths,
        Err(err) => {
            scan.push_issue(error_issue(
                context,
                ImportKind::Command,
                command_root,
                format!("failed to scan {display_name} commands directory: {err}"),
            ));
            return;
        }
    };
    for command_path in command_paths {
        match fs::read_to_string(&command_path) {
            Ok(content) => {
                let name = relative_markdown_name(command_root, &command_path);
                scan.push_candidate_result(convert_command_markdown(
                    context,
                    &command_path,
                    &content,
                    Some(&name),
                ));
            }
            Err(err) => scan.push_issue(error_issue(
                context,
                ImportKind::Command,
                &command_path,
                format!("failed to read {display_name} command markdown: {err}"),
            )),
        }
    }
}

fn scan_markdown_agents(
    scan: &mut OpenCodeScan,
    context: &ConversionContext,
    agent_root: &Path,
    display_name: &str,
) {
    let agent_paths = match recursive_markdown_files(agent_root) {
        Ok(agent_paths) => agent_paths,
        Err(err) => {
            scan.push_issue(error_issue(
                context,
                ImportKind::Subagent,
                agent_root,
                format!("failed to scan {display_name} agents directory: {err}"),
            ));
            return;
        }
    };
    for agent_path in agent_paths {
        match normalized_markdown_agent(context, agent_root, &agent_path, display_name) {
            Ok(agent) => scan.push_candidate_result(convert_subagent(context, &agent_path, &agent)),
            Err(issue) => scan.push_issue(issue),
        }
    }
}

fn scan_config_files(
    scan: &mut OpenCodeScan,
    context: &ConversionContext,
    config_files: &[PathBuf],
    display_name: &str,
) {
    for config_path in config_files {
        if !is_regular_file(config_path) {
            continue;
        }
        match read_json_or_jsonc(config_path) {
            Ok(config) => scan_config_value(scan, context, config_path, &config, display_name),
            Err(err) => scan.push_issue(error_issue(
                context,
                ImportKind::Command,
                config_path,
                format!("failed to parse {display_name} config: {err}"),
            )),
        }
    }
}

fn scan_config_value(
    scan: &mut OpenCodeScan,
    context: &ConversionContext,
    config_path: &Path,
    config: &JsonValue,
    display_name: &str,
) {
    match config.get("command") {
        Some(JsonValue::Object(commands)) => {
            scan_config_commands(scan, context, config_path, commands, display_name)
        }
        Some(_) => scan.push_issue(unsupported_issue(
            context,
            ImportKind::Command,
            config_path,
            format!("{display_name} command config is not an object"),
        )),
        None => {}
    }
    match config.get("agent") {
        Some(JsonValue::Object(agents)) => {
            scan_config_agents(scan, context, config_path, agents, display_name)
        }
        Some(_) => scan.push_issue(unsupported_issue(
            context,
            ImportKind::Subagent,
            config_path,
            format!("{display_name} agent config is not an object"),
        )),
        None => {}
    }
}

fn scan_config_commands(
    scan: &mut OpenCodeScan,
    context: &ConversionContext,
    config_path: &Path,
    commands: &JsonMap<String, JsonValue>,
    display_name: &str,
) {
    let mut names = commands.keys().collect::<Vec<_>>();
    names.sort();
    for name in names {
        let Some(value) = commands.get(name) else {
            continue;
        };
        match config_command_markdown(name, value) {
            Ok(markdown) => scan.push_candidate_result(convert_command_markdown(
                context,
                config_path,
                &markdown,
                Some(name),
            )),
            Err(message) => scan.push_issue(unsupported_issue(
                context,
                ImportKind::Command,
                config_path,
                format!("{display_name} command {name} skipped: {message}"),
            )),
        }
    }
}

fn scan_config_agents(
    scan: &mut OpenCodeScan,
    context: &ConversionContext,
    config_path: &Path,
    agents: &JsonMap<String, JsonValue>,
    display_name: &str,
) {
    let mut names = agents.keys().collect::<Vec<_>>();
    names.sort();
    for name in names {
        let Some(value) = agents.get(name) else {
            continue;
        };
        match normalized_config_agent(name, value) {
            Ok(agent) => scan.push_candidate_result(convert_subagent(context, config_path, &agent)),
            Err(message) => scan.push_issue(unsupported_issue(
                context,
                ImportKind::Subagent,
                config_path,
                format!("{display_name} agent {name} skipped: {message}"),
            )),
        }
    }
}

fn normalized_markdown_agent(
    context: &ConversionContext,
    agent_root: &Path,
    agent_path: &Path,
    display_name: &str,
) -> Result<NormalizedSubagent, ImportIssue> {
    let content = fs::read_to_string(agent_path).map_err(|err| {
        error_issue(
            context,
            ImportKind::Subagent,
            agent_path,
            format!("failed to read {display_name} agent markdown: {err}"),
        )
    })?;
    let (frontmatter, body) = parse_markdown_frontmatter(&content);
    let prompt = body.trim().to_string();
    if prompt.is_empty() {
        return Err(unsupported_issue(
            context,
            ImportKind::Subagent,
            agent_path,
            format!("{display_name} markdown agent has no prompt body"),
        ));
    }

    let id = relative_markdown_name(agent_root, agent_path);
    let title = yaml_string_any(&frontmatter, &["name", "title"]);
    let description = yaml_string(&frontmatter, "description");
    let mode = yaml_string(&frontmatter, "mode");
    let model = non_empty(yaml_string(&frontmatter, "model"));
    let max_steps = yaml_usize_any(&frontmatter, &["maxSteps", "max_steps", "steps"]);
    let tool_policy = tool_policy_from_yaml(
        yaml_value_any(&frontmatter, &["tools"]),
        yaml_value_any(&frontmatter, &["permission", "permissions"]),
    );
    let mut metadata = JsonMap::new();
    insert_string(&mut metadata, "source", "markdown");
    insert_string(&mut metadata, "mode", &mode);

    Ok(NormalizedSubagent {
        id: id.clone(),
        title: first_non_empty(&[title.as_str(), id.as_str()]).to_string(),
        description,
        prompt,
        tool_policy,
        max_steps,
        model,
        metadata: JsonValue::Object(metadata),
    })
}

fn normalized_config_agent(name: &str, value: &JsonValue) -> Result<NormalizedSubagent, String> {
    match value {
        JsonValue::String(prompt) => string_config_agent(name, prompt),
        JsonValue::Object(object) => object_config_agent(name, object),
        _ => Err("agent entry is not a string or object".to_string()),
    }
}

fn string_config_agent(name: &str, prompt: &str) -> Result<NormalizedSubagent, String> {
    if prompt.trim().is_empty() {
        return Err("agent prompt is empty".to_string());
    }
    let mut metadata = JsonMap::new();
    insert_string(&mut metadata, "source", "config");
    Ok(NormalizedSubagent {
        id: name.to_string(),
        title: name.to_string(),
        description: String::new(),
        prompt: prompt.trim().to_string(),
        tool_policy: ToolPolicy::missing(),
        max_steps: None,
        model: None,
        metadata: JsonValue::Object(metadata),
    })
}

fn object_config_agent(
    name: &str,
    object: &JsonMap<String, JsonValue>,
) -> Result<NormalizedSubagent, String> {
    if json_bool_any(object, &["disabled", "disable"]).unwrap_or(false) {
        return Err("agent entry is disabled".to_string());
    }
    let mode = json_string_any(object, &["mode"]);
    if mode.eq_ignore_ascii_case("primary") {
        return Err("primary agent mode cannot be safely mapped to a Refact subagent".to_string());
    }
    let prompt = json_string_any(object, &["prompt"]);
    if prompt.trim().is_empty() {
        return Err("agent prompt is missing".to_string());
    }

    let mut metadata = JsonMap::new();
    insert_string(&mut metadata, "source", "config");
    insert_string(&mut metadata, "mode", &mode);

    Ok(NormalizedSubagent {
        id: json_string_any(object, &["id", "name"])
            .trim()
            .to_string()
            .if_empty(name),
        title: json_string_any(object, &["title", "name"])
            .trim()
            .to_string()
            .if_empty(name),
        description: json_string_any(object, &["description"]),
        prompt,
        tool_policy: tool_policy_from_json(object.get("tools"), object.get("permission")),
        max_steps: json_usize_any(object, &["maxSteps", "max_steps", "steps"]),
        model: non_empty(json_string_any(object, &["model"])),
        metadata: JsonValue::Object(metadata),
    })
}

fn config_command_markdown(name: &str, value: &JsonValue) -> Result<String, String> {
    match value {
        JsonValue::String(body) => config_command_string_markdown(body),
        JsonValue::Object(object) => config_command_object_markdown(name, object),
        _ => Err("command entry is not a string or object".to_string()),
    }
}

fn config_command_string_markdown(body: &str) -> Result<String, String> {
    if body.trim().is_empty() {
        return Err("command body is empty".to_string());
    }
    render_markdown_with_frontmatter(&serde_yaml::Mapping::new(), body.trim())
}

fn config_command_object_markdown(
    _name: &str,
    object: &JsonMap<String, JsonValue>,
) -> Result<String, String> {
    if json_bool_any(object, &["disabled", "disable"]).unwrap_or(false) {
        return Err("command entry is disabled".to_string());
    }
    let body = json_string_any(object, &["template", "prompt", "content", "body"]);
    if body.trim().is_empty() {
        return Err("command body is missing".to_string());
    }

    let mut frontmatter = serde_yaml::Mapping::new();
    set_yaml_string(
        &mut frontmatter,
        "description",
        &json_string_any(object, &["description"]),
    );
    set_yaml_string(
        &mut frontmatter,
        "model",
        &json_string_any(object, &["model"]),
    );
    set_yaml_string(
        &mut frontmatter,
        "agent",
        &json_string_any(object, &["agent"]),
    );
    set_yaml_string(
        &mut frontmatter,
        "subtask",
        &json_string_any(object, &["subtask"]),
    );
    let allowed_tools = json_string_list_any(object, &["tools", "allowedTools", "allowed_tools"]);
    set_yaml_string_list(&mut frontmatter, "allowed-tools", &allowed_tools);
    render_markdown_with_frontmatter(&frontmatter, body.trim())
}

fn read_json_or_jsonc(path: &Path) -> Result<JsonValue, String> {
    let content = fs::read_to_string(path).map_err(|err| err.to_string())?;
    match serde_json::from_str::<JsonValue>(&content) {
        Ok(value) => Ok(value),
        Err(first_err) => {
            let stripped = strip_jsonc(&content)?;
            serde_json::from_str::<JsonValue>(&stripped)
                .map_err(|err| format!("{err}; original JSON parse error: {first_err}"))
        }
    }
}

fn strip_jsonc(input: &str) -> Result<String, String> {
    let without_comments = strip_jsonc_comments(input)?;
    Ok(strip_json_trailing_commas(&without_comments))
}

fn strip_jsonc_comments(input: &str) -> Result<String, String> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(input.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '/' && index + 1 < chars.len() {
            match chars[index + 1] {
                '/' => {
                    index += 2;
                    while index < chars.len() && chars[index] != '\n' {
                        index += 1;
                    }
                    if index < chars.len() {
                        out.push('\n');
                        index += 1;
                    }
                    continue;
                }
                '*' => {
                    index += 2;
                    let mut closed = false;
                    while index + 1 < chars.len() {
                        if chars[index] == '*' && chars[index + 1] == '/' {
                            index += 2;
                            closed = true;
                            break;
                        }
                        if chars[index] == '\n' {
                            out.push('\n');
                        } else {
                            out.push(' ');
                        }
                        index += 1;
                    }
                    if !closed {
                        return Err("unterminated block comment".to_string());
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(ch);
        index += 1;
    }
    Ok(out)
}

fn strip_json_trailing_commas(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(input.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == ',' {
            let mut lookahead = index + 1;
            while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                lookahead += 1;
            }
            if lookahead < chars.len() && matches!(chars[lookahead], '}' | ']') {
                index += 1;
                continue;
            }
        }
        out.push(ch);
        index += 1;
    }
    out
}

fn direct_child_dirs(root: &Path) -> IoResult<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn recursive_markdown_files(root: &Path) -> IoResult<Vec<PathBuf>> {
    if !root.exists() || !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .min_depth(1)
    {
        let entry = entry.map_err(|err| std::io::Error::new(ErrorKind::Other, err.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        if entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("md"))
            .unwrap_or(false)
        {
            paths.push(entry.path().to_path_buf());
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false)
}

fn relative_markdown_name(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let without_extension = relative.with_extension("");
    without_extension.to_string_lossy().replace('\\', "/")
}

fn yaml_string_any(frontmatter: &YamlValue, keys: &[&str]) -> String {
    keys.iter()
        .map(|key| yaml_string(frontmatter, key))
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

fn yaml_value_any<'a>(frontmatter: &'a YamlValue, keys: &[&str]) -> Option<&'a YamlValue> {
    keys.iter().find_map(|key| frontmatter.get(*key))
}

fn yaml_usize_any(frontmatter: &YamlValue, keys: &[&str]) -> Option<usize> {
    keys.iter()
        .filter_map(|key| frontmatter.get(*key))
        .find_map(yaml_value_to_usize)
}

fn yaml_value_to_usize(value: &YamlValue) -> Option<usize> {
    match value {
        YamlValue::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok()),
        YamlValue::String(value) => value.trim().parse::<usize>().ok(),
        YamlValue::Sequence(values) => Some(values.len()),
        _ => None,
    }
}

fn json_usize_any(object: &JsonMap<String, JsonValue>, keys: &[&str]) -> Option<usize> {
    keys.iter()
        .filter_map(|key| object.get(*key))
        .find_map(json_value_to_usize)
}

fn json_value_to_usize(value: &JsonValue) -> Option<usize> {
    match value {
        JsonValue::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok()),
        JsonValue::String(value) => value.trim().parse::<usize>().ok(),
        JsonValue::Array(values) => Some(values.len()),
        _ => None,
    }
}

fn json_bool_any(object: &JsonMap<String, JsonValue>, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .filter_map(|key| object.get(*key))
        .find_map(|value| match value {
            JsonValue::Bool(value) => Some(*value),
            JsonValue::String(value) if value.eq_ignore_ascii_case("true") => Some(true),
            JsonValue::String(value) if value.eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        })
}

fn json_string_any(object: &JsonMap<String, JsonValue>, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|key| object.get(*key))
        .find_map(json_value_to_string)
        .unwrap_or_default()
}

fn json_value_to_string(value: &JsonValue) -> Option<String> {
    value.as_str().map(|value| value.trim().to_string())
}

fn json_string_list_any(object: &JsonMap<String, JsonValue>, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter_map(|key| object.get(*key))
        .find_map(json_value_to_string_list)
        .unwrap_or_default()
}

fn json_value_to_string_list(value: &JsonValue) -> Option<Vec<String>> {
    match value {
        JsonValue::String(value) => Some(split_string_list(value)),
        JsonValue::Array(values) => Some(values.iter().filter_map(json_value_to_string).collect()),
        JsonValue::Object(object) => {
            let mut tools = Vec::new();
            for (key, value) in object {
                if tool_value_decision(value) == ToolDecision::Allow {
                    tools.push(key.clone());
                }
            }
            Some(tools)
        }
        _ => None,
    }
}

fn tool_policy_from_yaml(tools: Option<&YamlValue>, permission: Option<&YamlValue>) -> ToolPolicy {
    let tools = tools.and_then(|value| serde_json::to_value(value).ok());
    let permission = permission.and_then(|value| serde_json::to_value(value).ok());
    tool_policy_from_json(tools.as_ref(), permission.as_ref())
}

fn tool_policy_from_json(tools: Option<&JsonValue>, permission: Option<&JsonValue>) -> ToolPolicy {
    let mut builder = ToolPolicyBuilder::default();
    if let Some(tools) = tools {
        builder.absorb_allowed_value(tools);
    }
    if let Some(permission) = permission {
        builder.absorb_permission_value(permission);
    }
    builder.into_policy()
}

#[derive(Default)]
struct ToolPolicyBuilder {
    allowed: Vec<String>,
    denied: Vec<String>,
    allowed_seen: bool,
}

impl ToolPolicyBuilder {
    fn absorb_allowed_value(&mut self, value: &JsonValue) {
        match value {
            JsonValue::String(value) => {
                self.allowed_seen = true;
                for tool in split_string_list(value) {
                    push_unique(&mut self.allowed, &tool);
                }
            }
            JsonValue::Array(values) => {
                self.allowed_seen = true;
                for value in values {
                    if let Some(tool) = json_value_to_string(value) {
                        push_unique(&mut self.allowed, &tool);
                    }
                }
            }
            JsonValue::Object(object) => {
                self.allowed_seen = true;
                self.absorb_tool_object(object);
            }
            _ => {}
        }
    }

    fn absorb_permission_value(&mut self, value: &JsonValue) {
        match value {
            JsonValue::String(value) => {
                if is_negative_decision(value) {
                    push_unique(&mut self.denied, "all");
                }
            }
            JsonValue::Object(object) => self.absorb_tool_object(object),
            _ => {}
        }
    }

    fn absorb_tool_object(&mut self, object: &JsonMap<String, JsonValue>) {
        for (tool, value) in object {
            match tool.as_str() {
                "allow" | "allowed" | "allows" => self.absorb_allowed_value(value),
                "deny" | "denied" | "denies" => self.absorb_denied_value(value),
                _ => match tool_value_decision(value) {
                    ToolDecision::Allow => push_unique(&mut self.allowed, tool),
                    ToolDecision::Deny => push_unique(&mut self.denied, tool),
                    ToolDecision::Unknown => {}
                },
            }
        }
    }

    fn absorb_denied_value(&mut self, value: &JsonValue) {
        match value {
            JsonValue::String(value) => {
                for tool in split_string_list(value) {
                    push_unique(&mut self.denied, &tool);
                }
            }
            JsonValue::Array(values) => {
                for value in values {
                    if let Some(tool) = json_value_to_string(value) {
                        push_unique(&mut self.denied, &tool);
                    }
                }
            }
            JsonValue::Object(object) => {
                for (tool, value) in object {
                    if tool_value_decision(value) != ToolDecision::Allow {
                        push_unique(&mut self.denied, tool);
                    }
                }
            }
            _ => {}
        }
    }

    fn into_policy(self) -> ToolPolicy {
        ToolPolicy {
            allowed: if self.allowed_seen || !self.allowed.is_empty() {
                Some(self.allowed)
            } else {
                None
            },
            denied: self.denied,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolDecision {
    Allow,
    Deny,
    Unknown,
}

fn tool_value_decision(value: &JsonValue) -> ToolDecision {
    match value {
        JsonValue::Bool(true) => ToolDecision::Allow,
        JsonValue::Bool(false) => ToolDecision::Deny,
        JsonValue::String(value) if is_positive_decision(value) => ToolDecision::Allow,
        JsonValue::String(value) if is_negative_decision(value) => ToolDecision::Deny,
        JsonValue::Number(number) if number.as_i64() == Some(1) => ToolDecision::Allow,
        JsonValue::Number(number) if number.as_i64() == Some(0) => ToolDecision::Deny,
        JsonValue::Object(object) => nested_tool_decision(object),
        _ => ToolDecision::Unknown,
    }
}

fn nested_tool_decision(object: &JsonMap<String, JsonValue>) -> ToolDecision {
    if object
        .values()
        .any(|value| tool_value_decision(value) == ToolDecision::Deny)
    {
        return ToolDecision::Deny;
    }
    if object
        .get("*")
        .map(|value| tool_value_decision(value) == ToolDecision::Allow)
        .unwrap_or(false)
    {
        return ToolDecision::Allow;
    }
    ToolDecision::Deny
}

fn is_positive_decision(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "allow" | "allowed" | "true" | "yes" | "on" | "enabled"
    )
}

fn is_negative_decision(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "deny" | "denied" | "false" | "no" | "off" | "disabled" | "ask" | "prompt"
    )
}

fn split_string_list(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() && !values.iter().any(|existing| existing == trimmed) {
        values.push(trimmed.to_string());
    }
}

fn error_issue(
    context: &ConversionContext,
    kind: ImportKind,
    path: &Path,
    message: impl Into<String>,
) -> ImportIssue {
    issue(context, kind, path, ImportStatus::Error, message)
}

fn unsupported_issue(
    context: &ConversionContext,
    kind: ImportKind,
    path: &Path,
    message: impl Into<String>,
) -> ImportIssue {
    issue(context, kind, path, ImportStatus::Unsupported, message)
}

fn issue(
    context: &ConversionContext,
    kind: ImportKind,
    path: &Path,
    status: ImportStatus,
    message: impl Into<String>,
) -> ImportIssue {
    ImportIssue {
        competitor: Some(context.competitor),
        kind: Some(kind),
        scope: Some(context.scope.clone()),
        path: Some(path.to_path_buf()),
        status,
        message: message.into(),
    }
}

fn insert_string(metadata: &mut JsonMap<String, JsonValue>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        metadata.insert(key.to_string(), JsonValue::String(value.trim().to_string()));
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.trim().to_string())
    }
}

fn first_non_empty<'a>(values: &[&'a str]) -> &'a str {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or("")
}

trait StringIfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl StringIfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.trim().is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::markdown::{yaml_string as markdown_yaml_string, yaml_string_list_any};
    use crate::yaml_configs::customization_types::SubagentConfig;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn file_content(candidate: &ImportCandidate) -> &str {
        match &candidate.artifact {
            super::super::super::types::ImportArtifact::FileContent { content } => content,
            _ => panic!("expected file content"),
        }
    }

    fn subagent_config(candidate: &ImportCandidate) -> SubagentConfig {
        serde_yaml::from_str(file_content(candidate)).unwrap()
    }

    fn find_candidate<'a>(
        scan: &'a OpenCodeScan,
        kind: ImportKind,
        dest_name: &str,
    ) -> &'a ImportCandidate {
        scan.candidates
            .iter()
            .find(|candidate| candidate.kind == kind && candidate.dest_name == dest_name)
            .unwrap_or_else(|| panic!("missing {kind:?} candidate {dest_name}"))
    }

    #[test]
    fn project_skill_package_imports_as_skill() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join(".opencode").join("skills").join("foo");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Foo Skill\ndescription: Helps with foo\n---\n# Foo\nUse foo.",
        )
        .unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Skill, "foo-skill");
        assert_eq!(
            candidate.destination_path,
            PathBuf::from("skills/foo-skill")
        );
        assert!(scan.issues.is_empty());
    }

    #[test]
    fn project_markdown_command_imports_as_command() {
        let temp = tempfile::tempdir().unwrap();
        let command_path = temp
            .path()
            .join(".opencode")
            .join("commands")
            .join("review.md");
        fs::create_dir_all(command_path.parent().unwrap()).unwrap();
        fs::write(
            &command_path,
            "---\ndescription: Review code\nagent: reviewer\nmodel: claude\nsubtask: true\n---\nReview $ARGUMENTS",
        )
        .unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Command, "review");
        assert_eq!(
            candidate.destination_path,
            PathBuf::from("commands/review.md")
        );
        let (frontmatter, body) = parse_markdown_frontmatter(file_content(candidate));
        assert_eq!(
            markdown_yaml_string(&frontmatter, "description"),
            "Review code"
        );
        assert_eq!(markdown_yaml_string(&frontmatter, "model"), "claude");
        assert!(frontmatter.get("agent").is_none());
        assert!(frontmatter.get("subtask").is_none());
        assert_eq!(body, "Review $ARGUMENTS");
        assert!(candidate.metadata.get("competitor_fields").is_some());
    }

    #[test]
    fn markdown_agent_imports_as_schema_valid_subagent() {
        let temp = tempfile::tempdir().unwrap();
        let agent_path = temp
            .path()
            .join(".opencode")
            .join("agents")
            .join("explore.md");
        fs::create_dir_all(agent_path.parent().unwrap()).unwrap();
        fs::write(
            &agent_path,
            "---\ndescription: Explore code\nmode: subagent\nmodel: claude\ntools:\n  - read\n  - grep\nmaxSteps: 6\n---\nExplore the repository.",
        )
        .unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Subagent, "explore");
        let config = subagent_config(candidate);
        assert_eq!(config.id, "explore");
        assert_eq!(config.description, "Explore code");
        assert_eq!(config.subchat.max_steps, Some(6));
        assert_eq!(config.subchat.model.as_deref(), Some("claude"));
        assert_eq!(config.tools, strings(&["cat", "search_pattern"]));
    }

    #[test]
    fn markdown_agent_denied_edit_and_bash_removes_write_and_shell_tools() {
        let temp = tempfile::tempdir().unwrap();
        let agent_path = temp.path().join(".opencode").join("agents").join("safe.md");
        fs::create_dir_all(agent_path.parent().unwrap()).unwrap();
        fs::write(
            &agent_path,
            "---\ndescription: Safe agent\ntools:\n  - all\npermission:\n  edit: deny\n  bash: deny\n---\nStay safe.",
        )
        .unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Subagent, "safe");
        let config = subagent_config(candidate);
        assert!(!config.tools.contains(&"apply_patch".to_string()));
        assert!(!config.tools.contains(&"shell".to_string()));
        assert!(config.tools.contains(&"cat".to_string()));
    }

    #[test]
    fn markdown_agent_steps_maps_to_max_steps() {
        let temp = tempfile::tempdir().unwrap();
        let agent_path = temp
            .path()
            .join(".opencode")
            .join("agent")
            .join("planner.md");
        fs::create_dir_all(agent_path.parent().unwrap()).unwrap();
        fs::write(
            &agent_path,
            "---\ndescription: Planner\nsteps: 4\n---\nPlan carefully.",
        )
        .unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let candidate = find_candidate(&scan, ImportKind::Subagent, "planner");
        let config = subagent_config(candidate);
        assert_eq!(config.subchat.max_steps, Some(4));
    }

    #[test]
    fn valid_opencode_json_agent_and_command_entries_import() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("opencode.json"),
            serde_json::json!({
                "command": {
                    "review": {
                        "description": "Review code",
                        "template": "Review the diff",
                        "agent": "reviewer",
                        "model": "claude",
                        "subtask": "true"
                    }
                },
                "agent": {
                    "reviewer": {
                        "description": "Reviews code",
                        "mode": "subagent",
                        "prompt": "You review code.",
                        "tools": { "read": true, "edit": true, "bash": true },
                        "permission": { "edit": "deny", "bash": "deny" },
                        "maxSteps": 5,
                        "model": "claude"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let command = find_candidate(&scan, ImportKind::Command, "review");
        let (frontmatter, body) = parse_markdown_frontmatter(file_content(command));
        assert_eq!(
            markdown_yaml_string(&frontmatter, "description"),
            "Review code"
        );
        assert_eq!(body, "Review the diff");
        let agent = find_candidate(&scan, ImportKind::Subagent, "reviewer");
        let config = subagent_config(agent);
        assert_eq!(config.subchat.max_steps, Some(5));
        assert_eq!(config.subchat.model.as_deref(), Some("claude"));
        assert_eq!(config.tools, strings(&["cat"]));
        assert!(scan.issues.is_empty());
    }

    #[test]
    fn invalid_opencode_jsonc_is_reported_without_blocking_markdown_imports() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("opencode.jsonc"), "{ command: [ }").unwrap();
        let command_path = temp
            .path()
            .join(".opencode")
            .join("commands")
            .join("review.md");
        fs::create_dir_all(command_path.parent().unwrap()).unwrap();
        fs::write(&command_path, "Review anyway").unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        find_candidate(&scan, ImportKind::Command, "review");
        assert_eq!(scan.issues.len(), 1);
        assert_eq!(scan.issues[0].status, ImportStatus::Error);
    }

    #[test]
    fn jsonc_parser_strips_comments_and_trailing_commas() {
        let input = r#"
        {
            // line comment
            "url": "https://example.com//kept",
            "command": {
                "review": {
                    "template": "Keep /* this */ text",
                },
            },
            "tools": ["read", "grep",],
            /* block comment */
        }
        "#;

        let stripped = strip_jsonc(input).unwrap();
        let value: JsonValue = serde_json::from_str(&stripped).unwrap();

        assert_eq!(
            value["command"]["review"]["template"],
            JsonValue::String("Keep /* this */ text".to_string())
        );
        assert_eq!(value["tools"], serde_json::json!(["read", "grep"]));
    }

    #[test]
    fn config_entries_without_safe_mapping_are_reported_as_unsupported() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("opencode.json"),
            serde_json::json!({
                "agent": { "primary": { "mode": "primary", "prompt": "Do things" } },
                "command": { "empty": { "description": "No body" } }
            })
            .to_string(),
        )
        .unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        assert!(scan.candidates.is_empty());
        assert_eq!(scan.issues.len(), 2);
        assert!(scan
            .issues
            .iter()
            .all(|issue| issue.status == ImportStatus::Unsupported));
    }

    #[test]
    fn global_scan_uses_opencode_config_root() {
        let temp = tempfile::tempdir().unwrap();
        let config_root = temp.path().join("opencode");
        fs::create_dir_all(config_root.join("command")).unwrap();
        fs::write(
            config_root.join("command").join("summarize.md"),
            "Summarize",
        )
        .unwrap();
        fs::write(
            config_root.join("config.json"),
            serde_json::json!({
                "command": { "explain": "Explain this" }
            })
            .to_string(),
        )
        .unwrap();

        let scan = scan_global_root_with_staging(&config_root, &temp.path().join("staging"));

        assert!(scan
            .candidates
            .iter()
            .all(|candidate| candidate.scope == ImportScope::Global));
        find_candidate(&scan, ImportKind::Command, "summarize");
        find_candidate(&scan, ImportKind::Command, "explain");
    }

    #[tokio::test]
    async fn import_project_root_writes_candidates_through_shared_writer() {
        let temp = tempfile::tempdir().unwrap();
        let command_path = temp
            .path()
            .join(".opencode")
            .join("commands")
            .join("review.md");
        fs::create_dir_all(command_path.parent().unwrap()).unwrap();
        fs::write(&command_path, "Review").unwrap();

        let summary = import_project_root(temp.path()).await;

        assert_eq!(summary.outcomes.len(), 1);
        assert_eq!(
            fs::read_to_string(
                temp.path()
                    .join(".refact")
                    .join("commands")
                    .join("review.md")
            )
            .unwrap(),
            "Review"
        );
    }

    #[test]
    fn scan_summary_omits_artifact_content() {
        let temp = tempfile::tempdir().unwrap();
        let command_path = temp
            .path()
            .join(".opencode")
            .join("commands")
            .join("secret.md");
        fs::create_dir_all(command_path.parent().unwrap()).unwrap();
        fs::write(&command_path, "secret generated content").unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));
        let summary_json = serde_json::to_string(&scan.to_summary()).unwrap();

        assert!(!summary_json.contains("secret generated content"));
        assert!(summary_json.contains("secret.md"));
    }

    #[test]
    fn command_tools_object_maps_allowed_tools() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("opencode.json"),
            serde_json::json!({
                "command": {
                    "search": {
                        "template": "Search",
                        "tools": { "read": true, "grep": true, "edit": false }
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let scan = scan_project_root_with_staging(temp.path(), &temp.path().join("staging"));

        let command = find_candidate(&scan, ImportKind::Command, "search");
        let (frontmatter, _) = parse_markdown_frontmatter(file_content(command));
        assert_eq!(
            yaml_string_list_any(&frontmatter, &["allowed-tools"]),
            strings(&["cat", "search_pattern"])
        );
    }
}
