use glob::Pattern;
use regex::Regex;

use crate::command_classify::{executable_basename, segment_command, CommandSegments};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    Exec,
    Argv,
    Regex,
    Raw,
}

enum RuleMatcher {
    Glob(Pattern),
    Regex(Regex),
}

pub struct CommandRule {
    kind: RuleKind,
    matcher: RuleMatcher,
}

impl CommandRule {
    pub fn parse(rule: &str) -> Option<CommandRule> {
        let (kind, pattern) = if let Some(pattern) = rule.strip_prefix("exec:") {
            (RuleKind::Exec, pattern)
        } else if let Some(pattern) = rule.strip_prefix("argv:") {
            (RuleKind::Argv, pattern)
        } else if let Some(pattern) = rule.strip_prefix("re:") {
            (RuleKind::Regex, pattern)
        } else if let Some(pattern) = rule.strip_prefix("raw:") {
            (RuleKind::Raw, pattern)
        } else if rule.contains(|ch: char| ch.is_ascii_whitespace()) {
            (RuleKind::Argv, rule)
        } else {
            (RuleKind::Exec, rule)
        };

        let matcher = match kind {
            RuleKind::Regex => match Regex::new(pattern) {
                Ok(regex) => RuleMatcher::Regex(regex),
                Err(error) => {
                    tracing::warn!("Invalid regex pattern '{}': {}", rule, error);
                    return None;
                }
            },
            _ => match Pattern::new(pattern) {
                Ok(pattern) => RuleMatcher::Glob(pattern),
                Err(error) => {
                    tracing::warn!("Invalid glob pattern '{}': {}", rule, error);
                    return None;
                }
            },
        };

        Some(CommandRule { kind, matcher })
    }

    pub fn matches(&self, command: &str, segments: &CommandSegments) -> bool {
        match (&self.kind, &self.matcher) {
            (RuleKind::Exec, RuleMatcher::Glob(pattern)) => {
                segments.segments.iter().any(|segment| {
                    executable_basename(segment).is_some_and(|name| pattern.matches(name))
                })
            }
            (RuleKind::Argv, RuleMatcher::Glob(pattern)) => segments
                .segments
                .iter()
                .any(|segment| pattern.matches(&segment_command(segment))),
            (RuleKind::Regex, RuleMatcher::Regex(regex)) => segments
                .segments
                .iter()
                .any(|segment| regex.is_match(&segment_command(segment))),
            (RuleKind::Raw, RuleMatcher::Glob(pattern)) => pattern.matches(command),
            _ => false,
        }
    }

    pub fn kind(&self) -> RuleKind {
        self.kind
    }
}

pub fn first_matching_rule(
    command: &str,
    segments: &CommandSegments,
    rules: &[String],
) -> Option<String> {
    rules.iter().find_map(|source| {
        let rule = CommandRule::parse(source)?;
        rule.matches(command, segments).then(|| source.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_classify::extract_command_segments;

    fn matches(command: &str, rule: &str) -> bool {
        let segments = extract_command_segments(command);
        CommandRule::parse(rule).is_some_and(|rule| rule.matches(command, &segments))
    }

    #[test]
    fn exec_rules_do_not_match_text_inside_arguments() {
        let command = "git status --porcelain | head -30; echo \"=== are the flagged files modified by me? ===\"; git status --porcelain | grep -E \"ChatForm|DialogImage|Dropzone|Markdown.internalLinks|TabBar|workspaceSlice\"";
        assert!(!matches(command, "*rm*"));
        assert!(matches("rm -rf /", "*rm*"));
        assert!(matches("bash -c 'sudo rm -rf /'", "*rm*"));
        assert!(matches(command, "raw:*rm*"));
    }

    #[test]
    fn argv_rules_match_whole_segment_commands() {
        assert!(matches("git push --force", "git push*"));
        assert!(!matches("git status", "git push*"));
    }

    #[test]
    fn explicit_prefixes_select_the_documented_target() {
        assert!(matches("/usr/bin/rm -f x", "exec:rm"));
        assert!(matches("git push --force", "argv:git push*"));
        assert!(matches("git push --force", r"re:^git\s+push\b"));
        assert!(matches("echo harmless-rm-text", "raw:*rm*"));
        assert!(!matches("echo harmless-rm-text", "exec:*rm*"));
    }

    #[test]
    fn invalid_patterns_never_match() {
        assert!(CommandRule::parse("exec:[invalid").is_none());
        assert!(CommandRule::parse("re:(invalid").is_none());
        let command = "rm -rf /";
        let segments = extract_command_segments(command);
        let rules = vec!["exec:[invalid".to_string(), "re:(invalid".to_string()];
        assert_eq!(first_matching_rule(command, &segments, &rules), None);
    }
}
