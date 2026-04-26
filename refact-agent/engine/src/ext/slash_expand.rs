use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::RwLock as ARwLock;

use crate::ext::config_dirs::{get_ext_dirs, ExtDirs};
use crate::ext::skills::{load_skill_full, load_skill_indices, SkillIndex};
use crate::ext::slash_commands::{load_slash_commands, SlashCommand};
use crate::global_context::GlobalContext;
use crate::integrations::mcp::mcp_prompts::{MCP_PROMPT_PREFIX, execute_mcp_prompt};

const SLASH_CACHE_TTL: Duration = Duration::from_secs(5);

struct SlashCacheEntry {
    commands: Vec<SlashCommand>,
    skill_indices: Vec<SkillIndex>,
    loaded_at: Instant,
    generation: u64,
}

static SLASH_CACHE: OnceLock<tokio::sync::RwLock<Option<SlashCacheEntry>>> = OnceLock::new();

static PLACEHOLDER_RE: OnceLock<regex::Regex> = OnceLock::new();

fn placeholder_regex() -> &'static regex::Regex {
    PLACEHOLDER_RE.get_or_init(|| regex::Regex::new(r"\$(\d+)").unwrap())
}

pub struct SkillActivationInfo {
    pub name: String,
    pub body: String,
    pub skill_dir: PathBuf,
}

pub struct ExpandedCommand {
    pub expanded_text: String,
    pub model_override: Option<String>,
    pub allowed_tools: Vec<String>,
    pub source_command: String,
    pub context_fork: Option<String>,
    pub skill_to_activate: Option<SkillActivationInfo>,
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
    let result = body.replace("$ARGUMENTS", args_str);
    let re = placeholder_regex();
    re.replace_all(&result, |caps: &regex::Captures| {
        let n: usize = caps[1].parse().unwrap_or(0);
        if n >= 1 && n <= positional.len() {
            positional[n - 1].clone()
        } else {
            String::new()
        }
    })
    .into_owned()
}

