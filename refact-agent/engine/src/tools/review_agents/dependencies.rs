use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::LazyLock;

use regex::Regex;

use crate::global_context::GlobalContext;
use crate::tools::review_agents::{now_ms, AgentOutcome};
use crate::tools::review_scope::ReviewScope;
use crate::tools::review_types::{
    evidence_kinds, RankTier, ReviewEvidence, ReviewFinding, ReviewSeverity, VerificationStatus,
};

pub const AGENT_ID: &str = "s5_dependencies";
const MAX_FINDINGS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportLang {
    Rust,
    Js,
    Python,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportRef {
    pub file: String,
    pub line: u32,
    pub lang: ImportLang,
    pub module: String,
}

static RUST_USE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:pub\s+)?use\s+([A-Za-z_][A-Za-z0-9_]*)(?:::|;| )").unwrap()
});
static RUST_EXTERN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*extern\s+crate\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static JS_IMPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:import\s+[^'"]*from\s*|import\s*\(\s*|require\s*\(\s*)['"]([^'"]+)['"]"#)
        .unwrap()
});
static PY_IMPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:from\s+([A-Za-z_][A-Za-z0-9_]*)|import\s+([A-Za-z_][A-Za-z0-9_]*))")
        .unwrap()
});

const RUST_BUILTIN: &[&str] = &[
    "crate",
    "self",
    "super",
    "std",
    "core",
    "alloc",
    "test",
    "proc_macro",
];

const NODE_BUILTINS: &[&str] = &[
    "assert",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "dns",
    "domain",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "string_decoder",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

const PY_STDLIB: &[&str] = &[
    "abc",
    "argparse",
    "asyncio",
    "base64",
    "bisect",
    "builtins",
    "calendar",
    "collections",
    "concurrent",
    "configparser",
    "contextlib",
    "copy",
    "csv",
    "ctypes",
    "dataclasses",
    "datetime",
    "decimal",
    "difflib",
    "dis",
    "email",
    "enum",
    "errno",
    "fnmatch",
    "functools",
    "gc",
    "getpass",
    "glob",
    "gzip",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "http",
    "importlib",
    "inspect",
    "io",
    "itertools",
    "json",
    "logging",
    "math",
    "mimetypes",
    "multiprocessing",
    "operator",
    "os",
    "pathlib",
    "pickle",
    "platform",
    "pprint",
    "queue",
    "random",
    "re",
    "secrets",
    "select",
    "shlex",
    "shutil",
    "signal",
    "site",
    "socket",
    "sqlite3",
    "ssl",
    "stat",
    "statistics",
    "string",
    "struct",
    "subprocess",
    "sys",
    "tempfile",
    "textwrap",
    "threading",
    "time",
    "timeit",
    "tomllib",
    "traceback",
    "types",
    "typing",
    "unittest",
    "urllib",
    "uuid",
    "venv",
    "warnings",
    "weakref",
    "xml",
    "zipfile",
    "zoneinfo",
];

const PY_MODULE_ALIASES: &[(&str, &str)] = &[
    ("yaml", "pyyaml"),
    ("PIL", "pillow"),
    ("cv2", "opencv-python"),
    ("sklearn", "scikit-learn"),
    ("bs4", "beautifulsoup4"),
    ("dotenv", "python-dotenv"),
    ("dateutil", "python-dateutil"),
    ("attr", "attrs"),
];

#[derive(Debug, Default, Clone)]
pub struct ManifestIndex {
    pub rust: Option<BTreeSet<String>>,
    pub js: Option<BTreeSet<String>>,
    pub py: Option<BTreeSet<String>>,
}

fn norm_crate(name: &str) -> String {
    name.replace('-', "_").to_lowercase()
}

pub fn rust_deps_from_cargo_toml(text: &str) -> BTreeSet<String> {
    let mut deps = BTreeSet::new();
    let mut in_deps = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = trimmed.contains("dependencies");
            continue;
        }
        if trimmed.starts_with("name") && trimmed.contains('=') {
            if let Some(value) = trimmed.split('=').nth(1) {
                deps.insert(norm_crate(value.trim().trim_matches('"')));
            }
        }
        if in_deps {
            if let Some(name) = trimmed.split(['=', ' ', '.']).next() {
                if !name.is_empty() && !name.starts_with('#') {
                    deps.insert(norm_crate(name));
                }
            }
        }
    }
    deps
}

pub fn js_deps_from_package_json(text: &str) -> BTreeSet<String> {
    let mut deps = BTreeSet::new();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return deps;
    };
    for key in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(map) = value.get(key).and_then(|v| v.as_object()) {
            for name in map.keys() {
                deps.insert(name.clone());
            }
        }
    }
    deps
}

