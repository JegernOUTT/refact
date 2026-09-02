//! Restricted verification command parsing and extraction for planner-provided verify commands.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use refact_tool_api::{extract_command_segments, first_matching_rule};

use crate::global_context::GlobalContext;
use crate::tasks::types::BoardCard;
use crate::tools::shell_gate::{ApprovalMode, ShellGatePolicy};

// COMMAND_HINTS is only an extraction heuristic for scraping bare lines out of markdown prose; it
// is not a security boundary and never gates execution.
const COMMAND_HINTS: &[&str] = &[
    "cargo", "npm", "npx", "pnpm", "yarn", "bun", "deno", "node", "pytest", "python", "python3",
    "tox", "poetry", "uv", "go", "make", "cmake", "ninja", "bazel", "just", "gradle", "gradlew",
    "mvn", "dotnet", "dart", "flutter", "swift", "ruby", "rake", "bundle", "mix", "zig",
    "composer", "php", "tsc", "jest", "vitest", "ctest",
];

// Words that never appear in a real invocation but are common in acceptance-criteria prose. Hints
// like `make` and `go` are also ordinary English verbs, so a bare line containing any of these is
// treated as prose rather than a command.
const PROSE_TOKENS: &[&str] = &[
    "a", "an", "and", "are", "be", "been", "but", "can", "if", "is", "it", "its", "must", "of",
    "or", "our", "should", "sure", "that", "the", "their", "then", "these", "they", "this",
    "those", "through", "was", "we", "were", "when", "will", "would", "you", "your",
];

#[derive(Clone, Debug)]
pub(crate) struct VerifyCommandPolicy {
    deny: Vec<String>,
    mode: ApprovalMode,
}

impl VerifyCommandPolicy {
    pub(crate) async fn load(gcx: Arc<GlobalContext>) -> Self {
        Self::from_shell_policy(&crate::tools::shell_gate::load_policy(gcx).await)
    }

    pub(crate) fn from_shell_policy(policy: &ShellGatePolicy) -> Self {
        Self {
            deny: policy.deny.clone(),
            mode: policy.mode.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn permissive() -> Self {
        Self {
            deny: Vec::new(),
            mode: ApprovalMode::Yolo,
        }
    }

    pub(crate) fn check(&self, command: &str) -> Result<(), String> {
        let segments = extract_command_segments(command);
        match first_matching_rule(command, &segments, &self.deny) {
            Some(rule) => {
                tracing::debug!(
                    mode = ?self.mode,
                    rule = %rule,
                    "verification command denied by shell policy"
                );
                Err(format!("denied by shell policy rule '{}'", rule))
            }
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedVerification {
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) argv: Vec<String>,
}

pub(crate) fn parse_verification_argv(command: &str) -> Result<ParsedVerification, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("empty command".to_string());
    }
    reject_global_metacharacters(command)?;
    let tokens =
        shell_words::split(command).map_err(|error| format!("malformed quoting: {}", error))?;
    if tokens.is_empty() {
        return Err("empty command".to_string());
    }

    let (cwd, rest) = if tokens.len() >= 4 && tokens[0] == "cd" && tokens[2] == "&&" {
        validate_argv_token(&tokens[1])?;
        validate_cd_dir(&tokens[1])?;
        (Some(PathBuf::from(&tokens[1])), &tokens[3..])
    } else {
        if tokens.iter().any(|token| token == "&&") {
            return Err("only a leading 'cd <dir> &&' prefix is allowed".to_string());
        }
        (None, tokens.as_slice())
    };
    if rest.is_empty() {
        return Err("missing command after cd prefix".to_string());
    }
    if rest.iter().any(|token| token.contains('&')) {
        return Err("ampersand is only allowed in a leading cd prefix".to_string());
    }

    let boundary = rest
        .iter()
        .position(|token| !is_assignment_token(token))
        .unwrap_or(rest.len());
    let (assignment_tokens, argv_tokens) = rest.split_at(boundary);
    if argv_tokens.is_empty() {
        return Err("missing command after environment assignments".to_string());
    }
    let mut env = Vec::with_capacity(assignment_tokens.len());
    for token in assignment_tokens {
        let (key, value) = token
            .split_once('=')
            .ok_or_else(|| "malformed environment assignment".to_string())?;
        validate_assignment_value(value)?;
        env.push((key.to_string(), expand_assignment_value(value)));
    }
    for token in argv_tokens {
        validate_argv_token(token)?;
    }
    Ok(ParsedVerification {
        cwd,
        env,
        argv: argv_tokens.to_vec(),
    })
}

fn is_assignment_token(token: &str) -> bool {
    let Some((key, _)) = token.split_once('=') else {
        return false;
    };
    is_env_name(key)
}

fn is_env_name(name: &str) -> bool {
    !name.is_empty() && env_name_len(name.as_bytes()) == name.len()
}

fn env_name_len(bytes: &[u8]) -> usize {
    if !bytes
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
    {
        return 0;
    }
    bytes
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
        .count()
}

pub(crate) fn verification_commands(card: &BoardCard) -> Vec<String> {
    let mut commands = Vec::new();
    for command in commands_from_instructions(&card.instructions) {
        push_expanded_unique(&mut commands, &command);
    }
    if let Some(report) = card.final_report_structured.as_ref() {
        for result in &report.verification {
            push_expanded_unique(&mut commands, &result.command);
        }
    }
    commands
}

fn commands_from_instructions(instructions: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut in_verification_section = false;
    let mut in_fence = false;
    for line in instructions.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim().to_ascii_lowercase();
            in_verification_section = heading.contains("acceptance") || heading.contains("verif");
            continue;
        }
        if let Some(command) = explicit_verify_command(trimmed) {
            push_unique(&mut commands, command);
            continue;
        }
        if in_fence || !in_verification_section {
            continue;
        }
        let candidate = strip_list_and_backticks(trimmed);
        if looks_like_command(&candidate) {
            push_unique(&mut commands, candidate);
        }
    }
    commands
}

