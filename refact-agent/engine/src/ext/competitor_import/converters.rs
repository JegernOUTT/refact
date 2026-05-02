use std::ffi::OsString;
use std::fs;
use std::io::{Error, ErrorKind, Read, Result as IoResult, Write};
use std::path::{Component, Path, PathBuf};

use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::yaml_configs::customization_types::SubagentConfig;

use super::manifest::{hash_directory, hash_string, MAX_SCAN_ENTRIES};
use super::markdown::{
    first_useful_line_or_heading, frontmatter_mapping, parse_markdown_frontmatter,
    render_markdown_with_frontmatter, sanitize_command_name, sanitize_skill_id,
    sanitize_subagent_id, set_yaml_string, set_yaml_string_list, yaml_string, yaml_string_any,
    yaml_string_list_any,
};
use super::tools::{map_allowed_tools, resolve_subagent_tools, ToolMappingResult};
use super::types::{
    ConversionContext, ConversionError, ImportArtifact, ImportCandidate, ImportKind,
    ImportPrivacyFilter, NormalizedSubagent,
};

pub(crate) const MAX_MARKDOWN_FILE_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_CONFIG_FILE_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_SKILL_PACKAGE_FILES: usize = 128;
pub(crate) const MAX_SKILL_PACKAGE_BYTES: u64 = 4 * 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 16 * 1024;

pub(crate) fn read_markdown_file_limited(path: &Path) -> IoResult<String> {
    read_text_file_limited(path, MAX_MARKDOWN_FILE_BYTES, "markdown file")
}

pub(crate) fn read_config_file_limited(path: &Path) -> IoResult<String> {
    read_text_file_limited(path, MAX_CONFIG_FILE_BYTES, "config file")
}

fn read_text_file_limited(path: &Path, max_bytes: u64, label: &str) -> IoResult<String> {
    let metadata = fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{label} is not a regular file: {}", path.display()),
        ));
    }
    if metadata.len() > max_bytes {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "{label} exceeds {max_bytes} byte limit: {} bytes",
                metadata.len()
            ),
        ));
    }
    read_text_file_with_expected_len(path, max_bytes, label, metadata.len())
}

fn read_text_file_with_expected_len(
    path: &Path,
    max_bytes: u64,
    label: &str,
    expected_len: u64,
) -> IoResult<String> {
    let capacity = usize::try_from(max_bytes).map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("{label} byte limit is too large: {max_bytes}"),
        )
    })?;
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut bytes = Vec::with_capacity(capacity);
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, format!("{label} size overflow")))?;
        if total > max_bytes {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("{label} exceeds {max_bytes} byte limit while reading: {total} bytes"),
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    if total != expected_len {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "{label} changed while reading: expected {expected_len} bytes, read {total} bytes"
            ),
        ));
    }
    String::from_utf8(bytes).map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()))
}

pub fn convert_command_markdown(
    context: &ConversionContext,
    source_path: &Path,
    content: &str,
    explicit_name: Option<&str>,
) -> Result<ImportCandidate, ConversionError> {
    convert_command_markdown_with_source_hash(
        context,
        source_path,
        content,
        explicit_name,
        hash_string(content),
    )
}