pub fn py_deps_from_requirements(text: &str) -> BTreeSet<String> {
    let mut deps = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
            continue;
        }
        if let Some(name) = trimmed
            .split(['=', '<', '>', '!', '~', ';', '[', ' '])
            .next()
        {
            if !name.is_empty() {
                deps.insert(name.to_lowercase());
            }
        }
    }
    deps
}

pub fn py_deps_from_pyproject(text: &str) -> BTreeSet<String> {
    static DEP_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#""([A-Za-z0-9_.-]+)\s*(?:[=<>!~\[;]|")"#).unwrap());
    let mut deps = BTreeSet::new();
    let mut in_deps = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("dependencies") && trimmed.contains('[') {
            in_deps = true;
        }
        if in_deps {
            for cap in DEP_RE.captures_iter(trimmed) {
                deps.insert(cap[1].to_lowercase());
            }
            if trimmed.ends_with(']') && !trimmed.starts_with("dependencies") {
                in_deps = false;
            }
        }
    }
    deps
}

pub fn parse_added_imports(patch: &str) -> Vec<ImportRef> {
    let mut imports = Vec::new();
    let mut current_file = String::new();
    let mut new_line: u32 = 0;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            current_file = rest
                .split_whitespace()
                .nth(1)
                .map(|p| p.trim_matches('"').trim_start_matches("b/").to_string())
                .unwrap_or_default();
            new_line = 0;
            continue;
        }
        if line.starts_with("@@") {
            new_line = line
                .split_whitespace()
                .find(|part| part.starts_with('+'))
                .and_then(|part| {
                    part.trim_start_matches('+')
                        .split(',')
                        .next()
                        .and_then(|v| v.parse::<u32>().ok())
                })
                .unwrap_or(1);
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if let Some(text) = line.strip_prefix('+') {
            let lang = if current_file.ends_with(".rs") {
                Some(ImportLang::Rust)
            } else if current_file.ends_with(".js")
                || current_file.ends_with(".jsx")
                || current_file.ends_with(".ts")
                || current_file.ends_with(".tsx")
                || current_file.ends_with(".mjs")
                || current_file.ends_with(".cjs")
            {
                Some(ImportLang::Js)
            } else if current_file.ends_with(".py") {
                Some(ImportLang::Python)
            } else {
                None
            };
            if let Some(lang) = lang {
                match lang {
                    ImportLang::Rust => {
                        for re in [&*RUST_USE_RE, &*RUST_EXTERN_RE] {
                            if let Some(cap) = re.captures(text) {
                                imports.push(ImportRef {
                                    file: current_file.clone(),
                                    line: new_line,
                                    lang,
                                    module: cap[1].to_string(),
                                });
                            }
                        }
                    }
                    ImportLang::Js => {
                        for cap in JS_IMPORT_RE.captures_iter(text) {
                            let spec = &cap[1];
                            if spec.starts_with('.') || spec.starts_with('/') {
                                continue;
                            }
                            if let Some(stripped) = spec.strip_prefix("node:") {
                                let _ = stripped;
                                continue;
                            }
                            let module = if spec.starts_with('@') {
                                spec.split('/').take(2).collect::<Vec<_>>().join("/")
                            } else {
                                spec.split('/').next().unwrap_or(spec).to_string()
                            };
                            imports.push(ImportRef {
                                file: current_file.clone(),
                                line: new_line,
                                lang,
                                module,
                            });
                        }
                    }
                    ImportLang::Python => {
                        if let Some(cap) = PY_IMPORT_RE.captures(text) {
                            let module = cap
                                .get(1)
                                .or_else(|| cap.get(2))
                                .map(|m| m.as_str().to_string());
                            if let Some(module) = module {
                                imports.push(ImportRef {
                                    file: current_file.clone(),
                                    line: new_line,
                                    lang,
                                    module,
                                });
                            }
                        }
                    }
                }
            }
            new_line = new_line.saturating_add(1);
        } else if !line.starts_with('-') {
            new_line = new_line.saturating_add(1);
        }
    }
    imports
}