fn explicit_verify_command(line: &str) -> Option<String> {
    let line = line.trim_start_matches(['-', '*', ' ']).trim();
    let lower = line.to_ascii_lowercase();
    let index = lower.find("verify:")?;
    let command = strip_list_and_backticks(&line[index + "verify:".len()..]);
    (!command.is_empty()).then_some(command)
}

fn strip_list_and_backticks(value: &str) -> String {
    let value = value.trim_start_matches(['-', '*', ' ']).trim();
    if value.starts_with('`') && value.ends_with('`') && value.len() >= 2 {
        value[1..value.len() - 1].trim().to_string()
    } else {
        strip_sentence_period(value).trim().to_string()
    }
}

fn strip_sentence_period(value: &str) -> &str {
    let mut chars = value.chars().rev();
    match (chars.next(), chars.next()) {
        (Some('.'), Some(previous)) if previous.is_alphanumeric() => {
            &value[..value.len() - '.'.len_utf8()]
        }
        _ => value,
    }
}

fn looks_like_command(command: &str) -> bool {
    let Ok(tokens) = shell_words::split(command) else {
        return false;
    };
    let rest = if tokens.first().is_some_and(|token| token == "cd")
        && tokens.get(2).is_some_and(|token| token == "&&")
    {
        tokens.get(3..).unwrap_or_default()
    } else {
        tokens.as_slice()
    };
    if rest
        .iter()
        .filter(|token| !is_assignment_token(token))
        .any(|token| PROSE_TOKENS.contains(&token.to_ascii_lowercase().as_str()))
    {
        return false;
    }
    let program = rest.iter().find(|token| !is_assignment_token(token));
    program.is_some_and(|program| COMMAND_HINTS.contains(&program.as_str()))
}

fn push_expanded_unique(commands: &mut Vec<String>, command: &str) {
    match split_safe_chain(command) {
        Ok(expanded) => {
            for command in expanded {
                push_unique(commands, command);
            }
        }
        Err(_) => push_unique(commands, command.trim().to_string()),
    }
}

fn split_safe_chain(command: &str) -> Result<Vec<String>, String> {
    reject_global_metacharacters(command)?;
    let tokens = shell_words::split(command).map_err(|error| error.to_string())?;
    let separators = tokens.iter().filter(|token| token.as_str() == "&&").count();
    if separators == 0 {
        return Ok(vec![command.trim().to_string()]);
    }
    let (cwd, chain) = if tokens.first().is_some_and(|token| token == "cd") {
        if tokens.len() < 4 || tokens.get(2).is_none_or(|token| token != "&&") {
            return Err("only a leading 'cd <dir> &&' prefix is allowed".to_string());
        }
        validate_cd_dir(&tokens[1])?;
        (Some(tokens[1].clone()), &tokens[3..])
    } else {
        (None, tokens.as_slice())
    };
    let mut result = Vec::new();
    for segment in chain.split(|token| token == "&&") {
        if segment.is_empty() {
            return Err("empty command in verification chain".to_string());
        }
        let rendered = shell_words::join(segment);
        let rendered = match cwd.as_ref() {
            Some(cwd) => format!("cd {} && {}", shell_words::quote(cwd), rendered),
            None => rendered,
        };
        parse_verification_argv(&rendered)?;
        result.push(rendered);
    }
    Ok(result)
}