pub(crate) fn convert_command_markdown_with_source_hash(
    context: &ConversionContext,
    source_path: &Path,
    content: &str,
    explicit_name: Option<&str>,
    source_hash: String,
) -> Result<ImportCandidate, ConversionError> {
    let raw_name = explicit_name
        .filter(|name| !name.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            source_path
                .file_stem()
                .and_then(|name| name.to_str())
                .map(ToString::to_string)
        })
        .unwrap_or_default();
    let name = sanitize_command_name(&raw_name);
    if name.is_empty() {
        return Err(ConversionError::new(
            context,
            ImportKind::Command,
            source_path.to_path_buf(),
            "command name is empty after sanitization",
        ));
    }

    let (frontmatter, body) = parse_markdown_frontmatter(content);
    let description = yaml_string(&frontmatter, "description");
    let argument_hint = yaml_string_any(&frontmatter, &["argument-hint", "argument_hint"]);
    let model = yaml_string(&frontmatter, "model");
    let allowed_tools = yaml_string_list_any(&frontmatter, &["allowed-tools", "allowed_tools"]);
    let mapped_tools = map_allowed_tools(&allowed_tools);

    let mut rendered_frontmatter = Mapping::new();
    set_yaml_string(&mut rendered_frontmatter, "description", &description);
    set_yaml_string(&mut rendered_frontmatter, "argument-hint", &argument_hint);
    set_yaml_string_list(
        &mut rendered_frontmatter,
        "allowed-tools",
        &mapped_tools.tools,
    );
    set_yaml_string(&mut rendered_frontmatter, "model", &model);

    let rendered =
        render_markdown_with_frontmatter(&rendered_frontmatter, &body).map_err(|err| {
            ConversionError::new(
                context,
                ImportKind::Command,
                source_path.to_path_buf(),
                format!("failed to render command markdown: {err}"),
            )
        })?;

    let mut metadata = JsonMap::new();
    insert_string(&mut metadata, "original_name", &raw_name);
    add_tool_metadata(&mut metadata, &mapped_tools);
    let competitor_fields = frontmatter_fields_to_metadata(&frontmatter, &["agent", "subtask"]);
    if !competitor_fields.is_empty() {
        metadata.insert(
            "competitor_fields".to_string(),
            JsonValue::Object(competitor_fields),
        );
    }

    Ok(ImportCandidate {
        competitor: context.competitor,
        kind: ImportKind::Command,
        scope: context.scope.clone(),
        source_root: context.source_root.clone(),
        source_path: source_path.to_path_buf(),
        dest_name: name.clone(),
        destination_path: PathBuf::from("commands").join(format!("{name}.md")),
        source_hash,
        artifact_hash: hash_string(&rendered),
        artifact: ImportArtifact::FileContent { content: rendered },
        metadata: JsonValue::Object(metadata),
    })
}

pub fn convert_skill_package(
    context: &ConversionContext,
    skill_dir: &Path,
    staging_root: &Path,
) -> Result<ImportCandidate, ConversionError> {
    validate_skill_package_limits(skill_dir).map_err(|err| {
        ConversionError::new(
            context,
            ImportKind::Skill,
            skill_dir.to_path_buf(),
            format!("skill package skipped: {err}"),
        )
    })?;
    let skill_md = skill_dir.join("SKILL.md");
    let content = read_markdown_file_limited(&skill_md).map_err(|err| {
        ConversionError::new(
            context,
            ImportKind::Skill,
            skill_md.clone(),
            format!("failed to read SKILL.md: {err}"),
        )
    })?;
    let (frontmatter, body) = parse_markdown_frontmatter(&content);
    let source_dir_name = skill_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let original_name = yaml_string(&frontmatter, "name");
    let body_name = first_useful_line_or_heading(&body).unwrap_or_default();
    let name_basis =
        first_non_empty(&[original_name.as_str(), source_dir_name, body_name.as_str()]);
    let skill_id = sanitize_skill_id(name_basis);
    if skill_id.is_empty() {
        return Err(ConversionError::new(
            context,
            ImportKind::Skill,
            skill_dir.to_path_buf(),
            "skill name is empty after sanitization",
        ));
    }

    let original_description = yaml_string(&frontmatter, "description");
    let description = if original_description.is_empty() {
        first_useful_line_or_heading(&body).unwrap_or_else(|| format!("Imported skill {skill_id}"))
    } else {
        original_description.clone()
    };

    let mut updated_frontmatter = frontmatter_mapping(&frontmatter);
    set_yaml_string(&mut updated_frontmatter, "name", &skill_id);
    set_yaml_string(&mut updated_frontmatter, "description", &description);
    let normalized_content = render_markdown_with_frontmatter(&updated_frontmatter, &body)
        .map_err(|err| {
            ConversionError::new(
                context,
                ImportKind::Skill,
                skill_md.clone(),
                format!("failed to render SKILL.md: {err}"),
            )
        })?;
    let needs_rewrite = normalized_content != content;
    let source_dir = stage_skill_package(skill_dir, staging_root, &skill_id, &normalized_content)
        .map_err(|err| {
        ConversionError::new(
            context,
            ImportKind::Skill,
            skill_dir.to_path_buf(),
            format!("failed to stage generated skill package: {err}"),
        )
    })?;
    let directory_hash = hash_directory(&source_dir).map_err(|err| {
        ConversionError::new(
            context,
            ImportKind::Skill,
            skill_dir.to_path_buf(),
            format!("failed to hash staged skill package: {err}"),
        )
    })?;

    let mut metadata = JsonMap::new();
    insert_string(&mut metadata, "original_name", &original_name);
    insert_string(&mut metadata, "generated_name", &skill_id);
    insert_string(&mut metadata, "original_description", &original_description);
    metadata.insert(
        "rewrote_skill_md".to_string(),
        JsonValue::Bool(needs_rewrite),
    );

    Ok(ImportCandidate {
        competitor: context.competitor,
        kind: ImportKind::Skill,
        scope: context.scope.clone(),
        source_root: context.source_root.clone(),
        source_path: skill_dir.to_path_buf(),
        dest_name: skill_id.clone(),
        destination_path: PathBuf::from("skills").join(&skill_id),
        artifact: ImportArtifact::DirectoryCopy { source_dir },
        source_hash: directory_hash.clone(),
        artifact_hash: directory_hash,
        metadata: JsonValue::Object(metadata),
    })
}

