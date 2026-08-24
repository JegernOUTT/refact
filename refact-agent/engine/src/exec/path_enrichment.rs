use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use regex::Regex;
use serde::Serialize;

const MAX_SCAN_BYTES: usize = 128 * 1024;
const MAX_SCAN_LINES: usize = 2_000;
const MAX_REFERENCES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathReference {
    pub path: String,
    pub line1: Option<u32>,
    pub line2: Option<u32>,
    pub column1: Option<u32>,
    pub column2: Option<u32>,
    pub source: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathEnrichment {
    pub schema_version: u8,
    pub references: Vec<PathReference>,
    pub truncated: bool,
    pub omitted_count: usize,
    pub withheld_count: usize,
}

#[derive(Debug, Clone)]
pub struct PathEnrichmentCandidate {
    pub reference: PathReference,
    pub canonical_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CollectedPathEnrichment {
    pub metadata: PathEnrichment,
    pub candidates: Vec<PathEnrichmentCandidate>,
}

pub fn collect(
    command: &str,
    cwd: &Path,
    workspace: &Path,
    output: &str,
) -> CollectedPathEnrichment {
    let workspace = canonical_workspace(workspace);
    let cwd = canonical_cwd(cwd, &workspace);
    let mut collector = Collector::new(workspace, cwd);
    collector.collect_argv(command);
    collector.collect_diagnostics(output);
    collector.finish()
}

fn canonical_workspace(workspace: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(workspace)
        .ok()
        .filter(|path| path.is_dir())
}

fn canonical_cwd(cwd: &Path, workspace: &Option<PathBuf>) -> PathBuf {
    std::fs::canonicalize(cwd)
        .ok()
        .or_else(|| workspace.clone())
        .unwrap_or_else(|| cwd.to_path_buf())
}

struct Collector {
    workspace: Option<PathBuf>,
    cwd: PathBuf,
    metadata: PathEnrichment,
    candidates: Vec<PathEnrichmentCandidate>,
    seen: HashSet<PathBuf>,
}

impl Collector {
    fn new(workspace: Option<PathBuf>, cwd: PathBuf) -> Self {
        Self {
            workspace,
            cwd,
            metadata: PathEnrichment {
                schema_version: 1,
                references: Vec::new(),
                truncated: false,
                omitted_count: 0,
                withheld_count: 0,
            },
            candidates: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn collect_argv(&mut self, command: &str) {
        let Ok(argv) = shell_words::split(command) else {
            self.metadata.omitted_count += 1;
            return;
        };
        for operand in argv.into_iter().skip(1) {
            if operand.starts_with('-') || is_shell_expansion(&operand) {
                continue;
            }
            self.push_candidate(&operand, None, None, None, None, "argv", "high");
        }
    }

    fn collect_diagnostics(&mut self, output: &str) {
        let bytes = output.as_bytes();
        let (output, byte_truncated) = if bytes.len() > MAX_SCAN_BYTES {
            (&output[..valid_char_boundary(output, MAX_SCAN_BYTES)], true)
        } else {
            (output, false)
        };
        self.metadata.truncated |= byte_truncated;
        for (index, line) in output.lines().enumerate() {
            if index >= MAX_SCAN_LINES {
                self.metadata.truncated = true;
                break;
            }
            self.collect_diagnostic_line(line);
        }
    }

    fn collect_diagnostic_line(&mut self, line: &str) {
        let line = strip_ansi(line);
        for pattern in diagnostic_patterns() {
            let Some(captures) = pattern.captures(&line) else {
                continue;
            };
            let Some(path) = captures.name("path").map(|value| value.as_str()) else {
                continue;
            };
            let line1 = captures
                .name("line")
                .and_then(|value| value.as_str().parse::<u32>().ok())
                .filter(|value| *value > 0);
            let column1 = captures
                .name("column")
                .and_then(|value| value.as_str().parse::<u32>().ok())
                .filter(|value| *value > 0);
            self.push_candidate(path, line1, line1, column1, column1, "diagnostic", "high");
            break;
        }
    }

    fn push_candidate(
        &mut self,
        raw_path: &str,
        line1: Option<u32>,
        line2: Option<u32>,
        column1: Option<u32>,
        column2: Option<u32>,
        source: &str,
        confidence: &str,
    ) {
        let Some((canonical_path, path)) = self.resolve(raw_path) else {
            self.metadata.omitted_count += 1;
            return;
        };
        if !self.seen.insert(canonical_path.clone()) {
            return;
        }
        if self.candidates.len() >= MAX_REFERENCES {
            self.metadata.truncated = true;
            self.metadata.omitted_count += 1;
            return;
        }
        let reference = PathReference {
            path,
            line1,
            line2,
            column1,
            column2,
            source: source.to_string(),
            confidence: confidence.to_string(),
        };
        self.metadata.references.push(reference.clone());
        self.candidates.push(PathEnrichmentCandidate {
            reference,
            canonical_path,
        });
    }

    fn resolve(&self, raw_path: &str) -> Option<(PathBuf, String)> {
        let raw_path = raw_path.trim().trim_matches(['\'', '"']);
        if raw_path.is_empty()
            || raw_path.contains("://")
            || raw_path.starts_with('#')
            || raw_path.starts_with('-')
            || is_shell_expansion(raw_path)
        {
            return None;
        }
        let path = Path::new(raw_path);
        if path
            .components()
            .any(|component| component == Component::ParentDir)
        {
            return None;
        }
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        let canonical_path = std::fs::canonicalize(candidate).ok()?;
        if !canonical_path.is_file() {
            return None;
        }
        let workspace = self.workspace.as_ref()?;
        let relative = canonical_path.strip_prefix(workspace).ok()?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        (!relative.is_empty()).then_some((canonical_path, relative))
    }

    fn finish(self) -> CollectedPathEnrichment {
        CollectedPathEnrichment {
            metadata: self.metadata,
            candidates: self.candidates,
        }
    }
}

fn is_shell_expansion(value: &str) -> bool {
    value.contains('$')
        || value.contains('`')
        || value.contains('*')
        || value.contains('?')
        || value.contains('[')
        || value.contains('{')
        || value.starts_with('~')
}

fn valid_char_boundary(value: &str, limit: usize) -> usize {
    let mut index = limit.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn strip_ansi(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            while let Some(code) = chars.next() {
                if ('@'..='~').contains(&code) {
                    break;
                }
            }
        } else {
            result.push(character);
        }
    }
    result
}

fn diagnostic_patterns() -> &'static [Regex] {
    static PATTERNS: std::sync::LazyLock<Vec<Regex>> = std::sync::LazyLock::new(|| {
        vec![
            Regex::new(r#"(?:^|\s|-->)\s*(?P<path>[^\s:()]+(?:\.[[:alnum:]_+-]+)):(?P<line>[0-9]+):(?P<column>[0-9]+)(?:\s|:|$)"#).unwrap(),
            Regex::new(r#"(?:^|\s)(?P<path>[^\s:()]+(?:\.[[:alnum:]_+-]+)):(?P<line>[0-9]+):(?:\s|$)"#).unwrap(),
            Regex::new(r#"(?P<path>[^\s()]+(?:\.[[:alnum:]_+-]+))\((?P<line>[0-9]+),(?P<column>[0-9]+)\)"#).unwrap(),
            Regex::new(r#"File \"(?P<path>[^\"]+)\", line (?P<line>[0-9]+)"#).unwrap(),
            Regex::new(r#"\((?P<path>[^()\s]+(?:\.[[:alnum:]_+-]+)):(?P<line>[0-9]+)\)"#).unwrap(),
        ]
    });
    PATTERNS.as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_paths(result: &CollectedPathEnrichment) -> Vec<&str> {
        result
            .metadata
            .references
            .iter()
            .map(|reference| reference.path.as_str())
            .collect()
    }

    #[test]
    fn extracts_argv_and_compiler_diagnostics_without_changing_input() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src").join("main.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let output = "\u{1b}[31m--> src/main.rs:7:3\u{1b}[0m\n";

        let result = collect("cat 'src/main.rs'", temp.path(), temp.path(), output);

        assert_eq!(reference_paths(&result), vec!["src/main.rs"]);
        assert_eq!(result.metadata.references[0].source, "argv");
        assert_eq!(output, "\u{1b}[31m--> src/main.rs:7:3\u{1b}[0m\n");
    }

    #[test]
    fn extracts_typescript_python_and_java_stack_formats() {
        let temp = tempfile::tempdir().unwrap();
        for path in ["web/app.ts", "python/app.py", "java/App.java"] {
            let path = temp.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x\n").unwrap();
        }
        let output = "web/app.ts(2,4): error\nFile \"python/app.py\", line 3\n at Main.run(java/App.java:4)\n";

        let result = collect("true", temp.path(), temp.path(), output);

        assert_eq!(
            reference_paths(&result),
            vec!["web/app.ts", "python/app.py", "java/App.java"]
        );
        assert_eq!(result.metadata.references[0].column1, Some(4));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_traversal_symlink_escape_and_out_of_scope_paths() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside.txt");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(&outside, "outside\n").unwrap();
        symlink(&outside, workspace.join("escape.txt")).unwrap();

        let result = collect(
            "cat ../outside.txt escape.txt",
            &workspace,
            &workspace,
            "../outside.txt:1:1\nescape.txt:1:1\n",
        );

        assert!(result.metadata.references.is_empty());
        assert!(result.metadata.omitted_count >= 4);
    }

    #[test]
    fn bounds_adversarial_output() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src/lib.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "x\n").unwrap();
        let output = "src/lib.rs:1:1\n".repeat(MAX_SCAN_LINES + 10);

        let result = collect("true", temp.path(), temp.path(), &output);

        assert_eq!(result.metadata.references.len(), 1);
        assert!(result.metadata.truncated);
    }
}