fn push_unique(commands: &mut Vec<String>, command: String) {
    let command = command.trim();
    if !command.is_empty() && !commands.iter().any(|existing| existing == command) {
        commands.push(command.to_string());
    }
}

fn reject_global_metacharacters(command: &str) -> Result<(), String> {
    if command.contains('\n') || command.contains('\r') {
        return Err("newlines are not allowed".to_string());
    }
    if command.contains("$(") || command.contains('`') {
        return Err("command substitution is not allowed".to_string());
    }
    Ok(())
}

fn reject_shell_character(character: char) -> Result<(), String> {
    match character {
        '$' => Err("dollar expansion is not allowed".to_string()),
        '(' | ')' | '{' | '}' => Err("shell grouping is not allowed".to_string()),
        '*' | '?' | '[' | ']' => Err("globs are not allowed".to_string()),
        '~' => Err("home expansion is not allowed".to_string()),
        ';' => Err("command separators are not allowed".to_string()),
        '|' => Err("pipes are not allowed".to_string()),
        '<' | '>' => Err("redirects are not allowed".to_string()),
        _ => Ok(()),
    }
}

fn validate_argv_token(token: &str) -> Result<(), String> {
    for character in token.chars() {
        reject_shell_character(character)?;
    }
    Ok(())
}

// Assignment values allow a bare `$NAME` reference only; `${...}` and `$(...)` stay rejected so a
// value can never introduce shell parsing beyond a single variable lookup we expand ourselves.
fn validate_assignment_value(value: &str) -> Result<(), String> {
    let mut rest = value;
    while let Some(character) = rest.chars().next() {
        if character == '$' {
            let name_len = env_name_len(rest[1..].as_bytes());
            if name_len == 0 {
                return Err("dollar expansion is not allowed".to_string());
            }
            rest = &rest[1 + name_len..];
            continue;
        }
        reject_shell_character(character)?;
        rest = &rest[character.len_utf8()..];
    }
    Ok(())
}

