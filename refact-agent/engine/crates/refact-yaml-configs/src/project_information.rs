use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars_per_item: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub overrides: HashMap<String, FileOverride>,
}

impl Default for SectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_chars: None,
            max_items: None,
            max_chars_per_item: None,
            max_depth: None,
            overrides: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInformationDefaults {
    pub max_chars_per_item: usize,
    pub max_items_per_section: usize,
}

impl Default for ProjectInformationDefaults {
    fn default() -> Self {
        Self {
            max_chars_per_item: 8000,
            max_items_per_section: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInformationSections {
    #[serde(default)]
    pub system_info: SectionConfig,
    #[serde(default)]
    pub environment_instructions: SectionConfig,
    #[serde(default)]
    pub detected_environments: SectionConfig,
    #[serde(default)]
    pub git_info: SectionConfig,
    #[serde(default)]
    pub project_tree: SectionConfig,
    #[serde(default)]
    pub instruction_files: SectionConfig,
    #[serde(default)]
    pub project_configs: SectionConfig,
    #[serde(default)]
    pub memories: SectionConfig,
}

impl Default for ProjectInformationSections {
    fn default() -> Self {
        Self {
            system_info: SectionConfig {
                enabled: true,
                ..Default::default()
            },
            environment_instructions: SectionConfig {
                enabled: true,
                max_chars: Some(6000),
                ..Default::default()
            },
            detected_environments: SectionConfig {
                enabled: true,
                max_items: Some(50),
                ..Default::default()
            },
            git_info: SectionConfig {
                enabled: true,
                max_chars: Some(6000),
                ..Default::default()
            },
            project_tree: SectionConfig {
                enabled: true,
                max_depth: Some(4),
                max_chars: Some(16000),
                ..Default::default()
            },
            instruction_files: SectionConfig {
                enabled: true,
                max_chars_per_item: Some(8000),
                ..Default::default()
            },
            project_configs: SectionConfig {
                enabled: true,
                max_items: Some(30),
                max_chars_per_item: Some(4000),
                ..Default::default()
            },
            memories: SectionConfig {
                enabled: true,
                max_items: Some(30),
                max_chars_per_item: Some(2000),
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInformationConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub defaults: ProjectInformationDefaults,
    #[serde(default)]
    pub sections: ProjectInformationSections,
}

fn default_schema_version() -> u32 {
    1
}
fn default_enabled() -> bool {
    true
}

impl Default for ProjectInformationConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: true,
            defaults: ProjectInformationDefaults::default(),
            sections: ProjectInformationSections::default(),
        }
    }
}

pub fn is_safe_relative_path(path: &str, project_roots: &[PathBuf]) -> bool {
    if path.is_empty() {
        return false;
    }

    let path_obj = Path::new(path);

    if path_obj.is_absolute() {
        return false;
    }

    use std::path::Component;
    for component in path_obj.components() {
        match component {
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return false;
            }
            _ => {}
        }
    }

    for root in project_roots {
        let full_path = root.join(path);
        if let Ok(canonical) = full_path.canonicalize() {
            if let Ok(root_canonical) = root.canonicalize() {
                let canonical = dunce::simplified(&canonical).to_path_buf();
                let root_canonical = dunce::simplified(&root_canonical).to_path_buf();
                if canonical.starts_with(&root_canonical) {
                    return true;
                }
            }
        }
    }

    !project_roots.is_empty()
}

pub fn to_relative_path(absolute_path: &str, project_roots: &[PathBuf]) -> Option<String> {
    let abs_path = Path::new(absolute_path);
    if !abs_path.is_absolute() {
        return Some(absolute_path.to_string());
    }

    for root in project_roots {
        if let Ok(root_canonical) = root.canonicalize() {
            if let Ok(path_canonical) = abs_path.canonicalize() {
                let root_canonical = dunce::simplified(&root_canonical).to_path_buf();
                let path_canonical = dunce::simplified(&path_canonical).to_path_buf();
                if let Ok(relative) = path_canonical.strip_prefix(&root_canonical) {
                    return Some(relative.to_string_lossy().replace('\\', "/"));
                }
            }
            if let Ok(relative) = abs_path.strip_prefix(root) {
                return Some(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    None
}

pub fn override_key(absolute_path: &str, project_roots: &[PathBuf]) -> String {
    to_relative_path(absolute_path, project_roots).unwrap_or_else(|| absolute_path.replace('\\', "/"))
}

fn is_under_any_root(path: &Path, roots: &[PathBuf]) -> bool {
    for root in roots {
        if path.starts_with(root) {
            return true;
        }
        if let (Ok(path_canonical), Ok(root_canonical)) = (path.canonicalize(), root.canonicalize()) {
            if dunce::simplified(&path_canonical).starts_with(dunce::simplified(&root_canonical)) {
                return true;
            }
        }
    }
    false
}

pub fn is_allowed_override_key(path: &str, allowed_roots: &[PathBuf]) -> bool {
    if path.is_empty() {
        return false;
    }
    let path_obj = Path::new(path);
    if path_obj.is_absolute() {
        return is_under_any_root(path_obj, allowed_roots);
    }
    is_safe_relative_path(path, allowed_roots)
}

pub fn sanitize_overrides(
    overrides: &HashMap<String, FileOverride>,
    allowed_roots: &[PathBuf],
) -> HashMap<String, FileOverride> {
    overrides
        .iter()
        .filter(|(path, _)| is_allowed_override_key(path, allowed_roots))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_to_relative_path_already_relative() {
        let roots = vec![PathBuf::from("/project")];
        assert_eq!(
            to_relative_path("src/main.rs", &roots),
            Some("src/main.rs".to_string())
        );
    }

    #[test]
    fn test_to_relative_path_absolute_within_root() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().to_path_buf();
        let file_path = root.join("src/main.rs");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, "test").unwrap();

        let roots = vec![root];
        let result = to_relative_path(&file_path.to_string_lossy(), &roots);
        assert_eq!(result, Some("src/main.rs".to_string()));
    }

    #[test]
    fn test_to_relative_path_outside_root() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();

        let outside_file = temp_dir.path().join("outside.txt");
        std::fs::write(&outside_file, "test").unwrap();

        let roots = vec![root];
        let result = to_relative_path(&outside_file.to_string_lossy(), &roots);
        assert_eq!(result, None);
    }

    #[test]
    fn test_is_safe_relative_path_valid() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().to_path_buf();
        let file_path = root.join("AGENTS.md");
        std::fs::write(&file_path, "test").unwrap();

        let roots = vec![root];
        assert!(is_safe_relative_path("AGENTS.md", &roots));
    }

    #[test]
    fn test_is_safe_relative_path_rejects_absolute() {
        let roots = vec![PathBuf::from("/project")];
        assert!(!is_safe_relative_path("/etc/passwd", &roots));
    }

    #[test]
    fn test_is_safe_relative_path_rejects_parent_traversal() {
        let roots = vec![PathBuf::from("/project")];
        assert!(!is_safe_relative_path("../etc/passwd", &roots));
        assert!(!is_safe_relative_path("foo/../../../etc/passwd", &roots));
    }

    #[test]
    fn test_is_safe_relative_path_empty() {
        let roots = vec![PathBuf::from("/project")];
        assert!(!is_safe_relative_path("", &roots));
    }

    #[test]
    fn test_sanitize_overrides_keeps_valid() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().to_path_buf();
        let file_path = root.join("AGENTS.md");
        std::fs::write(&file_path, "test").unwrap();

        let mut overrides = HashMap::new();
        overrides.insert(
            "AGENTS.md".to_string(),
            FileOverride {
                enabled: Some(false),
                max_chars: Some(1000),
            },
        );

        let roots = vec![root];
        let result = sanitize_overrides(&overrides, &roots);
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("AGENTS.md"));
    }

