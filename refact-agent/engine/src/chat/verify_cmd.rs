//! Restricted verification command parsing for planner-provided verify commands.
//!
//! This parser is not a security boundary. It rejects common shell metacharacters and only allows
//! a small binary allowlist, but allowed binaries can still run arbitrary scripts. Path arguments
//! are not validated beyond the optional leading `cd <relative-dir> &&` prefix, and symlink escapes
//! are not prevented here.

use std::path::{Component, Path, PathBuf};

const ALLOWED_BINARIES: &[&str] = &["cargo", "npm", "npx", "pytest", "bun", "yarn"];

pub(crate) fn parse_restricted_argv(
    command: &str,
) -> Result<(Option<PathBuf>, Vec<String>), String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("empty command".to_string());
    }
    reject_metacharacters(command)?;

    let tokens = command.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return Err("empty command".to_string());
    }

    let (cwd, argv_tokens) = if tokens.len() >= 4 && tokens[0] == "cd" && tokens[2] == "&&" {
        validate_cd_dir(tokens[1])?;
        (Some(PathBuf::from(tokens[1])), &tokens[3..])
    } else {
        if tokens.iter().any(|token| *token == "&&") {
            return Err("only a leading 'cd <dir> &&' prefix is allowed".to_string());
        }
        (None, tokens.as_slice())
    };

    if argv_tokens.is_empty() {
        return Err("missing command after cd prefix".to_string());
    }
    if argv_tokens.iter().any(|token| token.contains('&')) {
        return Err("ampersand is only allowed in a leading cd prefix".to_string());
    }

    let program = argv_tokens[0];
    if !ALLOWED_BINARIES.iter().any(|allowed| program == *allowed) {
        return Err(format!("unsupported command binary '{}'", program));
    }

    Ok((
        cwd,
        argv_tokens
            .iter()
            .map(|token| (*token).to_string())
            .collect(),
    ))
}

fn reject_metacharacters(command: &str) -> Result<(), String> {
    if command.contains('\n') || command.contains('\r') {
        return Err("newlines are not allowed".to_string());
    }
    if command.contains("$(") || command.contains("${") {
        return Err("command substitution is not allowed".to_string());
    }
    if command.contains('`') {
        return Err("backticks are not allowed".to_string());
    }
    for character in command.chars() {
        match character {
            '$' => return Err("dollar expansion is not allowed".to_string()),
            '(' | ')' | '{' | '}' => return Err("shell grouping is not allowed".to_string()),
            '*' => return Err("globs are not allowed".to_string()),
            '~' => return Err("home expansion is not allowed".to_string()),
            ';' => return Err("command separators are not allowed".to_string()),
            '|' => return Err("pipes are not allowed".to_string()),
            '<' | '>' => return Err("redirects are not allowed".to_string()),
            _ => {}
        }
    }
    for token in command.split_whitespace() {
        if token.contains('&') && token != "&&" {
            return Err("ampersand is only allowed as '&&' in a cd prefix".to_string());
        }
    }
    Ok(())
}

fn validate_cd_dir(dir: &str) -> Result<(), String> {
    if dir.is_empty() {
        return Err("cd directory is empty".to_string());
    }
    let path = Path::new(dir);
    if path.is_absolute() {
        return Err("cd directory must be relative".to_string());
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err("cd directory must stay within the worktree".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_restricted_argv_rejects_command_substitution() {
        assert!(parse_restricted_argv("cargo test $(rm -rf /)").is_err());
    }

    #[test]
    fn parse_restricted_argv_rejects_backticks() {
        assert!(parse_restricted_argv("cargo test `curl http://example.com`").is_err());
    }

    #[test]
    fn parse_restricted_argv_rejects_pipes_and_redirects() {
        assert!(parse_restricted_argv("cargo test | tee f").is_err());
        assert!(parse_restricted_argv("cargo test > out").is_err());
    }

    #[test]
    fn parse_restricted_argv_accepts_simple_cargo() {
        let parsed = parse_restricted_argv("cargo test --lib foo").unwrap();
        assert_eq!(parsed.0, None);
        assert_eq!(parsed.1, vec!["cargo", "test", "--lib", "foo"]);
    }

    #[test]
    fn parse_restricted_argv_accepts_cd_prefix() {
        let parsed = parse_restricted_argv("cd refact-agent/engine && cargo check").unwrap();
        assert_eq!(parsed.0, Some(PathBuf::from("refact-agent/engine")));
        assert_eq!(parsed.1, vec!["cargo", "check"]);
    }

    #[test]
    fn parse_restricted_argv_rejects_unknown_binary() {
        assert!(parse_restricted_argv("bash -c cargo test").is_err());
    }
}