fn expand_assignment_value(value: &str) -> String {
    let mut expanded = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(character) = rest.chars().next() {
        let name_len = if character == '$' {
            env_name_len(rest[1..].as_bytes())
        } else {
            0
        };
        if name_len > 0 {
            expanded.push_str(&std::env::var(&rest[1..1 + name_len]).unwrap_or_default());
            rest = &rest[1 + name_len..];
        } else {
            expanded.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    expanded
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
    use crate::tasks::types::{FinalReport, VerificationResult};

    #[test]
    fn parses_quoted_arguments_and_cwd() {
        let parsed =
            parse_verification_argv("cd \"dir with spaces\" && cargo test \"named test\"").unwrap();
        assert_eq!(parsed.cwd, Some(PathBuf::from("dir with spaces")));
        assert!(parsed.env.is_empty());
        assert_eq!(parsed.argv, vec!["cargo", "test", "named test"]);
        assert!(parse_verification_argv("cargo test '").is_err());
    }

    #[test]
    fn parses_env_assignment_prefix_with_cd_and_expansion() {
        let real_path = std::env::var("PATH").unwrap_or_default();
        let parsed = parse_verification_argv(
            "cd flexus_frontend && FLEXUS_PYTHON=/abs/py PATH=/abs/tools:$PATH pnpm typecheck",
        )
        .unwrap();
        assert_eq!(parsed.cwd, Some(PathBuf::from("flexus_frontend")));
        assert_eq!(
            parsed.env,
            vec![
                ("FLEXUS_PYTHON".to_string(), "/abs/py".to_string()),
                ("PATH".to_string(), format!("/abs/tools:{}", real_path)),
            ]
        );
        assert_eq!(parsed.argv, vec!["pnpm", "typecheck"]);
    }

    #[test]
    fn parses_simple_env_prefix() {
        let parsed = parse_verification_argv("FOO=bar cargo test").unwrap();
        assert_eq!(parsed.cwd, None);
        assert_eq!(parsed.env, vec![("FOO".to_string(), "bar".to_string())]);
        assert_eq!(parsed.argv, vec!["cargo", "test"]);
    }

    #[test]
    fn assignment_after_program_is_a_plain_argument() {
        let parsed = parse_verification_argv("cargo test FOO=bar").unwrap();
        assert!(parsed.env.is_empty());
        assert_eq!(parsed.argv, vec!["cargo", "test", "FOO=bar"]);
    }

    #[test]
    fn assignments_without_a_program_are_rejected() {
        let error = parse_verification_argv("FOO=bar").unwrap_err();
        assert_eq!(error, "missing command after environment assignments");
    }

    #[test]
    fn dollar_stays_rejected_in_argv_tokens_and_substitutions() {
        for command in [
            "cargo test $HOME",
            "cargo test `date`",
            "cargo test $(rm -rf /)",
            "FOO=$(evil) cargo test",
            "FOO=${HOME} cargo test",
            "FOO=$ cargo test",
        ] {
            assert!(parse_verification_argv(command).is_err(), "{command}");
        }
    }

    #[test]
    fn positional_and_malformed_dollar_refs_are_rejected() {
        for command in [
            "FOO=$1 cargo test",
            "FOO=$- cargo test",
            "FOO=a$ cargo test",
            "FOO=$/etc cargo test",
        ] {
            assert!(parse_verification_argv(command).is_err(), "{command}");
        }
    }

    #[test]
    fn common_targets_that_collide_with_english_are_still_commands() {
        for command in [
            "make all",
            "cargo test -- all",
            "make install",
            "go build ./...",
        ] {
            assert!(looks_like_command(command), "{command}");
        }
        assert!(looks_like_command("FOO=that pnpm test"));
        assert!(!looks_like_command("make sure the tests pass"));
        assert!(!looks_like_command("go through the migration list"));
    }

    #[test]
    fn env_prefixes_survive_chain_splitting() {
        let segments = split_safe_chain("cd ui && FOO=bar pnpm test && BAR=baz pnpm lint").unwrap();
        assert_eq!(segments.len(), 2);
        let expected = [
            ("FOO", "bar", vec!["pnpm", "test"]),
            ("BAR", "baz", vec!["pnpm", "lint"]),
        ];
        for (segment, (key, value, argv)) in segments.iter().zip(expected) {
            let parsed = parse_verification_argv(segment).unwrap();
            assert_eq!(parsed.cwd, Some(PathBuf::from("ui")), "{segment}");
            assert_eq!(
                parsed.env,
                vec![(key.to_string(), value.to_string())],
                "{segment}"
            );
            assert_eq!(parsed.argv, argv, "{segment}");
        }
    }

    #[test]
    fn unset_variable_expands_to_empty_string() {
        let parsed = parse_verification_argv("FOO=$DEFINITELY_UNSET_VAR_XYZ cargo test").unwrap();
        assert_eq!(parsed.env, vec![("FOO".to_string(), String::new())]);
        assert_eq!(parsed.argv, vec!["cargo", "test"]);
    }

    #[test]
    fn env_prefixed_command_is_recognised_by_extraction() {
        assert!(looks_like_command("FOO=bar pnpm test"));
        assert!(looks_like_command("cd ui && FOO=bar pnpm test"));
        assert!(!looks_like_command("FOO=bar notatool test"));
    }

    #[test]
    fn rejects_shell_features() {
        for command in [
            "cargo test $(rm -rf /)",
            "cargo test `date`",
            "cargo test | tee f",
            "cargo test > out",
            "cargo test; npm test",
            "cargo test *.rs",
            "cargo test && npm test",
        ] {
            assert!(parse_verification_argv(command).is_err(), "{command}");
        }
    }

    #[test]
    fn parses_previously_unsupported_binaries() {
        for command in [
            "dotnet test",
            "go test ./...",
            "make check",
            "gradlew build",
            "bash -c cargo",
        ] {
            let parsed = parse_verification_argv(command);
            assert!(parsed.is_ok(), "{command}: {:?}", parsed.err());
        }
        assert_eq!(
            parse_verification_argv("go test ./...").unwrap().argv,
            vec!["go", "test", "./..."]
        );
    }

    #[test]
    fn default_deny_rules_block_sudo_but_allow_normal_commands() {
        let policy = VerifyCommandPolicy::from_shell_policy(&ShellGatePolicy::default());
        let denied = policy.check("sudo rm -rf /").unwrap_err();
        assert!(denied.contains("denied by shell policy rule"), "{denied}");
        assert!(policy.check("dotnet test").is_ok());
        assert!(policy.check("go test ./...").is_ok());
        assert!(policy.check("rm -rf /").is_ok());
    }

    #[test]
    fn yolo_mode_allows_arbitrary_binaries_but_still_denies() {
        let policy = VerifyCommandPolicy::from_shell_policy(&ShellGatePolicy {
            mode: ApprovalMode::Yolo,
            ..ShellGatePolicy::default()
        });
        for command in ["mycompany-build --release", "gradlew build", "zig build"] {
            assert!(parse_verification_argv(command).is_ok(), "{command}");
            assert!(policy.check(command).is_ok(), "{command}");
        }
        assert!(policy.check("sudo id").is_err());
        assert!(VerifyCommandPolicy::permissive().check("sudo id").is_ok());
    }

    #[test]
    fn acceptance_prose_is_not_extracted_as_a_command() {
        let card: BoardCard = serde_json::from_value(serde_json::json!({
            "id":"T","title":"t","column":"done",
            "instructions":"## Acceptance Criteria\n- make sure the tests pass\n- go through the migration list\n- just verify that it works\n- mix of unit and integration coverage\n",
            "assignee":null,"agent_chat_id":null,"created_at":"now","started_at":null,"completed_at":null
        })).unwrap();
        assert!(
            verification_commands(&card).is_empty(),
            "prose must not be executed: {:?}",
            verification_commands(&card)
        );
    }

    #[test]
    fn real_commands_in_acceptance_section_are_still_extracted() {
        let card: BoardCard = serde_json::from_value(serde_json::json!({
            "id":"T","title":"t","column":"done",
            "instructions":"## Verification\n- make check\n- go test ./...\n- dotnet test\n- npm run lint\n",
            "assignee":null,"agent_chat_id":null,"created_at":"now","started_at":null,"completed_at":null
        })).unwrap();
        assert_eq!(
            verification_commands(&card),
            vec!["make check", "go test ./...", "dotnet test", "npm run lint"]
        );
    }

    #[test]
    fn trailing_sentence_period_is_stripped_but_path_globs_survive() {
        assert_eq!(strip_list_and_backticks("- cargo test."), "cargo test");
        assert_eq!(strip_list_and_backticks("- go test ./..."), "go test ./...");
        assert_eq!(strip_list_and_backticks("- pytest tests/"), "pytest tests/");
    }

    #[test]
    fn extracts_and_splits_safe_chains_with_cwd() {
        let mut card: BoardCard = serde_json::from_value(serde_json::json!({
            "id":"T","title":"t","column":"done","instructions":"## Verification\n- Verify: `cd refact-agent/engine && cargo check && cargo test --lib`",
            "assignee":null,"agent_chat_id":null,"created_at":"now","started_at":null,"completed_at":null
        })).unwrap();
        assert_eq!(
            verification_commands(&card),
            vec![
                "cd refact-agent/engine && cargo check",
                "cd refact-agent/engine && cargo test --lib",
            ]
        );
        card.final_report_structured = Some(FinalReport {
            verification: vec![VerificationResult {
                command: "npm test -- \"unit suite\"".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        assert_eq!(
            verification_commands(&card).last().unwrap(),
            "npm test -- \"unit suite\""
        );
    }

    #[test]
    fn splits_safe_chains_without_cwd_and_rejects_bad_segments() {
        assert_eq!(
            split_safe_chain("cargo check && npm test").unwrap(),
            vec!["cargo check", "npm test"]
        );
        assert!(split_safe_chain("cargo check && && npm test").is_err());
        assert!(split_safe_chain("cargo check && npm test > out").is_err());
        assert_eq!(
            split_safe_chain("dotnet build && dotnet test").unwrap(),
            vec!["dotnet build", "dotnet test"]
        );
        assert!(split_safe_chain("cd crates && cargo check && npm test")
            .unwrap()
            .iter()
            .all(|command| command.starts_with("cd crates && ")));
    }

    #[test]
    fn ignores_fenced_prose_and_arbitrary_checklists() {
        let card: BoardCard = serde_json::from_value(serde_json::json!({
            "id":"T","title":"t","column":"done","instructions":"## Notes\n- cargo test is mentioned here\n```text\ncargo test\nprose\n```\n## Acceptance Criteria\n- behavior works",
            "assignee":null,"agent_chat_id":null,"created_at":"now","started_at":null,"completed_at":null
        })).unwrap();
        assert!(verification_commands(&card).is_empty());
    }
}