pub fn missing_imports(imports: &[ImportRef], manifests: &ManifestIndex) -> Vec<ImportRef> {
    let mut missing = Vec::new();
    let mut seen = BTreeSet::new();
    for import in imports {
        let key = (import.lang, import.module.clone());
        if !seen.insert(key) {
            continue;
        }
        let declared = match import.lang {
            ImportLang::Rust => {
                let Some(deps) = &manifests.rust else {
                    continue;
                };
                if RUST_BUILTIN.contains(&import.module.as_str()) {
                    continue;
                }
                deps.contains(&norm_crate(&import.module))
            }
            ImportLang::Js => {
                let Some(deps) = &manifests.js else { continue };
                if NODE_BUILTINS.contains(&import.module.as_str()) {
                    continue;
                }
                deps.contains(&import.module)
            }
            ImportLang::Python => {
                let Some(deps) = &manifests.py else { continue };
                if PY_STDLIB.contains(&import.module.as_str()) {
                    continue;
                }
                let lowered = import.module.to_lowercase();
                let alias = PY_MODULE_ALIASES
                    .iter()
                    .find(|(module, _)| *module == import.module)
                    .map(|(_, package)| package.to_string());
                deps.contains(&lowered)
                    || deps.contains(&lowered.replace('_', "-"))
                    || alias.map(|a| deps.contains(&a)).unwrap_or(false)
            }
        };
        if !declared {
            missing.push(import.clone());
        }
    }
    missing
}

async fn read_if_exists(path: &Path) -> Option<String> {
    tokio::fs::read_to_string(path).await.ok()
}

pub async fn build_manifest_index(roots: &[PathBuf]) -> ManifestIndex {
    let mut index = ManifestIndex::default();
    for root in roots {
        if let Some(text) = read_if_exists(&root.join("Cargo.toml")).await {
            let deps = index.rust.get_or_insert_with(BTreeSet::new);
            deps.extend(rust_deps_from_cargo_toml(&text));
        }
        if let Some(text) = read_if_exists(&root.join("package.json")).await {
            let deps = index.js.get_or_insert_with(BTreeSet::new);
            deps.extend(js_deps_from_package_json(&text));
        }
        for name in ["requirements.txt", "requirements-dev.txt"] {
            if let Some(text) = read_if_exists(&root.join(name)).await {
                let deps = index.py.get_or_insert_with(BTreeSet::new);
                deps.extend(py_deps_from_requirements(&text));
            }
        }
        if let Some(text) = read_if_exists(&root.join("pyproject.toml")).await {
            let deps = index.py.get_or_insert_with(BTreeSet::new);
            deps.extend(py_deps_from_pyproject(&text));
        }
    }
    index
}

fn manifest_roots(gcx: &GlobalContext, scope: &ReviewScope) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(folders) = gcx.documents_state.workspace_folders.lock() {
        roots.extend(folders.iter().cloned());
    }
    for file in &scope.files {
        let mut dir = file.parent();
        let mut hops = 0;
        while let Some(current) = dir {
            if hops >= 4 {
                break;
            }
            if !roots.contains(&current.to_path_buf()) {
                roots.push(current.to_path_buf());
            }
            dir = current.parent();
            hops += 1;
        }
    }
    roots.truncate(24);
    roots
}