#[cfg(test)]
pub fn convert_subagent(
    context: &ConversionContext,
    source_path: &Path,
    input: &NormalizedSubagent,
) -> Result<ImportCandidate, ConversionError> {
    convert_subagent_with_source_hash(context, source_path, input, hash_string(&input.prompt))
}

pub(crate) fn convert_subagent_with_source_hash(
    context: &ConversionContext,
    source_path: &Path,
    input: &NormalizedSubagent,
    source_hash: String,
) -> Result<ImportCandidate, ConversionError> {
    let fallback_stem = source_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let id_basis = first_non_empty(&[input.id.as_str(), input.title.as_str(), fallback_stem]);
    let id = sanitize_subagent_id(id_basis);
    if id.is_empty() {
        return Err(ConversionError::new(
            context,
            ImportKind::Subagent,
            source_path.to_path_buf(),
            "subagent id is empty after sanitization",
        ));
    }

    let title = if input.title.trim().is_empty() {
        id.clone()
    } else {
        input.title.trim().to_string()
    };
    let description = if input.description.trim().is_empty() {
        first_useful_line_or_heading(&input.prompt).unwrap_or_else(|| format!("Run {title}"))
    } else {
        input.description.trim().to_string()
    };
    let max_steps = input.max_steps.filter(|steps| *steps > 0).unwrap_or(10);
    let mapped_tools = resolve_subagent_tools(&input.tool_policy);
    let yaml = render_subagent_yaml(
        &id,
        &title,
        &description,
        &input.prompt,
        &mapped_tools.tools,
        max_steps,
        input.model.as_deref(),
    )
    .map_err(|err| {
        ConversionError::new(
            context,
            ImportKind::Subagent,
            source_path.to_path_buf(),
            format!("failed to render subagent yaml: {err}"),
        )
    })?;
    serde_yaml::from_str::<SubagentConfig>(&yaml).map_err(|err| {
        ConversionError::new(
            context,
            ImportKind::Subagent,
            source_path.to_path_buf(),
            format!("generated subagent yaml is invalid: {err}"),
        )
    })?;

    let mut metadata = object_metadata(input.metadata.clone());
    insert_string(&mut metadata, "original_id", &input.id);
    insert_string(&mut metadata, "generated_id", &id);
    add_tool_metadata(&mut metadata, &mapped_tools);

    Ok(ImportCandidate {
        competitor: context.competitor,
        kind: ImportKind::Subagent,
        scope: context.scope.clone(),
        source_root: context.source_root.clone(),
        source_path: source_path.to_path_buf(),
        dest_name: id.clone(),
        destination_path: PathBuf::from("subagents").join(format!("{id}.yaml")),
        source_hash,
        artifact_hash: hash_string(&yaml),
        artifact: ImportArtifact::FileContent { content: yaml },
        metadata: JsonValue::Object(metadata),
    })
}