async fn expand_with_data(
    commands: &[SlashCommand],
    skill_indices: &[SkillIndex],
    ext_dirs: &ExtDirs,
    raw_input: &str,
) -> Result<Option<ExpandedCommand>, String> {
    let commands_map: std::collections::HashMap<&str, &SlashCommand> =
        commands.iter().map(|c| (c.name.as_str(), c)).collect();
    let skill_names: std::collections::HashSet<&str> = skill_indices
        .iter()
        .filter(|s| s.user_invocable)
        .map(|s| s.name.as_str())
        .collect();
    let char_bytes: Vec<(usize, char)> = raw_input.char_indices().collect();
    for (char_idx, &(byte_pos, ch)) in char_bytes.iter().enumerate() {
        if ch != '/' {
            continue;
        }
        if char_idx > 0 && !char_bytes[char_idx - 1].1.is_whitespace() {
            continue;
        }
        let name_byte_start = byte_pos + 1;
        let name_byte_end = char_bytes[char_idx + 1..]
            .iter()
            .find(|(_, c)| c.is_whitespace())
            .map(|(b, _)| *b)
            .unwrap_or(raw_input.len());
        let cmd_name = &raw_input[name_byte_start..name_byte_end];
        if cmd_name.is_empty() {
            continue;
        }
        let args_str = raw_input[name_byte_end..].trim().to_string();
        let positional = shell_split(&args_str);
        let prefix = &raw_input[..byte_pos];
        if let Some(command) = commands_map.get(cmd_name) {
            return Ok(Some(ExpandedCommand {
                expanded_text: format!(
                    "{}{}",
                    prefix,
                    expand_template(&command.body, &args_str, &positional)
                ),
                model_override: command.model.clone(),
                allowed_tools: command.allowed_tools.clone(),
                source_command: cmd_name.to_string(),
                context_fork: None,
                skill_to_activate: None,
            }));
        }
        if skill_names.contains(cmd_name) {
            if let Some(skill) = load_skill_full(ext_dirs, cmd_name).await {
                if skill.index.user_invocable {
                    let agent_name = skill
                        .agent
                        .clone()
                        .unwrap_or_else(|| "subagent".to_string());
                    let context_fork = if skill.context.as_deref() == Some("fork") {
                        Some(agent_name)
                    } else {
                        None
                    };
                    let expanded_text = if args_str.is_empty() {
                        format!(
                            "{}Follow the instructions from the {} skill.",
                            prefix, cmd_name
                        )
                    } else {
                        format!("{}{}", prefix, args_str)
                    };
                    return Ok(Some(ExpandedCommand {
                        expanded_text,
                        model_override: skill.model.clone(),
                        allowed_tools: skill.allowed_tools.clone(),
                        source_command: cmd_name.to_string(),
                        context_fork,
                        skill_to_activate: Some(SkillActivationInfo {
                            name: cmd_name.to_string(),
                            body: skill.body.clone(),
                            skill_dir: skill.skill_dir.clone(),
                        }),
                    }));
                }
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
async fn expand_with_dirs(
    ext_dirs: &ExtDirs,
    raw_input: &str,
) -> Result<Option<ExpandedCommand>, String> {
    if !raw_input.contains('/') {
        return Ok(None);
    }
    let commands = load_slash_commands(ext_dirs).await;
    let skill_indices = load_skill_indices(ext_dirs).await;
    expand_with_data(&commands, &skill_indices, ext_dirs, raw_input).await
}

pub async fn expand_slash_command(
    gcx: Arc<ARwLock<GlobalContext>>,
    raw_input: &str,
) -> Result<Option<ExpandedCommand>, String> {
    if !raw_input.contains('/') {
        return Ok(None);
    }
    let ext_dirs = get_ext_dirs(gcx.clone()).await;
    let generation = {
        let gcx_locked = gcx.read().await;
        gcx_locked.ext_cache_generation.load(Ordering::Relaxed)
    };
    let lock = SLASH_CACHE.get_or_init(|| tokio::sync::RwLock::new(None));
    let (commands, skill_indices) = {
        let read = lock.read().await;
        let cached = read.as_ref().and_then(|entry| {
            if entry.generation == generation && entry.loaded_at.elapsed() < SLASH_CACHE_TTL {
                Some((entry.commands.clone(), entry.skill_indices.clone()))
            } else {
                None
            }
        });
        drop(read);
        if let Some(data) = cached {
            data
        } else {
            let commands = load_slash_commands(&ext_dirs).await;
            let skill_indices = load_skill_indices(&ext_dirs).await;
            let mut write = lock.write().await;
            *write = Some(SlashCacheEntry {
                commands: commands.clone(),
                skill_indices: skill_indices.clone(),
                loaded_at: Instant::now(),
                generation,
            });
            (commands, skill_indices)
        }
    };
    if let Some(expanded) =
        expand_with_data(&commands, &skill_indices, &ext_dirs, raw_input).await?
    {
        return Ok(Some(expanded));
    }
    expand_mcp_prompt_command(gcx, raw_input).await
}

async fn expand_mcp_prompt_command(
    gcx: Arc<ARwLock<GlobalContext>>,
    raw_input: &str,
) -> Result<Option<ExpandedCommand>, String> {
    let char_bytes: Vec<(usize, char)> = raw_input.char_indices().collect();
    for (char_idx, &(byte_pos, ch)) in char_bytes.iter().enumerate() {
        if ch != '/' {
            continue;
        }
        if char_idx > 0 && !char_bytes[char_idx - 1].1.is_whitespace() {
            continue;
        }
        let name_byte_start = byte_pos + 1;
        let name_byte_end = char_bytes[char_idx + 1..]
            .iter()
            .find(|(_, c)| c.is_whitespace())
            .map(|(b, _)| *b)
            .unwrap_or(raw_input.len());
        let cmd_name = &raw_input[name_byte_start..name_byte_end];
        if !cmd_name.starts_with(MCP_PROMPT_PREFIX) {
            continue;
        }
        let args_str = raw_input[name_byte_end..].trim().to_string();
        let prefix = &raw_input[..byte_pos];
        match execute_mcp_prompt(gcx.clone(), cmd_name, &args_str, 30).await {
            Ok(expanded_body) => {
                return Ok(Some(ExpandedCommand {
                    expanded_text: format!("{}{}", prefix, expanded_body),
                    model_override: None,
                    allowed_tools: vec![],
                    source_command: cmd_name.to_string(),
                    context_fork: None,
                    skill_to_activate: None,
                }));
            }
            Err(e) => {
                tracing::warn!("MCP prompt expansion failed for {}: {}", cmd_name, e);
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_ext_dirs(config_dir: PathBuf) -> ExtDirs {
        ExtDirs {
            global_dirs: vec![config_dir],
            installed_dirs: vec![],
            project_dirs: vec![],
        }
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
        assert_eq!(
            expand_template("Do: $ARGUMENTS", "hello", &["hello".to_string()]),
            "Do: hello"
        );
    }

    #[test]
    fn test_expand_template_positional() {
        let args = vec!["a".to_string(), "b c".to_string(), "d".to_string()];
        assert_eq!(
            expand_template("$1 and $2 and $3", "a \"b c\" d", &args),
            "a and b c and d"
        );
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
        assert_eq!(
            expand_template("@file path.rs $ARGUMENTS", "fix it", &args),
            "@file path.rs fix it"
        );
    }

    #[test]
    fn test_expand_template_dollar_10_not_corrupted() {
        let args: Vec<String> = (1..=10).map(|i| format!("arg{}", i)).collect();
        let result = expand_template(
            "$1 $10",
            "arg1 arg2 arg3 arg4 arg5 arg6 arg7 arg8 arg9 arg10",
            &args,
        );
        assert_eq!(
            result, "arg1 arg10",
            "$10 must not be corrupted by $1 replacement"
        );
    }

    #[tokio::test]
    async fn test_no_slash_returns_none() {
        let ext_dirs = make_ext_dirs(PathBuf::from("/nonexistent"));
        assert!(expand_with_dirs(&ext_dirs, "hello world")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_slash_space_returns_none() {
        let ext_dirs = make_ext_dirs(PathBuf::from("/nonexistent"));
        assert!(expand_with_dirs(&ext_dirs, "/ hello")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_mid_message_expansion() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("cmd.md"), "Expanded: $ARGUMENTS")
            .await
            .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "text /cmd arg")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.expanded_text, "text Expanded: arg");
        assert_eq!(result.source_command, "cmd");
    }

    #[tokio::test]
    async fn test_unknown_command_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        assert!(expand_with_dirs(&ext_dirs, "/nonexistent_cmd arg1")
            .await
            .unwrap()
            .is_none());
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
        let result = expand_with_dirs(&ext_dirs, "/greet world")
            .await
            .unwrap()
            .unwrap();
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
        tokio::fs::write(commands_dir.join("hi.md"), "Hi $ARGUMENTS")
            .await
            .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "  /hi there")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.expanded_text, "  Hi there");
    }

    #[tokio::test]
    async fn test_positional_args_with_quotes() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("show.md"), "$1 | $2 | $3")
            .await
            .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/show a \"b c\" d")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.expanded_text, "a | b c | d");
    }

    #[tokio::test]
    async fn test_missing_positional_becomes_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("fmt.md"), "[$1][$2][$3]")
            .await
            .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/fmt x y")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.expanded_text, "[x][y][]");
    }

    #[tokio::test]
    async fn test_no_args_arguments_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("cmd.md"), "Do: $ARGUMENTS")
            .await
            .unwrap();

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
        let result = expand_with_dirs(&ext_dirs, "/my-skill some args")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.expanded_text, "some args");
        assert_eq!(result.model_override, Some("gpt-4o".to_string()));
        assert_eq!(result.allowed_tools, vec!["cat"]);
        assert_eq!(result.source_command, "my-skill");
        assert!(result.context_fork.is_none());
        let info = result
            .skill_to_activate
            .expect("skill_to_activate must be Some for skill invocation");
        assert_eq!(info.name, "my-skill");
        assert!(info.body.contains("Do something with $ARGUMENTS"));
    }

    #[tokio::test]
    async fn test_skill_no_args_gets_default_message() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("my-skill");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: A useful skill\nuser-invocable: true\n---\nBody",
        )
        .await
        .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/my-skill")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            result.expanded_text,
            "Follow the instructions from the my-skill skill."
        );
        assert!(result.skill_to_activate.is_some());
    }

    #[tokio::test]
    async fn test_skill_with_args_uses_args() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("tester");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: tester\ndescription: Test skill\nuser-invocable: true\n---\nBody $ARGUMENTS",
        ).await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/tester fix the bug")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.expanded_text, "fix the bug");
        let info = result.skill_to_activate.unwrap();
        assert_eq!(info.name, "tester");
    }

    #[tokio::test]
    async fn test_skill_mid_message_uses_prefix_plus_args() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("helper");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: helper\ndescription: Helper skill\nuser-invocable: true\n---\nBody",
        )
        .await
        .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "please /helper do work")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.expanded_text, "please do work");
        assert!(result.skill_to_activate.is_some());
    }

    #[tokio::test]
    async fn test_skills_slash_invocation_non_user_invocable_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("hidden-skill");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: hidden-skill\ndescription: Hidden skill\nuser-invocable: false\n---\nBody",
        )
        .await
        .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/hidden-skill args")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "Non-user-invocable skill should not be invocable via /skill-name"
        );
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
        let result = expand_with_dirs(&ext_dirs, "/fork-skill some work")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            result.context_fork,
            Some("my-agent".to_string()),
            "Fork skill should set context_fork to agent name"
        );
        assert_eq!(result.source_command, "fork-skill");
        assert!(
            result.expanded_text.contains("some work"),
            "Expanded text should contain args"
        );
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
        let result = expand_with_dirs(&ext_dirs, "/default-fork")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            result.context_fork,
            Some("subagent".to_string()),
            "Default fork agent should be 'subagent'"
        );
    }

    #[tokio::test]
    async fn test_skills_command_takes_precedence_over_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(
            commands_dir.join("same-name.md"),
            "---\ndescription: Command version\n---\nCommand body: $ARGUMENTS",
        )
        .await
        .unwrap();

        let skill_dir = tmp.path().join("skills").join("same-name");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: same-name\ndescription: Skill version\nuser-invocable: true\n---\nSkill body: $ARGUMENTS",
        ).await.unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "/same-name arg")
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.expanded_text.contains("Command body"),
            "Slash command should take precedence over skill"
        );
        assert!(result.context_fork.is_none());
    }

    #[tokio::test]
    async fn test_mid_message_preserves_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("greet.md"), "Hello $ARGUMENTS!")
            .await
            .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "Please /greet world")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.expanded_text, "Please Hello world!");
    }

    #[tokio::test]
    async fn test_unknown_slash_skipped_known_found() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join("commands");
        tokio::fs::create_dir_all(&commands_dir).await.unwrap();
        tokio::fs::write(commands_dir.join("greet.md"), "Hello $ARGUMENTS!")
            .await
            .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path().to_path_buf());
        let result = expand_with_dirs(&ext_dirs, "see /usr/bin then /greet world")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.expanded_text, "see /usr/bin then Hello world!");
    }

    #[tokio::test]
    async fn test_url_not_expanded() {
        let ext_dirs = make_ext_dirs(PathBuf::from("/nonexistent"));
        assert!(expand_with_dirs(&ext_dirs, "visit http://example.com/path")
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_expand_template_high_placeholder_removed() {
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(expand_template("$1 $99 $2", "a b", &args), "a  b");
    }

    #[tokio::test]
    async fn test_no_slash_skips_loading() {
        let ext_dirs = make_ext_dirs(PathBuf::from("/nonexistent_dir_wont_be_read"));
        let result = expand_with_dirs(&ext_dirs, "plain text without any slashes here").await;
        assert!(
            result.unwrap().is_none(),
            "Input with no '/' should return None without loading"
        );
    }
}
