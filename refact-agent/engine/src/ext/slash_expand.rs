use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::ext::config_dirs::{get_ext_dirs, ExtDirs};
use crate::ext::skills::load_skill_full;
use crate::ext::slash_commands::load_slash_commands;
use crate::global_context::GlobalContext;

pub struct ExpandedCommand {
    pub expanded_text: String,
    pub model_override: Option<String>,
    pub allowed_tools: Vec<String>,
    pub source_command: String,
    pub context_fork: Option<String>,
}

fn shell_split(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_double = false;
    let mut in_single = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_double {
            if c == '"' {
                in_double = false;
            } else {
                current.push(c);
            }
        } else if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_double = true;
        } else if c == '\'' {
            in_single = true;
        } else if c == ' ' || c == '\t' {
            if !current.is_empty() {
                args.push(current.clone());
                current.clear();
            }
        } else {
            current.push(c);
        }
        i += 1;
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn expand_template(body: &str, args_str: &str, positional: &[String]) -> String {
    let mut result = body.to_string();
    result = result.replace("$ARGUMENTS", args_str);
    for i in (0..positional.len()).rev() {
        let placeholder = format!("${}", i + 1);
        result = result.replace(&placeholder, &positional[i]);
    }
    let max_seen = positional.len() + 3;
    for i in (positional.len()..max_seen).rev() {
        let placeholder = format!("${}", i + 1);
        result = result.replace(&placeholder, "");
    }
    result
}

async fn expand_with_dirs(ext_dirs: &ExtDirs, raw_input: &str) -> Result<Option<ExpandedCommand>, String> {
    let trimmed = raw_input.trim_start();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }

    let after_slash = &trimmed[1..];
    let cmd_name_end = after_slash
        .find(|c: char| c == ' ' || c == '\t' || c == '\n')
        .unwrap_or(after_slash.len());
    let cmd_name = &after_slash[..cmd_name_end];

    if cmd_name.is_empty() {
        return Ok(None);
    }

    let args_str = after_slash[cmd_name_end..].trim().to_string();
    let positional = shell_split(&args_str);

    let commands = load_slash_commands(ext_dirs).await;
    if let Some(command) = commands.into_iter().find(|c| c.name == cmd_name) {
        let expanded_text = expand_template(&command.body, &args_str, &positional);
        return Ok(Some(ExpandedCommand {
            expanded_text,
            model_override: command.model,
            allowed_tools: command.allowed_tools,
            source_command: cmd_name.to_string(),
            context_fork: None,
        }));
    }

    if let Some(skill) = load_skill_full(ext_dirs, cmd_name).await {
        if !skill.index.user_invocable {
            return Ok(None);
        }
        let agent_name = skill.agent.clone().unwrap_or_else(|| "subagent".to_string());
        let context_fork = if skill.context.as_deref() == Some("fork") {
            Some(agent_name)
        } else {
            None
        };
        let expanded_text = expand_template(&skill.body, &args_str, &positional);
        return Ok(Some(ExpandedCommand {
            expanded_text,
            model_override: skill.model.clone(),
            allowed_tools: skill.allowed_tools.clone(),
            source_command: cmd_name.to_string(),
            context_fork,
        }));
    }

    Ok(None)
}

