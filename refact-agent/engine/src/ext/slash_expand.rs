use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::app_state::AppState;
use crate::ext::config_dirs::{get_ext_dirs, ExtDirs};
use crate::ext::skills::{load_skill_full, load_skill_indices, SkillIndex};
use crate::ext::slash_commands::{load_slash_commands, SlashCommand};
use crate::integrations::mcp::mcp_prompts::{MCP_PROMPT_PREFIX, execute_mcp_prompt};

const SLASH_CACHE_TTL: Duration = Duration::from_secs(5);

struct SlashCacheEntry {
    commands: Arc<Vec<SlashCommand>>,
    skill_indices: Arc<Vec<SkillIndex>>,
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
    app: AppState,
    raw_input: &str,
) -> Result<Option<ExpandedCommand>, String> {
    if !raw_input.contains('/') {
        return Ok(None);
    }
    let ext_dirs = get_ext_dirs(app.clone()).await;
    let generation = app
        .integrations
        .ext_cache_generation
        .load(Ordering::Relaxed);
    let lock = SLASH_CACHE.get_or_init(|| tokio::sync::RwLock::new(None));
    let (commands, skill_indices) = {
        let read = lock.read().await;
        let cached = read.as_ref().and_then(|entry| {
            if entry.generation == generation && entry.loaded_at.elapsed() < SLASH_CACHE_TTL {
                Some((
                    Arc::clone(&entry.commands),
                    Arc::clone(&entry.skill_indices),
                ))
            } else {
                None
            }
        });
        drop(read);
        if let Some(data) = cached {
            data
        } else {
            let commands = Arc::new(load_slash_commands(&ext_dirs).await);
            let skill_indices = Arc::new(load_skill_indices(&ext_dirs).await);
            let mut write = lock.write().await;
            *write = Some(SlashCacheEntry {
                commands: Arc::clone(&commands),
                skill_indices: Arc::clone(&skill_indices),
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
    expand_mcp_prompt_command(app, raw_input).await
}

async fn expand_mcp_prompt_command(
    app: AppState,
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
        match execute_mcp_prompt(app.gcx.clone(), cmd_name, &args_str, 30).await {
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
}
