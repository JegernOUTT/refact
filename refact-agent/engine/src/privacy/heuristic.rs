//! Best-effort shell path attribution for degraded observation mode.
//!
//! This parser reports hints after syscall observation is unavailable. It is not an enforcement
//! mechanism and marks uncertainty instead of treating parsed paths as a complete access record.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use refact_privacy::{Attribution, CompiledPolicy, FileRecord};
use tree_sitter::{Node, Parser};

/// Paths attributed from a shell command and whether the result may omit accesses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeuristicAttribution {
    pub files: Vec<FileRecord>,
    pub incomplete: bool,
}

/// Attributes literal existing paths from a shell command without enforcing access.
pub fn attribute_shell_command(
    command: &str,
    cwd: &Path,
    policy: &CompiledPolicy,
) -> HeuristicAttribution {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .is_err()
    {
        return HeuristicAttribution {
            incomplete: true,
            ..HeuristicAttribution::default()
        };
    }
    let Some(tree) = parser.parse(command, None) else {
        return HeuristicAttribution {
            incomplete: true,
            ..HeuristicAttribution::default()
        };
    };

    let mut collector = Collector {
        source: command.as_bytes(),
        cwd,
        policy,
        roots: vec![cwd.to_path_buf()],
        seen: HashSet::new(),
        result: HeuristicAttribution {
            incomplete: tree.root_node().has_error(),
            ..HeuristicAttribution::default()
        },
    };
    collector.walk(tree.root_node(), None);
    collector.result
}

struct Collector<'a> {
    source: &'a [u8],
    cwd: &'a Path,
    policy: &'a CompiledPolicy,
    roots: Vec<PathBuf>,
    seen: HashSet<PathBuf>,
    result: HeuristicAttribution,
}

impl Collector<'_> {
    fn walk(&mut self, node: Node<'_>, command_name: Option<(usize, usize)>) {
        if is_uncertain_syntax(node) {
            self.result.incomplete = true;
        }

        let command_name = if node.kind() == "command" {
            let name_node = find_command_name(node);
            if let Some(name_node) = name_node {
                let name = node_text(name_node, self.source);
                if !is_known_literal_path_reader(name) {
                    self.result.incomplete = true;
                }
                if is_interpreter(name) && has_ancestor(node, "pipeline") {
                    self.result.incomplete = true;
                }
                Some((name_node.start_byte(), name_node.end_byte()))
            } else {
                self.result.incomplete = true;
                None
            }
        } else {
            command_name
        };

        let is_command_name = command_name
            .map(|range| range == (node.start_byte(), node.end_byte()))
            .unwrap_or(false);
        if !is_command_name && is_literal_word(node) {
            self.collect_path(node);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, command_name);
        }
    }

    fn collect_path(&mut self, node: Node<'_>) {
        if has_uncertain_descendant(node) {
            self.result.incomplete = true;
            return;
        }
        let Some(literal) = shell_literal(node_text(node, self.source)) else {
            return;
        };
        if literal.starts_with('-') {
            return;
        }

        let path = Path::new(&literal);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        if !resolved.exists() || !self.seen.insert(resolved.clone()) {
            return;
        }

        self.result.files.push(FileRecord {
            zone: self
                .policy
                .strictest_zone_for_paths_with_roots([path, resolved.as_path()], &self.roots)
                .name
                .clone(),
            path: literal,
            attribution: Attribution::Heuristic,
        });
    }
}

fn find_command_name(command: Node<'_>) -> Option<Node<'_>> {
    command
        .child_by_field_name("name")
        .or_else(|| command.child_by_field_name("command_name"))
        .or_else(|| {
            let mut cursor = command.walk();
            let name = command
                .children(&mut cursor)
                .find(|child| matches!(child.kind(), "command_name" | "word" | "identifier"));
            name
        })
}

fn node_text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

fn is_literal_word(node: Node<'_>) -> bool {
    matches!(node.kind(), "word" | "raw_string" | "string")
}

fn is_uncertain_syntax(node: Node<'_>) -> bool {
    node.is_error()
        || node.is_missing()
        || node.kind() == "ERROR"
        || node.kind().contains("expansion")
        || node.kind().contains("substitution")
}

fn has_uncertain_descendant(node: Node<'_>) -> bool {
    if is_uncertain_syntax(node) {
        return true;
    }
    let mut cursor = node.walk();
    let uncertain = node.children(&mut cursor).any(has_uncertain_descendant);
    uncertain
}

fn has_ancestor(mut node: Node<'_>, kind: &str) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == kind {
            return true;
        }
        node = parent;
    }
    false
}