fn render_subagent_yaml(
    id: &str,
    title: &str,
    description: &str,
    prompt: &str,
    tools: &[String],
    max_steps: usize,
    model: Option<&str>,
) -> Result<String, String> {
    let mut yaml = String::new();
    yaml.push_str("schema_version: 2\n");
    yaml.push_str(&format!("id: {}\n", quoted(id)?));
    yaml.push_str(&format!("title: {}\n", quoted(title)?));
    yaml.push_str(&format!("description: {}\n", quoted(description)?));
    yaml.push_str("specific: false\n");
    yaml.push_str("expose_as_tool: true\n");
    yaml.push_str("has_code: false\n");
    yaml.push_str("tool:\n");
    yaml.push_str(&format!("  description: {}\n", quoted(description)?));
    yaml.push_str("  agentic: true\n");
    yaml.push_str("  allow_parallel: true\n");
    yaml.push_str("  parameters:\n");
    yaml.push_str("    - name: task\n");
    yaml.push_str("      type: string\n");
    yaml.push_str("      description: Task description for the subagent\n");
    yaml.push_str("  required:\n");
    yaml.push_str("    - task\n");
    yaml.push_str("subchat:\n");
    yaml.push_str("  context_mode: bare\n");
    yaml.push_str("  stateful: false\n");
    yaml.push_str(&format!("  max_steps: {max_steps}\n"));
    if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
        yaml.push_str(&format!("  model: {}\n", quoted(model.trim())?));
    }
    yaml.push_str("  model_type: default\n");
    yaml.push_str("messages:\n");
    yaml.push_str("  system_prompt: |\n");
    yaml.push_str(&indent_block(prompt.trim(), 4));
    yaml.push('\n');
    yaml.push_str("  user_template: |\n");
    yaml.push_str("    {{task}}\n");
    if !tools.is_empty() {
        yaml.push_str("tools:\n");
        for tool in tools {
            yaml.push_str(&format!("  - {tool}\n"));
        }
    }
    Ok(yaml)
}

pub(crate) fn validate_skill_package_privacy(
    skill_dir: &Path,
    filter: &ImportPrivacyFilter,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(skill_dir).map_err(|err| err.to_string())?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_dir() {
        return Err(format!(
            "skill package is not a regular directory: {}",
            skill_dir.display()
        ));
    }
    let mut entry_count = 0usize;
    for entry in walkdir::WalkDir::new(skill_dir)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path != skill_dir {
            entry_count += 1;
            if entry_count > MAX_SCAN_ENTRIES {
                return Err(format!(
                    "skill package scan capped after {MAX_SCAN_ENTRIES} filesystem entries"
                ));
            }
        }
        if path == skill_dir || !entry.file_type().is_file() {
            continue;
        }
        let metadata = fs::symlink_metadata(path).map_err(|err| err.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            continue;
        }
        filter.check_path(path)?;
    }
    Ok(())
}

fn validate_skill_package_limits(skill_dir: &Path) -> IoResult<()> {
    let metadata = fs::symlink_metadata(skill_dir)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "skill package is not a regular directory: {}",
                skill_dir.display()
            ),
        ));
    }
    let mut file_count = 0usize;
    let mut total_bytes = 0u64;
    let mut entry_count = 0usize;
    for entry in walkdir::WalkDir::new(skill_dir)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|err| Error::new(ErrorKind::Other, err.to_string()))?;
        let path = entry.path();
        if path != skill_dir {
            entry_count += 1;
            if entry_count > MAX_SCAN_ENTRIES {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "skill package scan capped after {MAX_SCAN_ENTRIES} filesystem entries"
                    ),
                ));
            }
        }
        if path == skill_dir || !entry.file_type().is_file() {
            continue;
        }
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            continue;
        }
        file_count += 1;
        if file_count > MAX_SKILL_PACKAGE_FILES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "skill package exceeds {MAX_SKILL_PACKAGE_FILES} file limit: {file_count} files"
                ),
            ));
        }
        let remaining = MAX_SKILL_PACKAGE_BYTES
            .checked_sub(total_bytes)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "skill package size overflow"))?;
        let read = count_file_bytes_limited(path, remaining)?;
        total_bytes = total_bytes
            .checked_add(read)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "skill package size overflow"))?;
        if total_bytes > MAX_SKILL_PACKAGE_BYTES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "skill package exceeds {MAX_SKILL_PACKAGE_BYTES} byte limit: {total_bytes} bytes"
                ),
            ));
        }
        if read != metadata.len() {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "skill package file changed while reading: {} expected {} bytes, read {read} bytes",
                    path.display(),
                    metadata.len()
                ),
            ));
        }
    }
    Ok(())
}

fn count_file_bytes_limited(path: &Path, max_bytes: u64) -> IoResult<u64> {
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "skill package size overflow"))?;
        if total > max_bytes {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("skill package exceeds {MAX_SKILL_PACKAGE_BYTES} byte limit"),
            ));
        }
    }
    Ok(total)
}