    #[test]
    fn test_sanitize_overrides_drops_absolute() {
        let roots = vec![PathBuf::from("/project")];
        let mut overrides = HashMap::new();
        overrides.insert(
            "/etc/passwd".to_string(),
            FileOverride {
                enabled: Some(false),
                max_chars: None,
            },
        );

        let result = sanitize_overrides(&overrides, &roots);
        assert!(result.is_empty());
    }

    #[test]
    fn test_sanitize_overrides_drops_parent_traversal() {
        let roots = vec![PathBuf::from("/project")];
        let mut overrides = HashMap::new();
        overrides.insert(
            "../secret.txt".to_string(),
            FileOverride {
                enabled: Some(false),
                max_chars: None,
            },
        );

        let result = sanitize_overrides(&overrides, &roots);
        assert!(result.is_empty());
    }

    #[test]
    fn test_sanitize_overrides_keeps_absolute_under_allowed_root() {
        let temp_dir = TempDir::new().unwrap();
        let external = temp_dir.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        let memo = external.join("memory.md");
        std::fs::write(&memo, "note").unwrap();

        let project_root = temp_dir.path().join("proj");
        std::fs::create_dir_all(&project_root).unwrap();

        let key = memo.to_string_lossy().replace('\\', "/");
        let mut overrides = HashMap::new();
        overrides.insert(
            key.clone(),
            FileOverride {
                enabled: Some(false),
                max_chars: None,
            },
        );

        let allowed_roots = vec![project_root, external];
        let result = sanitize_overrides(&overrides, &allowed_roots);
        assert!(result.contains_key(&key));
    }

    #[test]
    fn test_override_key_relative_in_root_absolute_out_of_root() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().to_path_buf();
        let in_root = root.join("sub").join("AGENTS.md");
        std::fs::create_dir_all(in_root.parent().unwrap()).unwrap();
        std::fs::write(&in_root, "x").unwrap();
        let roots = vec![root];
        assert_eq!(
            override_key(&in_root.to_string_lossy(), &roots),
            "sub/AGENTS.md".to_string()
        );

        let other = TempDir::new().unwrap();
        let outside = other.path().join("MEMORY.md");
        std::fs::write(&outside, "y").unwrap();
        assert_eq!(
            override_key(&outside.to_string_lossy(), &roots),
            outside.to_string_lossy().replace('\\', "/")
        );
    }

    #[test]
    fn test_default_config_has_reasonable_limits() {
        let config = ProjectInformationConfig::default();

        assert!(config.enabled);
        assert_eq!(config.schema_version, 1);

        assert!(config.sections.system_info.enabled);
        assert!(config.sections.git_info.enabled);
        assert_eq!(config.sections.git_info.max_chars, Some(6000));

        assert!(config.sections.project_tree.enabled);
        assert_eq!(config.sections.project_tree.max_depth, Some(4));
        assert_eq!(config.sections.project_tree.max_chars, Some(16000));

        assert!(config.sections.instruction_files.enabled);
        assert_eq!(config.sections.instruction_files.max_items, None);
        assert_eq!(
            config.sections.instruction_files.max_chars_per_item,
            Some(8000)
        );

        assert!(config.sections.memories.enabled);
        assert_eq!(config.sections.memories.max_items, Some(30));
        assert_eq!(config.sections.memories.max_chars_per_item, Some(2000));
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let config = ProjectInformationConfig::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: ProjectInformationConfig = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(config.enabled, parsed.enabled);
        assert_eq!(config.schema_version, parsed.schema_version);
        assert_eq!(
            config.sections.project_tree.max_chars,
            parsed.sections.project_tree.max_chars
        );
    }
}