fn shell_literal(text: &str) -> Option<String> {
    let text = text.trim();
    let unquoted = if text.len() >= 2
        && ((text.starts_with('\'') && text.ends_with('\''))
            || (text.starts_with('"') && text.ends_with('"')))
    {
        &text[1..text.len() - 1]
    } else {
        text
    };
    if unquoted.is_empty()
        || unquoted.chars().any(|character| {
            matches!(
                character,
                '$' | '`' | '*' | '?' | '[' | ']' | '{' | '}' | '~'
            )
        })
    {
        return None;
    }
    Some(unquoted.to_string())
}

fn is_known_literal_path_reader(command: &str) -> bool {
    matches!(
        command.rsplit('/').next().unwrap_or(command),
        "cat"
            | "grep"
            | "rg"
            | "head"
            | "tail"
            | "wc"
            | "sort"
            | "uniq"
            | "cut"
            | "sed"
            | "awk"
            | "find"
            | "ls"
            | "stat"
            | "file"
            | "du"
    )
}

fn is_interpreter(command: &str) -> bool {
    matches!(
        command.rsplit('/').next().unwrap_or(command),
        "sh" | "bash"
            | "dash"
            | "zsh"
            | "ksh"
            | "fish"
            | "python"
            | "python3"
            | "node"
            | "ruby"
            | "perl"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_privacy::{PrivacyPolicy, ShellBehavior, SubagentPolicy, Zone};

    fn policy() -> CompiledPolicy {
        PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                Zone {
                    name: "secrets".to_string(),
                    patterns: vec![".env".to_string()],
                    send_to: Vec::new(),
                    on_shell_read: ShellBehavior::Withhold,
                },
                Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: ShellBehavior::Withhold,
                },
            ],
            subagents: SubagentPolicy::default(),
        }
        .compile()
        .expect("policy should compile")
    }

    fn paths(result: &HeuristicAttribution) -> Vec<&str> {
        result.files.iter().map(|file| file.path.as_str()).collect()
    }

    #[test]
    fn attributes_literal_file() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(temp.path().join(".env"), "secret").expect("file should be written");

        let result = attribute_shell_command("cat .env", temp.path(), &policy());

        assert_eq!(paths(&result), vec![".env"]);
        assert_eq!(result.files[0].zone, "secrets");
        assert_eq!(result.files[0].attribution, Attribution::Heuristic);
        assert!(!result.incomplete);
    }

    #[test]
    fn attributes_literal_directory_hint() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        std::fs::create_dir(temp.path().join("src")).expect("directory should be created");

        let result = attribute_shell_command("grep -rn foo src/", temp.path(), &policy());

        assert_eq!(paths(&result), vec!["src/"]);
        assert!(result
            .files
            .iter()
            .all(|file| file.attribution == Attribution::Heuristic));
        assert!(!result.incomplete);
    }

    #[test]
    fn marks_expansion_as_incomplete_without_guessing_path() {
        let temp = tempfile::tempdir().expect("tempdir should be created");

        let result = attribute_shell_command("cat $VAR", temp.path(), &policy());

        assert!(result.files.is_empty());
        assert!(result.incomplete);
    }

    #[test]
    fn attributes_known_path_but_marks_unknown_command_incomplete() {
        let temp = tempfile::tempdir().expect("tempdir should be created");

        let result = attribute_shell_command("tar cf - .", temp.path(), &policy());

        assert_eq!(paths(&result), vec!["."]);
        assert_eq!(result.files[0].attribution, Attribution::Heuristic);
        assert!(result.incomplete);
    }

    #[test]
    fn marks_pipe_to_interpreter_incomplete() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(temp.path().join("script.sh"), "echo hi").expect("file should be written");

        let result = attribute_shell_command("cat script.sh | sh", temp.path(), &policy());

        assert_eq!(paths(&result), vec!["script.sh"]);
        assert!(result.incomplete);
    }
}