fn stage_skill_package(
    skill_dir: &Path,
    staging_root: &Path,
    skill_id: &str,
    normalized_skill_md: &str,
) -> IoResult<PathBuf> {
    let canonical_staging_root = create_dir_all_no_symlinks(staging_root)?;
    let hash = hash_string(&format!(
        "{}\n{}\n{}",
        skill_dir.to_string_lossy(),
        skill_id,
        normalized_skill_md
    ));
    let staged = canonical_staging_root.join(format!("{}-{}", skill_id, &hash[..16]));
    ensure_existing_components_are_not_symlinks(&staged)?;
    let canonical_parent = fs::canonicalize(staged.parent().unwrap_or(&canonical_staging_root))
        .map(|path| dunce::simplified(&path).to_path_buf())?;
    if !canonical_parent.starts_with(&canonical_staging_root) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "staged skill path escapes staging root: {}",
                staged.display()
            ),
        ));
    }
    remove_existing_path(&staged)?;
    create_dir_all_no_symlinks(&staged)?;
    copy_directory_contents(skill_dir, &staged)?;
    fs::write(staged.join("SKILL.md"), normalized_skill_md)?;
    Ok(staged)
}

fn create_dir_all_no_symlinks(path: &Path) -> IoResult<PathBuf> {
    reject_parent_components(path)?;

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut existing = absolute.clone();
    let mut missing = Vec::<OsString>::new();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "staging path component is not a regular directory: {}",
                            existing.display()
                        ),
                    ));
                }
                break;
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                let Some(name) = existing.file_name().map(|name| name.to_os_string()) else {
                    return Err(Error::new(
                        ErrorKind::NotFound,
                        format!("staging path has no existing parent: {}", path.display()),
                    ));
                };
                missing.push(name);
                if !existing.pop() {
                    return Err(Error::new(
                        ErrorKind::NotFound,
                        format!("staging path has no existing parent: {}", path.display()),
                    ));
                }
            }
            Err(err) => return Err(err),
        }
    }

    let mut current =
        fs::canonicalize(existing).map(|path| dunce::simplified(&path).to_path_buf())?;
    for component in missing.iter().rev() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "staging path component is not a regular directory: {}",
                            current.display()
                        ),
                    ));
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(err) => return Err(err),
        }
    }
    Ok(current)
}

fn reject_parent_components(path: &Path) -> IoResult<()> {
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("staging path contains parent component: {}", path.display()),
            ));
        }
    }
    Ok(())
}

fn ensure_existing_components_are_not_symlinks(path: &Path) -> IoResult<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                continue;
            }
            Component::RootDir => {
                current.push(component.as_os_str());
                continue;
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("staging path contains parent component: {}", path.display()),
                ));
            }
            Component::Normal(value) => current.push(value),
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("staging path component is a symlink: {}", current.display()),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn remove_existing_path(path: &Path) -> IoResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if metadata.file_type().is_symlink() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "refusing to remove symlinked staged path: {}",
                path.display()
            ),
        ));
    }
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn copy_directory_contents(source_dir: &Path, target_dir: &Path) -> IoResult<()> {
    let source_metadata = fs::symlink_metadata(source_dir)?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("source is not a directory: {}", source_dir.display()),
        ));
    }
    fs::create_dir_all(target_dir)?;
    let mut entry_count = 0usize;
    let mut file_count = 0usize;
    let mut total_bytes = 0u64;
    for entry in walkdir::WalkDir::new(source_dir)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|err| Error::new(ErrorKind::Other, err.to_string()))?;
        let source_path = entry.path();
        if source_path == source_dir {
            continue;
        }
        entry_count += 1;
        if entry_count > MAX_SCAN_ENTRIES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("skill package copy capped after {MAX_SCAN_ENTRIES} filesystem entries"),
            ));
        }
        if entry.file_type().is_symlink() {
            continue;
        }
        let relative = source_path
            .strip_prefix(source_dir)
            .map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()))?;
        let target_path = target_dir.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target_path)?;
        } else if entry.file_type().is_file() {
            file_count += 1;
            if file_count > MAX_SKILL_PACKAGE_FILES {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "skill package exceeds {MAX_SKILL_PACKAGE_FILES} file limit: {file_count} files"
                    ),
                ));
            }
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let remaining = MAX_SKILL_PACKAGE_BYTES
                .checked_sub(total_bytes)
                .ok_or_else(|| Error::new(ErrorKind::InvalidData, "skill package size overflow"))?;
            let copied = copy_file_limited_sync(source_path, &target_path, remaining)?;
            total_bytes = total_bytes
                .checked_add(copied)
                .ok_or_else(|| Error::new(ErrorKind::InvalidData, "skill package size overflow"))?;
            if total_bytes > MAX_SKILL_PACKAGE_BYTES {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "skill package exceeds {MAX_SKILL_PACKAGE_BYTES} byte limit: {total_bytes} bytes"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn copy_file_limited_sync(source_path: &Path, target_path: &Path, max_bytes: u64) -> IoResult<u64> {
    let metadata = fs::symlink_metadata(source_path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "copy source is not a regular file: {}",
                source_path.display()
            ),
        ));
    }
    if metadata.len() > max_bytes {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("skill package exceeds {MAX_SKILL_PACKAGE_BYTES} byte limit"),
        ));
    }
    let mut source = fs::File::open(source_path)?;
    let mut target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target_path)?;
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut copied = 0u64;
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "skill package size overflow"))?;
        if copied > max_bytes {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("skill package exceeds {MAX_SKILL_PACKAGE_BYTES} byte limit while copying"),
            ));
        }
        target.write_all(&buffer[..read])?;
    }
    target.flush()?;
    if copied != metadata.len() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "skill package file changed while copying: {} expected {} bytes, copied {copied} bytes",
                source_path.display(),
                metadata.len()
            ),
        ));
    }
    Ok(copied)
}

