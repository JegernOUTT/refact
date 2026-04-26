use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::ext::config_dirs::{collect_md_files_recursive, source_for_dir, CommandSource, ExtDirs};
use crate::ext::yaml_util::{yaml_str, yaml_str_list};

const MAX_FILE_SIZE: u64 = 100 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub argument_hint: String,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub body: String,
    pub source: CommandSource,
    #[serde(skip)]
    pub file_path: PathBuf,
}

pub fn parse_frontmatter_and_body(content: &str) -> (serde_yaml::Value, String) {
    let empty_map = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    if !content.starts_with("---") {
        return (empty_map, content.to_string());
    }
    let after_dashes = &content[3..];
    let rest = if after_dashes.starts_with('\n') {
        &after_dashes[1..]
    } else if after_dashes.starts_with("\r\n") {
        &after_dashes[2..]
    } else {
        return (empty_map, content.to_string());
    };
    let (frontmatter_str, body) = if rest.starts_with("---") {
        let after_close = &rest[3..];
        let body = if after_close.starts_with('\n') {
            &after_close[1..]
        } else if after_close.starts_with("\r\n") {
            &after_close[2..]
        } else {
            after_close
        };
        ("", body)
    } else {
        let end_marker = "\n---";
        match rest.find(end_marker) {
            Some(end_pos) => {
                let fm = &rest[..end_pos];
                let after_end = &rest[end_pos + end_marker.len()..];
                let body = if after_end.starts_with('\n') {
                    &after_end[1..]
                } else if after_end.starts_with("\r\n") {
                    &after_end[2..]
                } else {
                    after_end
                };
                (fm, body)
            }
            None => return (empty_map, content.to_string()),
        }
    };
    if frontmatter_str.is_empty() {
        return (empty_map, body.to_string());
    }
    match serde_yaml::from_str::<serde_yaml::Value>(frontmatter_str) {
        Ok(v) => (v, body.to_string()),
        Err(e) => {
            tracing::warn!("Failed to parse frontmatter YAML: {}", e);
            (empty_map, body.to_string())
        }
    }
}

fn command_name_from_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

async fn load_command_from_file(path: &Path, source: CommandSource) -> Option<SlashCommand> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    if metadata.len() > MAX_FILE_SIZE {
        tracing::warn!("Skipping slash command file > 100KB: {:?}", path);
        return None;
    }
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read slash command file {:?}: {}", path, e);
            return None;
        }
    };
    let name = command_name_from_path(path)?;
    let (fm, body) = parse_frontmatter_and_body(&content);
    let description = yaml_str(&fm, "description");
    let argument_hint = yaml_str(&fm, "argument-hint");
    let allowed_tools = yaml_str_list(&fm, "allowed-tools");
    let model = fm
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(SlashCommand {
        name,
        description,
        argument_hint,
        allowed_tools,
        model,
        body,
        source,
        file_path: path.to_path_buf(),
    })
}

