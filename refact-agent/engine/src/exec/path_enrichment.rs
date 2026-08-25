use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;

const MAX_SCAN_BYTES: usize = 128 * 1024;
const MAX_SCAN_LINES: usize = 2_000;
const MAX_REFERENCES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathReference {
    pub(crate) path: String,
    pub(crate) line1: Option<u32>,
    pub(crate) line2: Option<u32>,
    pub(crate) column1: Option<u32>,
    pub(crate) column2: Option<u32>,
    pub(crate) source: String,
    pub(crate) confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathEnrichment {
    pub(crate) references: Vec<PathReference>,
    pub(crate) truncated: bool,
    pub(crate) omitted_count: usize,
    pub(crate) withheld_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct PathEnrichmentCandidate {
    pub(crate) reference: PathReference,
    pub(crate) canonical_path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct CollectedPathEnrichment {
    pub(crate) metadata: PathEnrichment,
    pub(crate) candidates: Vec<PathEnrichmentCandidate>,
}

#[derive(Debug, Clone)]
struct ResolvedPath {
    canonical_path: PathBuf,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReferenceKey {
    canonical_path: PathBuf,
    line1: Option<u32>,
    line2: Option<u32>,
    column1: Option<u32>,
    column2: Option<u32>,
}

#[cfg(test)]
pub(crate) fn collect(
    command: &str,
    cwd: &Path,
    workspace: &Path,
    output: &str,
) -> CollectedPathEnrichment {
    collect_in_roots(command, cwd, &[workspace.to_path_buf()], output)
}

#[cfg(test)]
pub(crate) fn collect_in_roots(
    command: &str,
    cwd: &Path,
    workspace_roots: &[PathBuf],
    output: &str,
) -> CollectedPathEnrichment {
    let (output, output_truncated) = bounded_output(output);
    collect_with_truncation(
        command,
        cwd,
        workspace_roots,
        &output,
        output_truncated,
        None,
    )
}

pub(crate) async fn collect_async(
    command: &str,
    cwd: &Path,
    workspace_roots: &[PathBuf],
    output: &str,
) -> CollectedPathEnrichment {
    let (output, output_truncated) = bounded_output(output);
    collect_async_with_truncation(command, cwd, workspace_roots, output, output_truncated).await
}

pub(crate) async fn collect_async_with_truncation(
    command: &str,
    cwd: &Path,
    workspace_roots: &[PathBuf],
    output: String,
    output_truncated: bool,
) -> CollectedPathEnrichment {
    let command = command.to_string();
    let cwd = cwd.to_path_buf();
    let workspace_roots = workspace_roots.to_vec();
    tokio::task::spawn_blocking(move || {
        collect_with_truncation(
            &command,
            &cwd,
            &workspace_roots,
            &output,
            output_truncated,
            None,
        )
    })
    .await
    .unwrap_or_else(|_| empty_collected(output_truncated))
}

fn empty_collected(truncated: bool) -> CollectedPathEnrichment {
    CollectedPathEnrichment {
        metadata: PathEnrichment {
            references: Vec::new(),
            truncated,
            omitted_count: 0,
            withheld_count: 0,
        },
        candidates: Vec::new(),
    }
}

fn collect_with_truncation(
    command: &str,
    cwd: &Path,
    workspace_roots: &[PathBuf],
    output: &str,
    output_truncated: bool,
    argv: Option<&[String]>,
) -> CollectedPathEnrichment {
    let workspace_roots = canonical_workspace_roots(workspace_roots);
    let cwd = canonical_cwd(cwd, &workspace_roots);
    let mut collector = Collector::new(workspace_roots, cwd, output_truncated);
    collector.collect_argv(command, argv);
    collector.collect_diagnostics(output);
    collector.finish()
}

fn canonical_workspace_roots(workspace_roots: &[PathBuf]) -> Vec<PathBuf> {
    workspace_roots
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .filter(|root| root.is_dir())
        .collect()
}

fn canonical_cwd(cwd: &Path, workspace_roots: &[PathBuf]) -> PathBuf {
    std::fs::canonicalize(cwd)
        .ok()
        .or_else(|| workspace_roots.first().cloned())
        .unwrap_or_else(|| cwd.to_path_buf())
}

fn bounded_output(output: &str) -> (String, bool) {
    let prefix = refact_core::string_utils::safe_truncate(output, MAX_SCAN_BYTES);
    (prefix.to_string(), prefix.len() < output.len())
}

pub(crate) fn bounded_diagnostic_output(stdout: &str, stderr: &str) -> (String, bool) {
    let total_len = stdout.len().saturating_add(1).saturating_add(stderr.len());
    let mut output = String::with_capacity(total_len.min(MAX_SCAN_BYTES));
    output.push_str(refact_core::string_utils::safe_truncate(
        stdout,
        MAX_SCAN_BYTES,
    ));
    if output.len() < MAX_SCAN_BYTES {
        output.push('\n');
        let remaining = MAX_SCAN_BYTES.saturating_sub(output.len());
        output.push_str(refact_core::string_utils::safe_truncate(stderr, remaining));
    }
    (output, total_len > MAX_SCAN_BYTES)
}

struct Collector {
    workspace_roots: Vec<PathBuf>,
    cwd: PathBuf,
    metadata: PathEnrichment,
    candidates: Vec<PathEnrichmentCandidate>,
    seen: HashMap<ReferenceKey, usize>,
    resolution_cache: HashMap<String, Option<ResolvedPath>>,
    resolution_attempts: usize,
}

impl Collector {
    fn new(workspace_roots: Vec<PathBuf>, cwd: PathBuf, output_truncated: bool) -> Self {
        Self {
            workspace_roots,
            cwd,
            metadata: PathEnrichment {
                references: Vec::new(),
                truncated: output_truncated,
                omitted_count: 0,
                withheld_count: 0,
            },
            candidates: Vec::new(),
            seen: HashMap::new(),
            resolution_cache: HashMap::new(),
            resolution_attempts: 0,
        }
    }

    fn collect_argv(&mut self, command: &str, argv: Option<&[String]>) {
        if let Some(argv) = argv {
            self.collect_argv_operands(argv.iter().map(String::as_str), "high");
            return;
        }
        if looks_like_windows_command(command) {
            let argv = split_windows_command(command);
            self.collect_argv_operands(argv.iter().map(String::as_str), "low");
            return;
        }
        let Ok(argv) = shell_words::split(command) else {
            self.metadata.omitted_count += 1;
            return;
        };
        self.collect_argv_operands(argv.iter().map(String::as_str), "high");
    }

    fn collect_argv_operands<'a>(
        &mut self,
        operands: impl IntoIterator<Item = &'a str>,
        confidence: &str,
    ) {
        for operand in operands.into_iter().skip(1) {
            if operand.starts_with('-') || is_shell_expansion(operand) {
                continue;
            }
            self.push_candidate(operand, None, None, None, None, "argv", confidence);
        }
    }

    fn collect_diagnostics(&mut self, output: &str) {
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
        let Some(resolved) = self.resolve(raw_path) else {
            self.metadata.omitted_count += 1;
            return;
        };
        let key = ReferenceKey {
            canonical_path: resolved.canonical_path.clone(),
            line1,
            line2,
            column1,
            column2,
        };
        let reference = PathReference {
            path: resolved.path,
            line1,
            line2,
            column1,
            column2,
            source: source.to_string(),
            confidence: confidence.to_string(),
        };
        if let Some(index) = self.seen.get(&key).copied() {
            merge_reference(&mut self.metadata.references[index], &reference);
            merge_reference(&mut self.candidates[index].reference, &reference);
            return;
        }
        if self.candidates.len() >= MAX_REFERENCES {
            self.metadata.truncated = true;
            self.metadata.omitted_count += 1;
            return;
        }
        self.seen.insert(key, self.candidates.len());
        self.metadata.references.push(reference.clone());
        self.candidates.push(PathEnrichmentCandidate {
            reference,
            canonical_path: resolved.canonical_path,
        });
    }

    fn resolve(&mut self, raw_path: &str) -> Option<ResolvedPath> {
        let cache_key = raw_path.to_string();
        if let Some(resolved) = self.resolution_cache.get(&cache_key) {
            return resolved.clone();
        }
        self.resolution_attempts += 1;
        let resolved = self.resolve_uncached(raw_path);
        self.resolution_cache.insert(cache_key, resolved.clone());
        resolved
    }

    fn resolve_uncached(&self, raw_path: &str) -> Option<ResolvedPath> {
        let raw_path = raw_path.trim().trim_matches(['\'', '"']);
        if raw_path.is_empty()
            || raw_path.contains("://")
            || raw_path.starts_with('#')
            || raw_path.starts_with('-')
            || is_shell_expansion(raw_path)
        {
            return None;
        }
        let raw_path = raw_path.replace('\\', "/");
        if raw_path.starts_with("//") || has_windows_drive_prefix(&raw_path) {
            return None;
        }
        let path = Path::new(&raw_path);
        if path.is_absolute() {
            return None;
        }
        let candidate = self.cwd.join(path);
        let canonical_path = std::fs::canonicalize(candidate).ok()?;
        if !canonical_path.is_file() {
            return None;
        }
        let workspace = self
            .workspace_roots
            .iter()
            .filter(|root| canonical_path.starts_with(root))
            .max_by_key(|root| root.components().count())?;
        let relative = canonical_path.strip_prefix(workspace).ok()?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        (!relative.is_empty()).then_some(ResolvedPath {
            canonical_path,
            path: relative,
        })
    }

    fn finish(self) -> CollectedPathEnrichment {
        CollectedPathEnrichment {
            metadata: self.metadata,
            candidates: self.candidates,
        }
    }
}

fn merge_reference(existing: &mut PathReference, incoming: &PathReference) {
    if incoming.source == "diagnostic" && existing.source != "diagnostic" {
        *existing = incoming.clone();
    }
}

fn has_windows_drive_prefix(value: &str) -> bool {
    value
        .as_bytes()
        .get(1)
        .is_some_and(|character| *character == b':')
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
}

fn looks_like_windows_command(command: &str) -> bool {
    if cfg!(target_os = "windows") {
        return true;
    }
    let program = command
        .trim_start_matches(|character: char| matches!(character, '&' | ' '))
        .split_ascii_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(['\'', '"'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        program.as_str(),
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe" | "cmd" | "cmd.exe"
    ) || matches!(
        program.as_str(),
        "get-childitem" | "get-content" | "set-location" | "test-path" | "select-string"
    )
}

fn split_windows_command(command: &str) -> Vec<String> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = command.chars();
    while let Some(character) = chars.next() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else if character == '`' {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            } else {
                current.push(character);
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character.is_ascii_whitespace() {
            if !current.is_empty() {
                argv.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if !current.is_empty() {
        argv.push(current);
    }
    argv
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
        let path = r#"(?:[^\s:()]+(?:\.[[:alnum:]_+-]+)|(?:[^\s:()]+[\\/])?(?:Makefile|Dockerfile|Justfile))"#;
        vec![
            Regex::new(&format!(
                r#"(?:^|\s|-->)\s*(?P<path>{path}):(?P<line>[0-9]+):(?P<column>[0-9]+)(?:\s|:|$)"#
            ))
            .unwrap(),
            Regex::new(&format!(
                r#"(?:^|\s)(?P<path>{path}):(?P<line>[0-9]+):(?:\s|$)"#
            ))
            .unwrap(),
            Regex::new(&format!(
                r#"(?P<path>{path})\((?P<line>[0-9]+),(?P<column>[0-9]+)\)"#
            ))
            .unwrap(),
            Regex::new(r#"File \"(?P<path>[^\"]+)\", line (?P<line>[0-9]+)"#).unwrap(),
            Regex::new(&format!(r#"\((?P<path>{path}):(?P<line>[0-9]+)\)"#)).unwrap(),
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

        assert_eq!(reference_paths(&result), vec!["src/main.rs", "src/main.rs"]);
        assert_eq!(result.metadata.references[0].source, "argv");
        assert_eq!(result.metadata.references[1].source, "diagnostic");
        assert_eq!(result.metadata.references[1].line1, Some(7));
        assert_eq!(result.metadata.references[1].column1, Some(3));
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

    #[test]
    fn normalizes_windows_separators_and_extensionless_diagnostics() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src/lib.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "pub fn visible() {}\n").unwrap();
        for name in ["Makefile", "Dockerfile", "Justfile"] {
            std::fs::write(temp.path().join(name), "all:\n").unwrap();
        }
        let nested = temp.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();

        let result = collect(
            "true",
            &nested,
            temp.path(),
            "..\\src\\lib.rs:2:3\n../src/lib.rs:3:5\n../Makefile:1:1\n../Dockerfile:1:1\n../Justfile:1:1\n",
        );

        assert_eq!(
            reference_paths(&result),
            vec![
                "src/lib.rs",
                "src/lib.rs",
                "Makefile",
                "Dockerfile",
                "Justfile"
            ]
        );
        assert_eq!(result.metadata.references[0].column1, Some(3));
        assert_eq!(result.metadata.references[1].column1, Some(5));
    }

    #[test]
    fn resolves_each_raw_candidate_once() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src/lib.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "x\n").unwrap();
        let roots = canonical_workspace_roots(&[temp.path().to_path_buf()]);
        let cwd = canonical_cwd(temp.path(), &roots);
        let mut collector = Collector::new(roots, cwd, false);

        collector.collect_diagnostics("src/lib.rs:1:1\nsrc/lib.rs:2:1\n");

        assert_eq!(collector.resolution_attempts, 1);
        assert_eq!(collector.metadata.references.len(), 2);
    }

    #[test]
    fn windows_command_parsing_preserves_quoted_operands_at_low_confidence() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src/space name.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "x\n").unwrap();
        let command = "powershell Get-Content 'src\\space name.rs'";

        let result = collect(command, temp.path(), temp.path(), "");

        assert_eq!(reference_paths(&result), vec!["src/space name.rs"]);
        assert_eq!(result.metadata.references[0].confidence, "low");
    }

    #[test]
    fn enriches_secondary_workspace_roots() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        let file = second.join("src/lib.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&first).unwrap();
        std::fs::write(file, "x\n").unwrap();

        let result = collect_in_roots(
            "cat ../second/src/lib.rs",
            &first,
            &[first.clone(), second],
            "",
        );

        assert_eq!(reference_paths(&result), vec!["src/lib.rs"]);
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
            "../outside.txt:1:1\nescape.txt:1:1\nC:\\outside.txt:1:1\n\\\\host\\share\\file.rs:1:1\nhttps://example.test/file.rs:1:1\n",
        );

        assert!(result.metadata.references.is_empty());
        assert!(result.metadata.omitted_count >= 4);
    }

    #[test]
    fn bounds_adversarial_output() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("src/lib.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "x\n").unwrap();
        let output = "src/lib.rs:1:1\n".repeat(MAX_SCAN_LINES + 10);

        let result = collect("true", temp.path(), temp.path(), &output);

        assert_eq!(result.metadata.references.len(), 1);
        assert!(result.metadata.truncated);
    }

    #[test]
    fn bounds_huge_lines_without_copying_the_full_output() {
        let temp = tempfile::tempdir().unwrap();
        let output = format!("{}src/lib.rs:1:1", "x".repeat(MAX_SCAN_BYTES * 2));

        let result = collect("true", temp.path(), temp.path(), &output);

        assert!(result.metadata.references.is_empty());
        assert!(result.metadata.truncated);
    }
}