fn frontmatter_fields_to_metadata(
    frontmatter: &YamlValue,
    keys: &[&str],
) -> JsonMap<String, JsonValue> {
    let mut metadata = JsonMap::new();
    for key in keys {
        if let Some(value) = frontmatter.get(*key) {
            if let Ok(json_value) = serde_json::to_value(value) {
                metadata.insert((*key).to_string(), json_value);
            }
        }
    }
    metadata
}

fn add_tool_metadata(metadata: &mut JsonMap<String, JsonValue>, mapping: &ToolMappingResult) {
    insert_array(metadata, "unknown_tools", &mapping.unknown);
    insert_array(metadata, "denied_tools", &mapping.denied);
    if mapping.used_default {
        metadata.insert("defaulted_tools".to_string(), JsonValue::Bool(true));
    }
}

fn object_metadata(value: JsonValue) -> JsonMap<String, JsonValue> {
    match value {
        JsonValue::Object(map) => map,
        JsonValue::Null => JsonMap::new(),
        other => {
            let mut map = JsonMap::new();
            map.insert("source_metadata".to_string(), other);
            map
        }
    }
}

fn insert_string(metadata: &mut JsonMap<String, JsonValue>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        metadata.insert(key.to_string(), JsonValue::String(value.trim().to_string()));
    }
}

fn insert_array(metadata: &mut JsonMap<String, JsonValue>, key: &str, values: &[String]) {
    if !values.is_empty() {
        metadata.insert(
            key.to_string(),
            JsonValue::Array(
                values
                    .iter()
                    .map(|value| JsonValue::String(value.clone()))
                    .collect(),
            ),
        );
    }
}

fn first_non_empty<'a>(values: &[&'a str]) -> &'a str {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or("")
}

fn quoted(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| err.to_string())
}