pub async fn load_slash_commands(ext_dirs: &ExtDirs) -> Vec<SlashCommand> {
    let mut commands: HashMap<String, SlashCommand> = HashMap::new();
    for dir in ext_dirs.all_dirs_in_order() {
        let commands_dir = dir.join("commands");
        let source = source_for_dir(dir, &ext_dirs.global_dirs, &ext_dirs.installed_dirs);
        let files = collect_md_files_recursive(&commands_dir).await;
        for file in files {
            if let Some(cmd) = load_command_from_file(&file, source.clone()).await {
                commands.insert(cmd.name.clone(), cmd);
            }
        }
    }
    let mut result: Vec<SlashCommand> = commands.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::ext::yaml_util::{yaml_str, yaml_str_list};

    #[test]
    fn test_parse_frontmatter_valid() {
        let content = "---\ndescription: My command\nargument-hint: \"<query>\"\nallowed-tools:\n  - tool1\n  - tool2\nmodel: gpt-4o\n---\nBody text here";
        let (fm, body) = parse_frontmatter_and_body(content);
        assert_eq!(yaml_str(&fm, "description"), "My command");
        assert_eq!(yaml_str(&fm, "argument-hint"), "<query>");
        assert_eq!(yaml_str_list(&fm, "allowed-tools"), vec!["tool1", "tool2"]);
        assert_eq!(fm.get("model").and_then(|v| v.as_str()), Some("gpt-4o"));
        assert_eq!(body, "Body text here");
    }

    #[test]
    fn test_parse_frontmatter_missing() {
        let content = "Just a body with no frontmatter\nSecond line";
        let (fm, body) = parse_frontmatter_and_body(content);
        assert!(fm.as_mapping().map(|m| m.is_empty()).unwrap_or(true));
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_frontmatter_empty() {
        let content = "---\n---\nBody only";
        let (fm, body) = parse_frontmatter_and_body(content);
        assert!(fm.as_mapping().map(|m| m.is_empty()).unwrap_or(true));
        assert_eq!(body, "Body only");
    }

    #[test]
    fn test_parse_frontmatter_no_closing() {
        let content = "---\ndescription: unclosed\nno closing dashes";
        let (_, body) = parse_frontmatter_and_body(content);
        assert_eq!(body, content);
    }

    #[test]
    fn test_command_name_from_path() {
        assert_eq!(
            command_name_from_path(Path::new("/config/commands/format.md")),
            Some("format".to_string())
        );
        assert_eq!(
            command_name_from_path(Path::new("/config/commands/docs/review.md")),
            Some("review".to_string())
        );
        assert_eq!(
            command_name_from_path(Path::new("/config/commands/my-command.md")),
            Some("my-command".to_string())
        );
    }

    #[test]
    fn test_parse_frontmatter_unknown_fields_ignored() {
        let content = "---\ndescription: My command\nunknown_field: value\nanother: 123\n---\nBody";
        let (fm, body) = parse_frontmatter_and_body(content);
        assert_eq!(yaml_str(&fm, "description"), "My command");
        assert_eq!(body, "Body");
    }

    #[tokio::test]
    async fn test_load_slash_commands_from_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();

        tokio::fs::write(
            commands_dir.join("greet.md"),
            "---\ndescription: Greet someone\nargument-hint: \"<name>\"\n---\nHello $ARGUMENTS!",
        )
        .await
        .unwrap();
        tokio::fs::write(
            commands_dir.join("review.md"),
            "Please review the code: $ARGUMENTS",
        )
        .await
        .unwrap();

        let ext_dirs = crate::ext::config_dirs::ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let commands = load_slash_commands(&ext_dirs).await;
        assert_eq!(commands.len(), 2);

        let greet = commands.iter().find(|c| c.name == "greet").unwrap();
        assert_eq!(greet.description, "Greet someone");
        assert_eq!(greet.argument_hint, "<name>");
        assert_eq!(greet.body, "Hello $ARGUMENTS!");

        let review = commands.iter().find(|c| c.name == "review").unwrap();
        assert!(review.description.is_empty());
        assert_eq!(review.body, "Please review the code: $ARGUMENTS");
    }

    #[tokio::test]
    async fn test_load_slash_commands_precedence() {
        let global_tmp = tempfile::tempdir().unwrap();
        let project_tmp = tempfile::tempdir().unwrap();

        let global_cmds = global_tmp.path().join("commands");
        let project_cmds = project_tmp.path().join("commands");
        tokio::fs::create_dir_all(&global_cmds).await.unwrap();
        tokio::fs::create_dir_all(&project_cmds).await.unwrap();

        tokio::fs::write(
            global_cmds.join("deploy.md"),
            "---\ndescription: Global deploy\n---\nDeploy globally",
        )
        .await
        .unwrap();
        tokio::fs::write(
            project_cmds.join("deploy.md"),
            "---\ndescription: Project deploy\n---\nDeploy project",
        )
        .await
        .unwrap();

        let ext_dirs = crate::ext::config_dirs::ExtDirs {
            global_dirs: vec![global_tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![project_tmp.path().to_path_buf()],
        };

        let commands = load_slash_commands(&ext_dirs).await;
        assert_eq!(commands.len(), 1);
        let deploy = &commands[0];
        assert_eq!(deploy.description, "Project deploy");
    }

    #[tokio::test]
    async fn test_load_slash_commands_subdir_name_from_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let subdir = tmp.path().join("commands").join("docs");
        tokio::fs::create_dir_all(&subdir).await.unwrap();
        tokio::fs::write(subdir.join("format.md"), "Format the code")
            .await
            .unwrap();

        let ext_dirs = crate::ext::config_dirs::ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let commands = load_slash_commands(&ext_dirs).await;
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "format");
    }

    #[tokio::test]
    async fn test_load_slash_commands_missing_dir() {
        let ext_dirs = crate::ext::config_dirs::ExtDirs {
            global_dirs: vec![PathBuf::from("/nonexistent/path/that/does/not/exist")],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let commands = load_slash_commands(&ext_dirs).await;
        assert!(commands.is_empty());
    }
}