pub async fn expand_slash_command(
    gcx: Arc<ARwLock<GlobalContext>>,
    raw_input: &str,
) -> Result<Option<ExpandedCommand>, String> {
    let ext_dirs = get_ext_dirs(gcx).await;
    expand_with_dirs(&ext_dirs, raw_input).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_ext_dirs(config_dir: PathBuf) -> ExtDirs {
        ExtDirs { global_dirs: vec![config_dir], project_dirs: vec![] }
    }

    #[test]
    fn test_shell_split_basic() {
        assert_eq!(shell_split("a b c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_shell_split_double_quotes() {
        assert_eq!(shell_split(r#"a "b c" d"#), vec!["a", "b c", "d"]);
    }

    #[test]
    fn test_shell_split_single_quotes() {
        assert_eq!(shell_split("a 'b c' d"), vec!["a", "b c", "d"]);
    }

    #[test]
    fn test_shell_split_empty() {
        assert!(shell_split("").is_empty());
    }

    #[test]
    fn test_expand_template_arguments() {
        assert_eq!(expand_template("Do: $ARGUMENTS", "hello", &["hello".to_string()]), "Do: hello");
    }

    #[test]
    fn test_expand_template_positional() {
        let args = vec!["a".to_string(), "b c".to_string(), "d".to_string()];
        assert_eq!(expand_template("$1 and $2 and $3", "a \"b c\" d", &args), "a and b c and d");
    }

    #[test]
    fn test_expand_template_missing_positional() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(expand_template("$1 $2 $3", "a b", &args), "a b ");
    }

    #[test]
    fn test_expand_template_no_args() {
        assert_eq!(expand_template("Do: $ARGUMENTS", "", &[]), "Do: ");
    }

    #[test]
    fn test_expand_template_with_at_commands() {
        let args = vec!["fix".to_string(), "it".to_string()];
        assert_eq!(expand_template("@file path.rs $ARGUMENTS", "fix it", &args), "@file path.rs fix it");
    }

    #[test]
    fn test_expand_template_dollar_10_not_corrupted() {
        let args: Vec<String> = (1..=10).map(|i| format!("arg{}", i)).collect();
        let result = expand_template("$1 $10", "arg1 arg2 arg3 arg4 arg5 arg6 arg7 arg8 arg9 arg10", &args);
        assert_eq!(result, "arg1 arg10", "$10 must not be corrupted by $1 replacement");
    }

    #[tokio::test]
    async fn test_no_slash_returns_none() {
        let ext_dirs = make_ext_dirs(PathBuf::from("/nonexistent"));
        assert!(expand_with_dirs(&ext_dirs, "hello world").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_slash_space_returns_none() {
        let ext_dirs = make_ext_dirs(PathBuf::from("/nonexistent"));
        assert!(expand_with_dirs(&ext_dirs, "/ hello").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_not_at_start_returns_none() {
        let ext_dirs = make_ext_dirs(PathBuf::from("/nonexistent"));
        assert!(expand_with_dirs(&ext_dirs, "text /cmd arg").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_unknown_command_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        assert!(expand_with_dirs(&ext_dirs, "/nonexistent_cmd arg1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_basic_expansion() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(
            commands_dir.join("greet.md"),
            "---\ndescription: Greet\nallowed-tools:\n  - cat\n  - tree\nmodel: gpt-4o\n---\nHello $ARGUMENTS!",
        ).await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/greet world").await.unwrap().unwrap();
        assert_eq!(result.expanded_text, "Hello world!");
        assert_eq!(result.model_override, Some("gpt-4o".to_string()));
        assert_eq!(result.allowed_tools, vec!["cat", "tree"]);
        assert_eq!(result.source_command, "greet");
    }

    #[tokio::test]
    async fn test_whitespace_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("hi.md"), "Hi $ARGUMENTS").await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "  /hi there").await.unwrap().unwrap();
        assert_eq!(result.expanded_text, "Hi there");
    }

    #[tokio::test]
    async fn test_positional_args_with_quotes() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("show.md"), "$1 | $2 | $3").await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/show a \"b c\" d").await.unwrap().unwrap();
        assert_eq!(result.expanded_text, "a | b c | d");
    }

    #[tokio::test]
    async fn test_missing_positional_becomes_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("fmt.md"), "[$1][$2][$3]").await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/fmt x y").await.unwrap().unwrap();
        assert_eq!(result.expanded_text, "[x][y][]");
    }

    #[tokio::test]
    async fn test_no_args_arguments_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("cmd.md"), "Do: $ARGUMENTS").await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/cmd").await.unwrap().unwrap();
        assert_eq!(result.expanded_text, "Do: ");
    }

    #[tokio::test]
    async fn test_skills_slash_invocation_loads_full_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("my-skill");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: A useful skill\nallowed-tools:\n  - cat\nmodel: gpt-4o\nuser-invocable: true\n---\nDo something with $ARGUMENTS",
        ).await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/my-skill some args").await.unwrap().unwrap();
        assert_eq!(result.expanded_text, "Do something with some args");
        assert_eq!(result.model_override, Some("gpt-4o".to_string()));
        assert_eq!(result.allowed_tools, vec!["cat"]);
        assert_eq!(result.source_command, "my-skill");
        assert!(result.context_fork.is_none());
    }

    #[tokio::test]
    async fn test_skills_slash_invocation_non_user_invocable_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("hidden-skill");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: hidden-skill\ndescription: Hidden skill\nuser-invocable: false\n---\nBody",
        ).await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/hidden-skill args").await.unwrap();
        assert!(result.is_none(), "Non-user-invocable skill should not be invocable via /skill-name");
    }

    #[tokio::test]
    async fn test_skills_fork_creates_subchat() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("fork-skill");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: fork-skill\ndescription: A forking skill\ncontext: fork\nagent: my-agent\nuser-invocable: true\n---\nDo the fork task: $ARGUMENTS",
        ).await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/fork-skill some work").await.unwrap().unwrap();
        assert_eq!(result.context_fork, Some("my-agent".to_string()), "Fork skill should set context_fork to agent name");
        assert_eq!(result.source_command, "fork-skill");
        assert!(result.expanded_text.contains("some work"), "Expanded text should contain args");
    }

    #[tokio::test]
    async fn test_skills_fork_default_agent_name() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("default-fork");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: default-fork\ndescription: Fork skill with default agent\ncontext: fork\nuser-invocable: true\n---\nBody",
        ).await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/default-fork").await.unwrap().unwrap();
        assert_eq!(result.context_fork, Some("subagent".to_string()), "Default fork agent should be 'subagent'");
    }

    #[tokio::test]
    async fn test_skills_command_takes_precedence_over_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(
            commands_dir.join("same-name.md"),
            "---\ndescription: Command version\n---\nCommand body: $ARGUMENTS",
        ).await.unwrap();

        let skill_dir = tmp.path().join("skills").join("same-name");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: same-name\ndescription: Skill version\nuser-invocable: true\n---\nSkill body: $ARGUMENTS",
        ).await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/same-name arg").await.unwrap().unwrap();
        assert!(result.expanded_text.contains("Command body"), "Slash command should take precedence over skill");
        assert!(result.context_fork.is_none());
    }
}