fn indent_block(input: &str, spaces: usize) -> String {
    let indent = " ".repeat(spaces);
    if input.is_empty() {
        return indent;
    }
    input
        .lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::{Competitor, ImportScope, ToolPolicy};

    fn context(root: &Path) -> ConversionContext {
        ConversionContext {
            competitor: Competitor::ClaudeCode,
            scope: ImportScope::Global,
            source_root: root.to_path_buf(),
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn markdown_command_conversion_preserves_key_frontmatter_fields() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("Review Code.md");
        let content = "---\ndescription: Review changes\nargument-hint: \"<diff>\"\nallowed-tools:\n  - read\n  - grep\n  - unknown_tool\nmodel: gpt-4o\nagent: reviewer\nsubtask: code\n---\nReview $ARGUMENTS";
        fs::write(&source_path, content).unwrap();

        let candidate =
            convert_command_markdown(&context(temp.path()), &source_path, content, None).unwrap();

        assert_eq!(
            candidate.destination_path,
            PathBuf::from("commands/review-code.md")
        );
        let ImportArtifact::FileContent { content } = candidate.artifact else {
            panic!("expected file content");
        };
        let (frontmatter, body) = parse_markdown_frontmatter(&content);
        assert_eq!(yaml_string(&frontmatter, "description"), "Review changes");
        assert_eq!(yaml_string(&frontmatter, "argument-hint"), "<diff>");
        assert_eq!(
            yaml_string_list_any(&frontmatter, &["allowed-tools"]),
            strings(&["cat", "search_pattern"])
        );
        assert_eq!(yaml_string(&frontmatter, "model"), "gpt-4o");
        assert!(frontmatter.get("agent").is_none());
        assert_eq!(body, "Review $ARGUMENTS");
        assert_eq!(
            candidate.metadata.get("unknown_tools"),
            Some(&serde_json::json!(["unknown_tool"]))
        );
        assert_eq!(
            candidate
                .metadata
                .get("competitor_fields")
                .and_then(|value| value.get("agent")),
            Some(&serde_json::json!("reviewer"))
        );
    }

    #[test]
    fn skill_conversion_normalizes_name_without_mutating_source_files() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("source skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let original = "---\nname: My Skill\n---\n# Helps review code\nUse carefully.";
        fs::write(skill_dir.join("SKILL.md"), original).unwrap();
        fs::write(skill_dir.join("notes.txt"), "notes").unwrap();
        let staging = temp.path().join("staging");

        let candidate = convert_skill_package(&context(temp.path()), &skill_dir, &staging).unwrap();

        assert_eq!(candidate.destination_path, PathBuf::from("skills/my-skill"));
        assert_eq!(
            fs::read_to_string(skill_dir.join("SKILL.md")).unwrap(),
            original
        );
        let ImportArtifact::DirectoryCopy { source_dir } = candidate.artifact else {
            panic!("expected directory copy");
        };
        assert_ne!(source_dir, skill_dir);
        let staged_skill = fs::read_to_string(source_dir.join("SKILL.md")).unwrap();
        let (frontmatter, body) = parse_markdown_frontmatter(&staged_skill);
        assert_eq!(yaml_string(&frontmatter, "name"), "my-skill");
        assert_eq!(
            yaml_string(&frontmatter, "description"),
            "Helps review code"
        );
        assert_eq!(body, "# Helps review code\nUse carefully.");
        assert_eq!(
            fs::read_to_string(source_dir.join("notes.txt")).unwrap(),
            "notes"
        );
    }

    #[test]
    fn repeated_skill_conversion_reuses_staging_directory() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("source skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: My Skill\n---\n# Helps review code\nUse carefully.",
        )
        .unwrap();
        fs::write(skill_dir.join("notes.txt"), "notes").unwrap();
        let staging = temp.path().join("staging");

        let first = convert_skill_package(&context(temp.path()), &skill_dir, &staging).unwrap();
        let ImportArtifact::DirectoryCopy { source_dir } = first.artifact else {
            panic!("expected directory copy");
        };
        fs::write(source_dir.join("stale.txt"), "stale").unwrap();

        let second = convert_skill_package(&context(temp.path()), &skill_dir, &staging).unwrap();
        let ImportArtifact::DirectoryCopy {
            source_dir: second_source_dir,
        } = second.artifact
        else {
            panic!("expected directory copy");
        };

        assert_eq!(second_source_dir, source_dir);
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 1);
        assert!(!second_source_dir.join("stale.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_competitor_staging_root_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("source skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: My Skill\n---\n# Helps review code\nUse carefully.",
        )
        .unwrap();
        let staging = temp.path().join(".refact/imports/staging/claude");
        let outside = temp.path().join("outside");
        fs::create_dir_all(staging.parent().unwrap()).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &staging).unwrap();

        let err = convert_skill_package(&context(temp.path()), &skill_dir, &staging).unwrap_err();

        assert!(err.message.contains("staging"));
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_final_staged_path_is_rejected_without_removing_target() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("source skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: My Skill\n---\n# Helps review code\nUse carefully.",
        )
        .unwrap();
        let staging = temp.path().join("staging");
        let first = convert_skill_package(&context(temp.path()), &skill_dir, &staging).unwrap();
        let ImportArtifact::DirectoryCopy { source_dir } = first.artifact else {
            panic!("expected directory copy");
        };
        fs::remove_dir_all(&source_dir).unwrap();
        let victim = temp.path().join("victim");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("keep.txt"), "keep").unwrap();
        std::os::unix::fs::symlink(&victim, &source_dir).unwrap();

        let err = convert_skill_package(&context(temp.path()), &skill_dir, &staging).unwrap_err();

        assert!(err.message.contains("symlink"));
        assert_eq!(fs::read_to_string(victim.join("keep.txt")).unwrap(), "keep");
    }

    #[test]
    fn oversized_skill_package_bytes_are_rejected_before_staging() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("large skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Large\n---\nUse carefully.",
        )
        .unwrap();
        std::fs::File::create(skill_dir.join("large.bin"))
            .unwrap()
            .set_len(MAX_SKILL_PACKAGE_BYTES + 1)
            .unwrap();
        let staging = temp.path().join("staging");

        let err = convert_skill_package(&context(temp.path()), &skill_dir, &staging).unwrap_err();

        assert!(err.message.contains("byte limit"));
        assert!(!staging.exists());
    }

    #[test]
    fn text_file_limit_is_enforced_while_reading() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("growing.md");
        fs::write(&path, "x".repeat(32)).unwrap();

        let err = read_text_file_with_expected_len(&path, 16, "markdown file", 32).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(err.to_string().contains("while reading"));
    }

    #[test]
    fn changed_text_file_is_rejected_after_bounded_read() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("changed.md");
        fs::write(&path, "short").unwrap();

        let err = read_text_file_with_expected_len(&path, 64, "markdown file", 32).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(err.to_string().contains("changed while reading"));
    }

    #[test]
    fn subagent_converter_output_parses_as_subagent_config() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("agent.md");
        fs::write(&source_path, "agent source").unwrap();
        let input = NormalizedSubagent {
            id: "Code Reviewer".to_string(),
            title: "Code Reviewer".to_string(),
            description: "Reviews code".to_string(),
            prompt: "You review code.".to_string(),
            tool_policy: ToolPolicy {
                allowed: Some(strings(&["read", "bash", "edit"])),
                denied: strings(&["bash"]),
            },
            max_steps: Some(3),
            model: Some("gpt-4o".to_string()),
            metadata: serde_json::json!({"source": "unit"}),
        };

        let candidate = convert_subagent(&context(temp.path()), &source_path, &input).unwrap();

        assert_eq!(
            candidate.destination_path,
            PathBuf::from("subagents/code-reviewer.yaml")
        );
        let ImportArtifact::FileContent { content } = candidate.artifact else {
            panic!("expected file content");
        };
        let config: SubagentConfig = serde_yaml::from_str(&content).unwrap();
        assert_eq!(config.schema_version, 2);
        assert_eq!(config.id, "code-reviewer");
        assert!(config.expose_as_tool);
        let tool = config.tool.unwrap();
        assert!(tool.agentic);
        assert!(tool.allow_parallel);
        assert_eq!(tool.parameters.len(), 1);
        assert_eq!(tool.parameters[0].name, "task");
        assert_eq!(tool.required, vec!["task"]);
        assert_eq!(config.messages.user_template.as_deref(), Some("{{task}}\n"));
        assert_eq!(config.subchat.max_steps, Some(3));
        assert_eq!(config.subchat.model.as_deref(), Some("gpt-4o"));
        assert_eq!(config.tools, strings(&["cat", "apply_patch"]));
    }

    #[test]
    fn subagent_converter_defaults_missing_tools_to_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("agent.md");
        fs::write(&source_path, "agent source").unwrap();
        let input = NormalizedSubagent {
            id: "Research".to_string(),
            title: "Research".to_string(),
            description: String::new(),
            prompt: "# Research things".to_string(),
            tool_policy: ToolPolicy::missing(),
            max_steps: Some(0),
            model: None,
            metadata: JsonValue::Null,
        };

        let candidate = convert_subagent(&context(temp.path()), &source_path, &input).unwrap();
        let ImportArtifact::FileContent { content } = candidate.artifact else {
            panic!("expected file content");
        };
        let config: SubagentConfig = serde_yaml::from_str(&content).unwrap();

        assert_eq!(config.description, "Research things");
        assert_eq!(config.subchat.max_steps, Some(10));
        assert_eq!(config.tools, strings(&["tree", "cat", "search_pattern"]));
        assert_eq!(
            candidate.metadata.get("defaulted_tools"),
            Some(&JsonValue::Bool(true))
        );
    }
}