pub async fn s5_dependencies(gcx: Arc<GlobalContext>, scope: &ReviewScope) -> AgentOutcome {
    let started = now_ms();
    let Some(patch) = scope.diff_patch.as_deref() else {
        return AgentOutcome::skipped(AGENT_ID, "no_diff_patch");
    };
    let imports = parse_added_imports(patch);
    if imports.is_empty() {
        return AgentOutcome::ran(AGENT_ID, None, 0, vec![], started);
    }
    let roots = manifest_roots(gcx.as_ref(), scope);
    let manifests = build_manifest_index(&roots).await;
    let missing = missing_imports(&imports, &manifests);
    let candidates = missing.len();
    let findings = missing
        .into_iter()
        .take(MAX_FINDINGS)
        .map(|import| {
            let manifest = match import.lang {
                ImportLang::Rust => "Cargo.toml",
                ImportLang::Js => "package.json",
                ImportLang::Python => "requirements/pyproject",
            };
            ReviewFinding {
                id: String::new(),
                category: "correctness".to_string(),
                severity: ReviewSeverity::High,
                confidence: 0.7,
                verification_status: VerificationStatus::Unverified,
                rank_tier: RankTier::Unverified,
                sources: vec![AGENT_ID.to_string()],
                file: import.file.clone(),
                line1: import.line.max(1),
                line2: import.line.max(1),
                claim: format!(
                    "Import `{}` is not declared in {manifest}; it may be a hallucinated or missing dependency.",
                    import.module
                ),
                evidence: vec![ReviewEvidence {
                    kind: evidence_kinds::STATIC_FACT.to_string(),
                    path: Some(import.file.clone()),
                    line1: Some(import.line.max(1)),
                    line2: Some(import.line.max(1)),
                    content: format!("new import `{}` not found in {manifest}", import.module),
                }],
                impact: None,
                remediation: None,
                checks_performed: vec!["s5:missing_dependency".to_string()],
            }
        })
        .collect();
    AgentOutcome::ran(AGENT_ID, None, candidates, findings, started)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s5_parses_added_imports_across_langs() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n\
            @@ -1,0 +1,2 @@\n\
            +use serde_json::Value;\n\
            +use crate::tools;\n\
            diff --git a/web/app.ts b/web/app.ts\n\
            @@ -1,0 +1,3 @@\n\
            +import { x } from \"@scope/pkg/sub\";\n\
            +import fs from \"node:fs\";\n\
            +const y = require(\"leftpad\");\n\
            diff --git a/tools/run.py b/tools/run.py\n\
            @@ -1,0 +1,2 @@\n\
            +import requests\n\
            +from yaml import safe_load\n";
        let imports = parse_added_imports(patch);
        let modules: Vec<&str> = imports.iter().map(|i| i.module.as_str()).collect();
        assert!(modules.contains(&"serde_json"));
        assert!(modules.contains(&"crate"));
        assert!(modules.contains(&"@scope/pkg"));
        assert!(modules.contains(&"leftpad"));
        assert!(modules.contains(&"requests"));
        assert!(modules.contains(&"yaml"));
        assert!(!modules.contains(&"fs"));
    }

    #[test]
    fn s5_flags_only_undeclared_imports() {
        let mut manifests = ManifestIndex::default();
        manifests.rust = Some(rust_deps_from_cargo_toml(
            "[package]\nname = \"my-app\"\n[dependencies]\nserde_json = \"1\"\ntokio = { version = \"1\" }\n",
        ));
        manifests.js = Some(js_deps_from_package_json(
            r#"{"dependencies":{"react":"18"},"devDependencies":{"vitest":"1"}}"#,
        ));
        manifests.py = Some(py_deps_from_requirements("requests==2.31\npyyaml>=6\n"));
        let imports = vec![
            ImportRef {
                file: "a.rs".into(),
                line: 1,
                lang: ImportLang::Rust,
                module: "serde_json".into(),
            },
            ImportRef {
                file: "a.rs".into(),
                line: 2,
                lang: ImportLang::Rust,
                module: "left_pad".into(),
            },
            ImportRef {
                file: "a.rs".into(),
                line: 3,
                lang: ImportLang::Rust,
                module: "std".into(),
            },
            ImportRef {
                file: "a.rs".into(),
                line: 4,
                lang: ImportLang::Rust,
                module: "my_app".into(),
            },
            ImportRef {
                file: "b.ts".into(),
                line: 1,
                lang: ImportLang::Js,
                module: "react".into(),
            },
            ImportRef {
                file: "b.ts".into(),
                line: 2,
                lang: ImportLang::Js,
                module: "leftpad".into(),
            },
            ImportRef {
                file: "b.ts".into(),
                line: 3,
                lang: ImportLang::Js,
                module: "path".into(),
            },
            ImportRef {
                file: "c.py".into(),
                line: 1,
                lang: ImportLang::Python,
                module: "requests".into(),
            },
            ImportRef {
                file: "c.py".into(),
                line: 2,
                lang: ImportLang::Python,
                module: "yaml".into(),
            },
            ImportRef {
                file: "c.py".into(),
                line: 3,
                lang: ImportLang::Python,
                module: "os".into(),
            },
            ImportRef {
                file: "c.py".into(),
                line: 4,
                lang: ImportLang::Python,
                module: "flask".into(),
            },
        ];
        let missing = missing_imports(&imports, &manifests);
        let modules: Vec<&str> = missing.iter().map(|i| i.module.as_str()).collect();
        assert_eq!(modules, vec!["left_pad", "leftpad", "flask"]);
    }

    #[test]
    fn s5_missing_manifest_means_no_findings_for_that_lang() {
        let manifests = ManifestIndex::default();
        let imports = vec![ImportRef {
            file: "a.rs".into(),
            line: 1,
            lang: ImportLang::Rust,
            module: "left_pad".into(),
        }];
        assert!(missing_imports(&imports, &manifests).is_empty());
    }

    #[test]
    fn s5_pyproject_deps_parse() {
        let deps = py_deps_from_pyproject(
            "[project]\ndependencies = [\n  \"fastapi>=0.100\",\n  \"uvicorn[standard]\",\n]\n",
        );
        assert!(deps.contains("fastapi"));
        assert!(deps.contains("uvicorn"));
    }
}
