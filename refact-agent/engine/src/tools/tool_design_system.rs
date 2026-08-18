use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::Mutex as AMutex;
use walkdir::{DirEntry, WalkDir};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::codegraph::code_intel_api::ToolJson;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const DEFAULT_MAX_FINDINGS: usize = 100;
const MAX_FINDINGS: usize = 500;
const MAX_COMPONENTS: usize = 100;
const MAX_TOKEN_OUTPUT: usize = 300;
const MAX_OUTPUT_CHARS: usize = 60_000;
const MAX_OUTPUT_TOKENS: usize = 16_000;
const MAX_SCAN_FILES: usize = 20_000;
const MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct DesignSystemArgs {
    scope: Option<String>,
    #[serde(default = "default_true")]
    include_components: bool,
    #[serde(default = "default_true")]
    include_drift: bool,
    #[serde(default = "default_max_findings")]
    max_findings: usize,
}

fn default_true() -> bool {
    true
}

fn default_max_findings() -> usize {
    DEFAULT_MAX_FINDINGS
}

#[derive(Clone, Debug)]
struct SourceFile {
    absolute: PathBuf,
    relative: String,
    text: String,
}

#[derive(Clone, Debug)]
struct TokenRecord {
    name: String,
    category: String,
    source: String,
    priority: u8,
    variants: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct ComponentRecord {
    name: String,
    path: String,
    exported: bool,
    props: Vec<String>,
    variants: BTreeMap<String, Vec<String>>,
    approx_usage_count: usize,
    usage_count: usize,
    method: &'static str,
    heuristic: bool,
    source: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct DriftFinding {
    kind: String,
    value: String,
    path: String,
    line: usize,
    nearest_token: Option<String>,
    nearest_value: Option<String>,
    heuristic: bool,
    method: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct DesignSystemResponse {
    detected: bool,
    scope: String,
    looked_for: Vec<String>,
    scanned_files: usize,
    scanned_bytes: u64,
    scan_truncated: bool,
    token_sources: Vec<String>,
    detected_prefixes: Vec<String>,
    token_count: usize,
    token_output_count: usize,
    tokens_truncated: bool,
    token_categories: BTreeMap<String, usize>,
    tokens: Value,
    component_inventory_source: String,
    component_count: usize,
    component_output_count: usize,
    components_truncated: bool,
    components: Vec<ComponentRecord>,
    drift_count: usize,
    drift_output_count: usize,
    findings_truncated: bool,
    drift: Vec<DriftFinding>,
}

struct GraphFacts {
    generation: u64,
    records: Vec<(i64, String, String, String, Option<String>)>,
    incoming: HashMap<i64, usize>,
}

pub struct ToolDesignSystem {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolDesignSystem {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let args: DesignSystemArgs =
            serde_json::from_value(Value::Object(args.clone().into_iter().collect()))
                .map_err(|error| format!("invalid arguments: {error}"))?;
        if args.max_findings == 0 || args.max_findings > MAX_FINDINGS {
            return Err(format!("max_findings must be between 1 and {MAX_FINDINGS}"));
        }
        let (gcx, execution_scope) = {
            let guard = ccx.lock().await;
            (guard.app.gcx.clone(), guard.execution_scope.clone())
        };
        let workspace_root = match execution_scope {
            Some(scope) => {
                scope.ensure_active_root()?;
                scope.effective_root().to_path_buf()
            }
            None => crate::files_correction::get_project_dirs(gcx.clone())
                .await
                .into_iter()
                .next()
                .ok_or_else(|| "No workspace root is available".to_string())?,
        };
        let workspace_root = std::fs::canonicalize(&workspace_root)
            .map_err(|error| format!("failed to resolve workspace root: {error}"))?;
        let scope = resolve_scope(&workspace_root, args.scope.as_deref())?;
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let paths = collect_scan_paths(&scope);
        let paths =
            crate::files_in_workspace::filter_privacy_allowed_files(gcx.clone(), paths).await;
        let (files, scanned_bytes, scan_truncated) = load_source_files(&workspace_root, paths);
        let codegraph = gcx.codegraph.lock().await.clone();
        let graph = load_graph_facts(codegraph.as_ref()).await;
        let report = analyze_design_system(
            &workspace_root,
            &scope,
            &files,
            graph.as_ref(),
            args.include_components,
            args.include_drift,
            args.max_findings,
            scanned_bytes,
            scan_truncated,
        );
        let summary = if !report.detected {
            format!(
                "No design system detected in `{}` after checking token files, theme configs, and UI components",
                report.scope
            )
        } else {
            format!(
                "Design system: {} tokens, {} components, {} drift findings",
                report.token_count, report.component_count, report.drift_count
            )
        };
        let record_paths = report_paths(&workspace_root, &report);
        let text = ToolJson::new("design_system", summary, report).to_text();
        let mut messages = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            output_filter: Some(design_system_output_filter()),
            ..Default::default()
        })];
        let records = crate::privacy::records::declared_file_records(&gcx, record_paths)?;
        if let Some(ContextEnum::ChatMessage(message)) = messages.first_mut() {
            crate::privacy::records::merge_records(message, records);
        }
        Ok((false, messages))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "design_system".to_string(),
            display_name: "Design System".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Extract repository design tokens as DTCG JSON, inventory UI components, and find hardcoded visual values with nearest-token suggestions. Usage counts and drift findings are approximations, not verified references: without CodeGraph, usage is an identifier grep (`approx_usage_count`, `method: identifier-grep`) and every drift row is a regex scan carrying `heuristic: true`. Output is bounded; check `tokens_truncated`, `components_truncated`, and `findings_truncated` with their counts before treating a list as complete.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "description": "Optional workspace-relative or absolute path. Defaults to the workspace."},
                    "include_components": {"type": "boolean", "default": true},
                    "include_drift": {"type": "boolean", "default": true},
                    "max_findings": {"type": "integer", "minimum": 1, "maximum": MAX_FINDINGS, "default": DEFAULT_MAX_FINDINGS}
                }
            }),
            output_schema: None,
            annotations: None,
        }
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

fn resolve_scope(workspace_root: &Path, requested: Option<&str>) -> Result<PathBuf, String> {
    let candidate = match requested.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("workspace") => workspace_root.to_path_buf(),
        Some(value) => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                workspace_root.join(path)
            }
        }
    };
    let candidate = std::fs::canonicalize(&candidate)
        .map_err(|error| format!("failed to resolve scope `{}`: {error}", candidate.display()))?;
    if !candidate.starts_with(workspace_root) {
        return Err(format!(
            "scope `{}` is outside workspace `{}`",
            candidate.display(),
            workspace_root.display()
        ));
    }
    Ok(candidate)
}

fn excluded_entry(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }
    !matches!(
        entry.file_name().to_string_lossy().as_ref(),
        ".git"
            | ".refact"
            | ".cache"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | "coverage"
            | "vendor"
            | "__pycache__"
    )
}

fn is_ui_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("css")
            | Some("scss")
            | Some("less")
            | Some("tsx")
            | Some("jsx")
            | Some("ts")
            | Some("js")
            | Some("vue")
            | Some("svelte")
    )
}

fn token_source_priority(path: &Path) -> Option<u8> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if name == "tokens.css" || normalized.ends_with("/styles/tokens.css") {
        return Some(0);
    }
    if matches!(
        name.as_str(),
        "tailwind.config.js" | "tailwind.config.ts" | "tailwind.config.cjs" | "tailwind.config.mjs"
    ) {
        return Some(1);
    }
    let theme_name = name.starts_with("theme.") || name.contains(".theme.") || name == "theme.css";
    if theme_name
        && matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("css")
                | Some("json")
                | Some("yaml")
                | Some("yml")
                | Some("js")
                | Some("ts")
                | Some("cjs")
                | Some("mjs")
        )
    {
        return Some(2);
    }
    None
}

fn is_scan_file(path: &Path) -> bool {
    is_ui_source(path) || token_source_priority(path).is_some()
}

fn collect_scan_paths(scope: &Path) -> Vec<PathBuf> {
    if scope.is_file() {
        return is_scan_file(scope)
            .then(|| scope.to_path_buf())
            .into_iter()
            .collect();
    }
    let mut paths = WalkDir::new(scope)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(excluded_entry)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_scan_file(entry.path()))
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| {
        token_source_priority(left)
            .unwrap_or(3)
            .cmp(&token_source_priority(right).unwrap_or(3))
            .then_with(|| left.cmp(right))
    });
    paths
}

fn load_source_files(workspace_root: &Path, paths: Vec<PathBuf>) -> (Vec<SourceFile>, u64, bool) {
    let mut files = Vec::new();
    let mut scanned_bytes = 0u64;
    let mut truncated = false;
    for path in paths {
        if files.len() >= MAX_SCAN_FILES {
            truncated = true;
            break;
        }
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.len() > MAX_FILE_BYTES || scanned_bytes + metadata.len() > MAX_SCAN_BYTES {
            truncated = true;
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        scanned_bytes += metadata.len();
        files.push(SourceFile {
            relative: relative_path(workspace_root, &path),
            absolute: path,
            text,
        });
    }
    (files, scanned_bytes, truncated)
}

async fn load_graph_facts(
    service: Option<&Arc<refact_codegraph::CodeGraphService>>,
) -> Option<GraphFacts> {
    let service = service?;
    let cached = service.cached_graph_analytics().await.ok()?;
    let records = service.graph_node_records().await.ok()?;
    let mut incoming = HashMap::new();
    for (_source, destination, kind) in &cached.data.edges {
        if kind != "defined_in" {
            *incoming.entry(*destination).or_default() += 1;
        }
    }
    Some(GraphFacts {
        generation: cached.generation,
        records,
        incoming,
    })
}

fn analyze_design_system(
    workspace_root: &Path,
    scope: &Path,
    files: &[SourceFile],
    graph: Option<&GraphFacts>,
    include_components: bool,
    include_drift: bool,
    max_findings: usize,
    scanned_bytes: u64,
    scan_truncated: bool,
) -> DesignSystemResponse {
    let tokens = extract_tokens(files);
    let token_sources = tokens
        .values()
        .map(|token| token.source.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut token_categories = BTreeMap::new();
    for token in tokens.values() {
        *token_categories.entry(token.category.clone()).or_default() += 1;
    }
    let detected_prefixes = detect_prefixes(tokens.values());
    let output_tokens = tokens
        .iter()
        .take(MAX_TOKEN_OUTPUT)
        .map(|(name, token)| (name.clone(), token.clone()))
        .collect::<BTreeMap<_, _>>();
    let dtcg = build_dtcg(&output_tokens);
    let (component_inventory_source, component_count, components_truncated, components) =
        if include_components {
            inventory_components(workspace_root, scope, files, graph)
        } else {
            ("disabled".to_string(), 0, false, Vec::new())
        };
    let (drift_count, findings_truncated, drift) = if include_drift {
        detect_drift(files, &tokens, max_findings)
    } else {
        (0, false, Vec::new())
    };
    DesignSystemResponse {
        detected: !tokens.is_empty() || component_count > 0,
        scope: relative_path(workspace_root, scope),
        looked_for: vec![
            "tokens.css and styles/tokens.css custom properties".to_string(),
            "Tailwind theme configuration".to_string(),
            "theme CSS, JSON, YAML, JavaScript, and TypeScript configuration".to_string(),
            "exported TSX, JSX, Vue, and Svelte components".to_string(),
        ],
        scanned_files: files.len(),
        scanned_bytes,
        scan_truncated,
        token_sources,
        detected_prefixes,
        token_count: tokens.len(),
        token_output_count: output_tokens.len(),
        tokens_truncated: tokens.len() > output_tokens.len(),
        token_categories,
        tokens: dtcg,
        component_inventory_source,
        component_count,
        component_output_count: components.len(),
        components_truncated,
        components,
        drift_count,
        drift_output_count: drift.len(),
        findings_truncated,
        drift,
    }
}

fn design_system_output_filter() -> OutputFilter {
    OutputFilter {
        limit_lines: usize::MAX,
        limit_chars: MAX_OUTPUT_CHARS,
        limit_tokens: Some(MAX_OUTPUT_TOKENS),
        ..Default::default()
    }
}

fn extract_tokens(files: &[SourceFile]) -> BTreeMap<String, TokenRecord> {
    let mut tokens = BTreeMap::new();
    for file in files {
        let Some(priority) = token_source_priority(&file.absolute) else {
            continue;
        };
        if file
            .absolute
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("css")
        {
            parse_css_tokens(&file.text, &file.relative, priority, &mut tokens);
        } else {
            parse_config_tokens(&file.text, &file.relative, priority, &mut tokens);
        }
    }
    tokens
}

fn parse_css_tokens(
    text: &str,
    source: &str,
    priority: u8,
    tokens: &mut BTreeMap<String, TokenRecord>,
) {
    let comment = Regex::new(r"(?s)/\*.*?\*/").unwrap();
    let text = comment.replace_all(text, "");
    parse_css_segment(&text, source, priority, &[], tokens);
}

fn parse_css_segment(
    text: &str,
    source: &str,
    priority: u8,
    inherited: &[String],
    tokens: &mut BTreeMap<String, TokenRecord>,
) {
    let bytes = text.as_bytes();
    let mut cursor = 0usize;
    while let Some(open_offset) = text[cursor..].find('{') {
        let open = cursor + open_offset;
        let selector = text[cursor..open].trim();
        let Some(close) = matching_brace(bytes, open) else {
            break;
        };
        let body = &text[open + 1..close];
        let contexts = selector_contexts(selector, inherited);
        let direct = direct_block_text(body);
        parse_css_declarations(&direct, source, priority, &contexts, tokens);
        parse_css_segment(body, source, priority, &contexts, tokens);
        cursor = close + 1;
    }
}

fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        if escaped {
            escaped = false;
            continue;
        }
        if *byte == b'\\' {
            escaped = true;
            continue;
        }
        if matches!(*byte, b'\'' | b'"') {
            if quote == Some(*byte) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(*byte);
            }
            continue;
        }
        if quote.is_some() {
            continue;
        }
        if *byte == b'{' {
            depth += 1;
        } else if *byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn direct_block_text(body: &str) -> String {
    let mut output = String::with_capacity(body.len());
    let mut depth = 0usize;
    for ch in body.chars() {
        if ch == '{' {
            depth += 1;
            output.push(' ');
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
            output.push(' ');
        } else if depth == 0 {
            output.push(ch);
        } else if ch == '\n' {
            output.push('\n');
        } else {
            output.push(' ');
        }
    }
    output
}

fn selector_contexts(selector: &str, inherited: &[String]) -> Vec<String> {
    let lower = selector.to_ascii_lowercase();
    if lower.starts_with("@supports") {
        return inherited.to_vec();
    }
    let mut contexts = BTreeSet::new();
    if lower.contains(":root") {
        contexts.insert("default".to_string());
    }
    if lower.contains(".dark") || lower.contains("appearance=\"dark\"") {
        contexts.insert("dark".to_string());
    }
    if lower.contains(".light") || lower.contains("appearance=\"light\"") {
        contexts.insert("light".to_string());
    }
    if lower.contains("data-host=\"jetbrains\"") {
        contexts.insert("host-jetbrains".to_string());
    }
    if lower.starts_with("@media") {
        let name = if lower.contains("prefers-reduced-transparency") {
            "prefers-reduced-transparency"
        } else if lower.contains("prefers-reduced-motion") {
            "prefers-reduced-motion"
        } else {
            "media"
        };
        contexts.insert(name.to_string());
    }
    if contexts.is_empty() {
        contexts.extend(inherited.iter().cloned());
    } else if !inherited.is_empty() {
        contexts = contexts
            .into_iter()
            .map(|context| format!("{}+{}", inherited.join("+"), context))
            .collect();
    }
    if contexts.is_empty() {
        contexts.insert("default".to_string());
    }
    contexts.into_iter().collect()
}

fn parse_css_declarations(
    text: &str,
    source: &str,
    priority: u8,
    variants: &[String],
    tokens: &mut BTreeMap<String, TokenRecord>,
) {
    let declaration = Regex::new(r"(?s)(--[A-Za-z0-9_-]+)\s*:\s*([^;{}]+);").unwrap();
    for captures in declaration.captures_iter(text) {
        let name = captures[1].to_string();
        let value = captures[2].split_whitespace().collect::<Vec<_>>().join(" ");
        insert_token(tokens, name, value, variants, source, priority);
    }
}

fn parse_config_tokens(
    text: &str,
    source: &str,
    priority: u8,
    tokens: &mut BTreeMap<String, TokenRecord>,
) {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        if parse_config_value(&value, source, priority, tokens) {
            return;
        }
    }
    if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(text) {
        if let Ok(value) = serde_json::to_value(value) {
            if parse_config_value(&value, source, priority, tokens) {
                return;
            }
        }
    }
    for (config_key, category) in config_categories() {
        let pattern = Regex::new(&format!(
            r#"(?m)[\"']?{}[\"']?\s*:\s*\{{"#,
            regex::escape(config_key)
        ))
        .unwrap();
        let Some(found) = pattern.find(text) else {
            continue;
        };
        let open = found.end() - 1;
        let Some(close) = matching_brace(text.as_bytes(), open) else {
            continue;
        };
        let body = &text[open + 1..close];
        let entry = Regex::new(
            r#"(?m)[\"']?([A-Za-z0-9_-]+)[\"']?\s*:\s*(?:[\"'`]([^\"'`\n]+)[\"'`]|(-?[0-9]+(?:\.[0-9]+)?))"#,
        )
        .unwrap();
        for captures in entry.captures_iter(body) {
            let value = captures
                .get(2)
                .or_else(|| captures.get(3))
                .map(|capture| capture.as_str())
                .unwrap_or_default();
            insert_token(
                tokens,
                format!("{}.{}", config_key, &captures[1]),
                value.to_string(),
                &["default".to_string()],
                source,
                priority,
            );
        }
        if category == "motion" && body.trim().is_empty() {
            continue;
        }
    }
}

fn parse_config_value(
    value: &Value,
    source: &str,
    priority: u8,
    tokens: &mut BTreeMap<String, TokenRecord>,
) -> bool {
    let before = tokens.len();
    for (config_key, category) in config_categories() {
        if let Some(group) = config_group(value, config_key) {
            flatten_config_value(group, config_key, category, source, priority, tokens);
        }
    }
    tokens.len() > before
}

fn config_group<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value
        .get("theme")
        .and_then(|theme| theme.get(key))
        .or_else(|| {
            value
                .get("theme")
                .and_then(|theme| theme.get("extend"))
                .and_then(|extend| extend.get(key))
        })
        .or_else(|| value.get(key))
}

fn config_categories() -> Vec<(&'static str, &'static str)> {
    vec![
        ("colors", "color"),
        ("spacing", "spacing"),
        ("borderRadius", "radius"),
        ("fontSize", "typography"),
        ("fontFamily", "typography"),
        ("boxShadow", "shadow"),
        ("zIndex", "z-index"),
        ("transitionDuration", "motion"),
        ("transitionTimingFunction", "motion"),
        ("animation", "motion"),
    ]
}

fn flatten_config_value(
    value: &Value,
    prefix: &str,
    category: &str,
    source: &str,
    priority: u8,
    tokens: &mut BTreeMap<String, TokenRecord>,
) {
    match value {
        Value::Object(entries) => {
            for (key, value) in entries {
                flatten_config_value(
                    value,
                    &format!("{prefix}.{key}"),
                    category,
                    source,
                    priority,
                    tokens,
                );
            }
        }
        Value::String(value) => {
            insert_config_token(tokens, prefix, value, category, source, priority)
        }
        Value::Number(value) => insert_config_token(
            tokens,
            prefix,
            &value.to_string(),
            category,
            source,
            priority,
        ),
        _ => {}
    }
}

fn insert_config_token(
    tokens: &mut BTreeMap<String, TokenRecord>,
    name: &str,
    value: &str,
    category: &str,
    source: &str,
    priority: u8,
) {
    let entry = tokens
        .entry(name.to_string())
        .or_insert_with(|| TokenRecord {
            name: name.to_string(),
            category: category.to_string(),
            source: source.to_string(),
            priority,
            variants: BTreeMap::new(),
        });
    if priority < entry.priority {
        entry.source = source.to_string();
        entry.priority = priority;
        entry.category = category.to_string();
        entry.variants.clear();
    }
    if priority == entry.priority && entry.source == source {
        entry
            .variants
            .insert("default".to_string(), value.to_string());
    }
}

fn insert_token(
    tokens: &mut BTreeMap<String, TokenRecord>,
    name: String,
    value: String,
    variants: &[String],
    source: &str,
    priority: u8,
) {
    let category = token_category(&name, &value).to_string();
    let entry = tokens.entry(name.clone()).or_insert_with(|| TokenRecord {
        name,
        category: category.clone(),
        source: source.to_string(),
        priority,
        variants: BTreeMap::new(),
    });
    if priority < entry.priority {
        entry.source = source.to_string();
        entry.priority = priority;
        entry.category = category;
        entry.variants.clear();
    }
    if priority == entry.priority && entry.source == source {
        for variant in variants {
            entry.variants.insert(variant.clone(), value.clone());
        }
    }
}

fn token_category(name: &str, value: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.contains("radius") {
        "radius"
    } else if lower.contains("font")
        || lower.contains("text")
        || lower.ends_with("-line")
        || lower.contains("line-height")
    {
        "typography"
    } else if lower.contains("shadow") || lower.contains("elev") {
        "shadow"
    } else if lower.contains("z-") || lower.contains("zindex") || lower.contains("z-index") {
        "z-index"
    } else if lower.contains("motion")
        || lower.contains("dur")
        || lower.contains("ease")
        || lower.contains("stagger")
        || lower.contains("transition")
        || lower.contains("animation")
        || value.contains("cubic-bezier")
    {
        "motion"
    } else if lower.contains("color")
        || lower.contains("bg")
        || lower.contains("surface")
        || lower.contains("border")
        || lower.contains("accent")
        || lower.contains("chart")
        || lower.contains("focus")
        || is_color_value(value)
    {
        "color"
    } else {
        "spacing"
    }
}

fn is_color_value(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with('#')
        || value.starts_with("rgb(")
        || value.starts_with("rgba(")
        || value.starts_with("hsl(")
        || value.starts_with("hsla(")
        || value == "transparent"
}

fn detect_prefixes<'a>(tokens: impl Iterator<Item = &'a TokenRecord>) -> Vec<String> {
    let mut counts = BTreeMap::new();
    for token in tokens {
        if let Some(rest) = token.name.strip_prefix("--") {
            if let Some(prefix) = rest.split('-').next().filter(|prefix| !prefix.is_empty()) {
                *counts.entry(format!("--{prefix}-")).or_insert(0usize) += 1;
            }
        }
    }
    let mut prefixes = counts.into_iter().collect::<Vec<_>>();
    prefixes.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    prefixes
        .into_iter()
        .map(|(prefix, _)| prefix)
        .take(5)
        .collect()
}

fn build_dtcg(tokens: &BTreeMap<String, TokenRecord>) -> Value {
    let mut groups: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    for token in tokens.values() {
        let value = token
            .variants
            .get("default")
            .or_else(|| token.variants.get("dark"))
            .or_else(|| token.variants.get("light"))
            .or_else(|| token.variants.values().next())
            .cloned()
            .unwrap_or_default();
        let value = dtcg_alias(&value, tokens);
        let key = token.name.trim_start_matches("--").to_string();
        groups.entry(token.category.clone()).or_default().insert(
            key,
            json!({
                "$type": dtcg_type(token),
                "$value": value,
                "$extensions": {
                    "org.refact.source": {"name": token.name, "path": token.source},
                    "org.refact.variants": token.variants
                }
            }),
        );
    }
    serde_json::to_value(groups).unwrap_or_else(|_| Value::Object(Map::new()))
}

fn dtcg_alias(value: &str, tokens: &BTreeMap<String, TokenRecord>) -> String {
    let reference = Regex::new(r"^var\((--[A-Za-z0-9_-]+)\)$").unwrap();
    let Some(captures) = reference.captures(value.trim()) else {
        return value.to_string();
    };
    let name = &captures[1];
    let Some(token) = tokens.get(name) else {
        return value.to_string();
    };
    format!(
        "{{{}.{}}}",
        token.category,
        token.name.trim_start_matches("--")
    )
}

fn dtcg_type(token: &TokenRecord) -> &'static str {
    match token.category.as_str() {
        "color" => "color",
        "spacing" | "radius" => "dimension",
        "shadow" => "shadow",
        "z-index" => "number",
        "typography" if token.name.to_ascii_lowercase().contains("font") => "fontFamily",
        "typography" if token.name.to_ascii_lowercase().contains("line") => "number",
        "typography" => "dimension",
        "motion" if token.name.to_ascii_lowercase().contains("ease") => "cubicBezier",
        "motion"
            if token.name.to_ascii_lowercase().contains("dur")
                || token.name.to_ascii_lowercase().contains("stagger") =>
        {
            "duration"
        }
        _ => "number",
    }
}

fn inventory_components(
    workspace_root: &Path,
    scope: &Path,
    files: &[SourceFile],
    graph: Option<&GraphFacts>,
) -> (String, usize, bool, Vec<ComponentRecord>) {
    let component_files = files
        .iter()
        .filter(|file| is_component_file(&file.absolute) && !is_test_file(&file.absolute))
        .map(|file| (normalize_path(&file.relative), file))
        .collect::<HashMap<_, _>>();
    let identifiers = count_identifiers(files);
    let mut components = BTreeMap::<(String, String), ComponentRecord>::new();
    let mut graph_used = false;
    if let Some(graph) = graph {
        for (id, kind, name, path, _) in &graph.records {
            if !matches!(kind.as_str(), "function" | "struct") || !is_component_name(name) {
                continue;
            }
            let Some((relative, file)) =
                resolve_graph_file(workspace_root, scope, path, &component_files)
            else {
                continue;
            };
            let (props, variants) = component_contract(name, &file.text);
            let usage = graph.incoming.get(id).copied().unwrap_or_default();
            components.insert(
                (name.clone(), relative.clone()),
                ComponentRecord {
                    name: name.clone(),
                    path: relative,
                    exported: is_exported(name, &file.text),
                    props,
                    variants,
                    approx_usage_count: usage,
                    usage_count: usage,
                    method: "codegraph-incoming-edges",
                    heuristic: false,
                    source: format!("codegraph-generation-{}", graph.generation),
                },
            );
            graph_used = true;
        }
    }
    for file in component_files.values() {
        for name in component_names(&file.absolute, &file.text) {
            let key = (name.clone(), file.relative.clone());
            components.entry(key).or_insert_with(|| {
                let (props, variants) = component_contract(&name, &file.text);
                let usage = identifiers
                    .get(&name)
                    .copied()
                    .unwrap_or(1)
                    .saturating_sub(1);
                ComponentRecord {
                    approx_usage_count: usage,
                    usage_count: usage,
                    method: "identifier-grep",
                    heuristic: true,
                    exported: is_exported(&name, &file.text),
                    props,
                    variants,
                    name,
                    path: file.relative.clone(),
                    source: "filesystem".to_string(),
                }
            });
        }
    }
    let total = components.len();
    let truncated = total > MAX_COMPONENTS;
    let mut components = components.into_values().collect::<Vec<_>>();
    components.sort_by(|left, right| {
        right
            .approx_usage_count
            .cmp(&left.approx_usage_count)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.path.cmp(&right.path))
    });
    components.truncate(MAX_COMPONENTS);
    (
        if graph_used {
            "codegraph+filesystem".to_string()
        } else {
            "filesystem".to_string()
        },
        total,
        truncated,
        components,
    )
}

fn is_component_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("tsx") | Some("jsx") | Some("vue") | Some("svelte")
    )
}

fn is_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.contains(".test.") || name.contains(".spec.") || name.contains(".stories.")
}

fn is_component_name(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn resolve_graph_file<'a>(
    workspace_root: &Path,
    scope: &Path,
    indexed: &str,
    files: &'a HashMap<String, &'a SourceFile>,
) -> Option<(String, &'a SourceFile)> {
    let indexed_path = PathBuf::from(indexed);
    let absolute = if indexed_path.is_absolute() {
        indexed_path
    } else {
        workspace_root.join(indexed_path)
    };
    if !absolute.starts_with(scope) {
        return None;
    }
    let relative = normalize_path(&relative_path(workspace_root, &absolute));
    files.get(&relative).map(|file| (relative, *file))
}

fn component_names(path: &Path, text: &str) -> Vec<String> {
    let declaration = Regex::new(
        r"(?m)\bexport\s+(?:default\s+)?(?:async\s+)?(?:function|class|const|let|var)\s+([A-Z][A-Za-z0-9_]*)",
    )
    .unwrap();
    let mut names = declaration
        .captures_iter(text)
        .map(|captures| captures[1].to_string())
        .collect::<BTreeSet<_>>();
    if text.contains("export default") {
        if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            if is_component_name(stem) {
                names.insert(stem.to_string());
            }
        }
    }
    names.into_iter().collect()
}

fn count_identifiers(files: &[SourceFile]) -> HashMap<String, usize> {
    let identifier = Regex::new(r"\b[A-Z][A-Za-z0-9_]*\b").unwrap();
    let mut counts = HashMap::new();
    for file in files {
        if !matches!(
            file.absolute
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("tsx") | Some("jsx") | Some("ts") | Some("js") | Some("vue") | Some("svelte")
        ) {
            continue;
        }
        for found in identifier.find_iter(&file.text) {
            *counts.entry(found.as_str().to_string()).or_default() += 1;
        }
    }
    counts
}

fn is_exported(name: &str, text: &str) -> bool {
    Regex::new(&format!(
        r"(?m)\bexport\s+(?:default\s+)?(?:async\s+)?(?:function|class|const|let|var)\s+{}\b",
        regex::escape(name)
    ))
    .unwrap()
    .is_match(text)
        || Regex::new(&format!(
            r"(?m)\bexport\s+default\s+{}\b",
            regex::escape(name)
        ))
        .unwrap()
        .is_match(text)
}

fn component_contract(name: &str, text: &str) -> (Vec<String>, BTreeMap<String, Vec<String>>) {
    let mut props = BTreeSet::new();
    let mut variants = BTreeMap::new();
    let props_block = Regex::new(&format!(
        r"(?s)(?:interface\s+{}Props(?:\s+extends[^{{]+)?|type\s+{}Props\s*=)\s*\{{([^}}]*)\}}",
        regex::escape(name),
        regex::escape(name)
    ))
    .unwrap();
    let property =
        Regex::new(r"(?m)(?:^|[;,])\s*([A-Za-z_$][A-Za-z0-9_$]*)\??\s*:\s*([^;\n]+)").unwrap();
    let literal = Regex::new(r#"[\"']([^\"']+)[\"']"#).unwrap();
    if let Some(captures) = props_block.captures(text) {
        for property_capture in property.captures_iter(&captures[1]) {
            let prop = property_capture[1].to_string();
            let values = literal
                .captures_iter(&property_capture[2])
                .map(|captures| captures[1].to_string())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if values.len() > 1 || prop == "variant" || prop == "size" || prop == "tone" {
                if !values.is_empty() {
                    variants.insert(prop.clone(), values);
                }
            }
            props.insert(prop);
        }
    }
    let destructured = Regex::new(&format!(
        r"(?s)(?:function\s+{}|{}\s*=\s*(?:\([^)]*\)|[^=]+)=>)\s*\(\s*\{{([^}}]*)\}}",
        regex::escape(name),
        regex::escape(name)
    ))
    .unwrap();
    if let Some(captures) = destructured.captures(text) {
        for part in captures[1].split(',') {
            let prop = part
                .trim()
                .split([':', '='])
                .next()
                .unwrap_or_default()
                .trim();
            if !prop.is_empty()
                && prop
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                props.insert(prop.to_string());
            }
        }
    }
    (props.into_iter().collect(), variants)
}

fn detect_drift(
    files: &[SourceFile],
    tokens: &BTreeMap<String, TokenRecord>,
    max_findings: usize,
) -> (usize, bool, Vec<DriftFinding>) {
    let token_sources = tokens
        .values()
        .map(|token| normalize_path(&token.source))
        .collect::<HashSet<_>>();
    let hex = Regex::new(r"#[0-9A-Fa-f]{3,8}\b").unwrap();
    let px = Regex::new(r"\b-?[0-9]+(?:\.[0-9]+)?px\b").unwrap();
    let z_index = Regex::new(r"(?:z-index\s*:\s*|zIndex\s*[:=]\s*)(-?[0-9]+)").unwrap();
    let mut findings = Vec::new();
    let mut total = 0usize;
    for file in files {
        if token_sources.contains(&normalize_path(&file.relative))
            || !is_styled_file(&file.absolute)
        {
            continue;
        }
        let css_file = matches!(
            file.absolute
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("css") | Some("scss") | Some("less")
        );
        for (line_index, line) in file.text.lines().enumerate() {
            if line.trim_start().starts_with("--") || (!css_file && !styled_line(line)) {
                continue;
            }
            for found in hex.find_iter(line) {
                total += 1;
                if findings.len() < max_findings {
                    let (nearest_token, nearest_value) = nearest_color(found.as_str(), tokens);
                    findings.push(DriftFinding {
                        kind: "color".to_string(),
                        value: found.as_str().to_string(),
                        path: file.relative.clone(),
                        line: line_index + 1,
                        nearest_token,
                        nearest_value,
                        heuristic: true,
                        method: "regex-scan-nearest-token",
                    });
                }
            }
            for found in px.find_iter(line) {
                total += 1;
                if findings.len() < max_findings {
                    let (nearest_token, nearest_value) =
                        nearest_number(found.as_str(), tokens, false);
                    findings.push(DriftFinding {
                        kind: "dimension".to_string(),
                        value: found.as_str().to_string(),
                        path: file.relative.clone(),
                        line: line_index + 1,
                        nearest_token,
                        nearest_value,
                        heuristic: true,
                        method: "regex-scan-nearest-token",
                    });
                }
            }
            for captures in z_index.captures_iter(line) {
                total += 1;
                if findings.len() < max_findings {
                    let (nearest_token, nearest_value) = nearest_number(&captures[1], tokens, true);
                    findings.push(DriftFinding {
                        kind: "z_index".to_string(),
                        value: captures[1].to_string(),
                        path: file.relative.clone(),
                        line: line_index + 1,
                        nearest_token,
                        nearest_value,
                        heuristic: true,
                        method: "regex-scan-nearest-token",
                    });
                }
            }
        }
    }
    (total, total > findings.len(), findings)
}

fn is_styled_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("css")
            | Some("scss")
            | Some("less")
            | Some("tsx")
            | Some("jsx")
            | Some("ts")
            | Some("js")
            | Some("vue")
            | Some("svelte")
    )
}

fn styled_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("style=")
        || lower.contains("styled.")
        || lower.contains("css`")
        || lower.contains("classname")
        || [
            "color",
            "background",
            "padding",
            "margin",
            "gap",
            "width",
            "height",
            "font-size",
            "fontsize",
            "border-radius",
            "borderradius",
            "z-index",
            "zindex",
        ]
        .iter()
        .any(|property| lower.contains(property))
}

fn nearest_color(
    requested: &str,
    tokens: &BTreeMap<String, TokenRecord>,
) -> (Option<String>, Option<String>) {
    let Some(requested) = parse_color(requested) else {
        return (None, None);
    };
    tokens
        .values()
        .filter(|token| token.category == "color")
        .flat_map(|token| {
            token
                .variants
                .values()
                .filter_map(move |value| parse_color(value).map(|color| (token, value, color)))
        })
        .min_by_key(|(_, _, color)| color_distance(requested, *color))
        .map(|(token, value, _)| (Some(token.name.clone()), Some(value.clone())))
        .unwrap_or((None, None))
}

fn parse_color(value: &str) -> Option<(i32, i32, i32)> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let expanded;
        let hex = if hex.len() == 3 || hex.len() == 4 {
            expanded = hex
                .chars()
                .take(3)
                .flat_map(|ch| [ch, ch])
                .collect::<String>();
            expanded.as_str()
        } else if hex.len() == 6 || hex.len() == 8 {
            &hex[..6]
        } else {
            return None;
        };
        return Some((
            i32::from_str_radix(&hex[0..2], 16).ok()?,
            i32::from_str_radix(&hex[2..4], 16).ok()?,
            i32::from_str_radix(&hex[4..6], 16).ok()?,
        ));
    }
    let rgb = Regex::new(r"rgba?\(\s*([0-9]+)\s*,\s*([0-9]+)\s*,\s*([0-9]+)").unwrap();
    let captures = rgb.captures(value)?;
    Some((
        captures[1].parse().ok()?,
        captures[2].parse().ok()?,
        captures[3].parse().ok()?,
    ))
}

fn color_distance(left: (i32, i32, i32), right: (i32, i32, i32)) -> i32 {
    let red = left.0 - right.0;
    let green = left.1 - right.1;
    let blue = left.2 - right.2;
    red * red + green * green + blue * blue
}

fn nearest_number(
    requested: &str,
    tokens: &BTreeMap<String, TokenRecord>,
    z_index: bool,
) -> (Option<String>, Option<String>) {
    let requested = requested.trim_end_matches("px").parse::<f64>().ok();
    let Some(requested) = requested else {
        return (None, None);
    };
    tokens
        .values()
        .filter(|token| {
            if z_index {
                token.category == "z-index"
            } else {
                matches!(token.category.as_str(), "spacing" | "radius" | "typography")
            }
        })
        .flat_map(|token| {
            token.variants.values().filter_map(move |value| {
                value
                    .trim()
                    .trim_end_matches("px")
                    .parse::<f64>()
                    .ok()
                    .map(|number| (token, value, (requested - number).abs()))
            })
        })
        .min_by(|left, right| left.2.total_cmp(&right.2))
        .map(|(token, value, _)| (Some(token.name.clone()), Some(value.clone())))
        .unwrap_or((None, None))
}

fn report_paths(workspace_root: &Path, report: &DesignSystemResponse) -> Vec<PathBuf> {
    report
        .token_sources
        .iter()
        .chain(report.components.iter().map(|component| &component.path))
        .chain(report.drift.iter().map(|finding| &finding.path))
        .map(|path| workspace_root.join(path))
        .filter(|path| path.exists())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &Path, relative: &str, text: &str) -> SourceFile {
        SourceFile {
            absolute: path.join(relative),
            relative: relative.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn css_custom_properties_preserve_light_and_dark_variants_in_dtcg() {
        let root = Path::new("/tmp/design-system-css");
        let files = vec![source(
            root,
            "styles/tokens.css",
            ":root, .dark { --brand-color: #112233; --space-sm: 8px; }\n.light { --brand-color: #fefefe; }",
        )];
        let tokens = extract_tokens(&files);
        assert_eq!(tokens["--brand-color"].variants["dark"], "#112233");
        assert_eq!(tokens["--brand-color"].variants["light"], "#fefefe");
        let dtcg = build_dtcg(&tokens);
        assert_eq!(dtcg["color"]["brand-color"]["$type"], "color");
        assert_eq!(
            dtcg["color"]["brand-color"]["$extensions"]["org.refact.variants"]["light"],
            "#fefefe"
        );
        assert_eq!(dtcg["spacing"]["space-sm"]["$value"], "8px");
    }

    #[test]
    fn tailwind_config_tokens_are_grouped_by_design_category() {
        let root = Path::new("/tmp/design-system-tailwind");
        let files = vec![source(
            root,
            "tailwind.config.ts",
            "export default { theme: { colors: { brand: '#123456' }, spacing: { card: '18px' }, zIndex: { modal: 700 } } }",
        )];
        let tokens = extract_tokens(&files);
        assert_eq!(tokens["colors.brand"].category, "color");
        assert_eq!(tokens["spacing.card"].category, "spacing");
        assert_eq!(tokens["zIndex.modal"].category, "z-index");
    }

    #[test]
    fn theme_yaml_tokens_are_extracted() {
        let root = Path::new("/tmp/design-system-theme-yaml");
        let files = vec![source(
            root,
            "theme.yaml",
            "colors:\n  brand: '#654321'\nspacing:\n  panel: 20px\n",
        )];
        let tokens = extract_tokens(&files);
        assert_eq!(tokens["colors.brand"].variants["default"], "#654321");
        assert_eq!(tokens["spacing.panel"].variants["default"], "20px");
    }

    #[test]
    fn drift_detection_ignores_tokenized_values_and_suggests_nearest_tokens() {
        let root = Path::new("/tmp/design-system-drift");
        let files = vec![
            source(
                root,
                "styles/tokens.css",
                ":root { --brand: #112233; --space-2: 8px; --z-modal: 700; }",
            ),
            source(
                root,
                "Button.module.css",
                ".ok { color: var(--brand); padding: var(--space-2); }\n.bad { color: #102234; padding: 9px; z-index: 710; }",
            ),
        ];
        let tokens = extract_tokens(&files);
        let (count, truncated, findings) = detect_drift(&files, &tokens, 10);
        assert_eq!(count, 3);
        assert!(!truncated);
        assert_eq!(findings[0].path, "Button.module.css");
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].nearest_token.as_deref(), Some("--brand"));
        assert_eq!(findings[1].nearest_token.as_deref(), Some("--space-2"));
        assert_eq!(findings[2].nearest_token.as_deref(), Some("--z-modal"));
    }

    #[test]
    fn component_inventory_extracts_props_variants_and_usage_counts_without_codegraph() {
        let root = Path::new("/tmp/design-system-components");
        let files = vec![
            source(
                root,
                "Button.tsx",
                "export interface ButtonProps { variant?: 'primary' | 'ghost'; disabled?: boolean }\nexport function Button({ variant, disabled }: ButtonProps) { return <button /> }",
            ),
            source(root, "Page.tsx", "export const Page = () => <Button />;"),
        ];
        let (inventory_source, count, _, components) =
            inventory_components(root, root, &files, None);
        assert_eq!(inventory_source, "filesystem");
        assert_eq!(count, 2);
        let button = components
            .iter()
            .find(|component| component.name == "Button")
            .unwrap();
        assert_eq!(button.props, vec!["disabled", "variant"]);
        assert_eq!(button.variants["variant"], vec!["ghost", "primary"]);
        assert_eq!(button.approx_usage_count, 1);
        assert_eq!(button.usage_count, button.approx_usage_count);
        assert_eq!(button.method, "identifier-grep");
        assert!(button.heuristic);
    }

    #[test]
    fn drift_findings_declare_themselves_as_heuristics() {
        let root = Path::new("/tmp/design-system-heuristic");
        let files = vec![
            source(root, "styles/tokens.css", ":root { --brand: #112233; }"),
            source(root, "Card.module.css", ".bad { color: #102234; }"),
        ];
        let tokens = extract_tokens(&files);
        let (_, _, findings) = detect_drift(&files, &tokens, 10);
        assert!(!findings.is_empty());
        for finding in &findings {
            assert!(finding.heuristic, "{finding:?}");
            assert_eq!(finding.method, "regex-scan-nearest-token");
        }
    }

    #[test]
    fn component_inventory_is_bounded_and_ranked_by_usage() {
        let root = Path::new("/tmp/design-system-bounded");
        let mut files = Vec::new();
        for index in 0..(MAX_COMPONENTS + 5) {
            files.push(source(
                root,
                Box::leak(format!("Comp{index}.tsx").into_boxed_str()),
                Box::leak(
                    format!("export function Comp{index}() {{ return null }}").into_boxed_str(),
                ),
            ));
        }
        files.push(source(
            root,
            "Page.tsx",
            "export const Page = () => <Comp0 />;",
        ));
        let (_, total, truncated, components) = inventory_components(root, root, &files, None);
        assert!(truncated, "{total} components should exceed the cap");
        assert_eq!(components.len(), MAX_COMPONENTS);
        assert_eq!(components.first().unwrap().name, "Comp0");
        assert_eq!(components.first().unwrap().approx_usage_count, 1);
        assert!(components
            .windows(2)
            .all(|pair| pair[0].approx_usage_count >= pair[1].approx_usage_count));
    }

    #[test]
    fn design_system_output_is_bounded_instead_of_unlimited() {
        let filter = design_system_output_filter();
        assert!(!filter.skip);
        assert_eq!(filter.limit_chars, MAX_OUTPUT_CHARS);
        assert_eq!(filter.limit_tokens, Some(MAX_OUTPUT_TOKENS));
        assert!(filter.limit_chars < usize::MAX);
    }

    #[test]
    fn empty_scope_reports_a_confident_empty_design_system() {
        let root = Path::new("/tmp/design-system-empty");
        let report = analyze_design_system(root, root, &[], None, true, true, 10, 0, false);
        assert!(!report.detected);
        assert_eq!(report.token_count, 0);
        assert_eq!(report.component_count, 0);
        assert_eq!(report.drift_count, 0);
        assert_eq!(report.looked_for.len(), 4);
    }

    #[test]
    fn repo_tokens_css_extracts_real_categorized_tokens() {
        let engine = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = engine.join("../gui/src/styles/tokens.css");
        let text = std::fs::read_to_string(&path).unwrap();
        let files = vec![SourceFile {
            absolute: path,
            relative: "refact-agent/gui/src/styles/tokens.css".to_string(),
            text,
        }];
        let tokens = extract_tokens(&files);
        assert!(tokens.len() > 80, "extracted {} tokens", tokens.len());
        assert_eq!(tokens["--rf-color-accent"].category, "color");
        assert_eq!(tokens["--rf-space-3"].category, "spacing");
        assert_eq!(tokens["--rf-radius-card"].category, "radius");
        assert_eq!(tokens["--rf-dur-fast"].category, "motion");
        assert!(tokens["--rf-bg"].variants.contains_key("dark"));
        assert!(tokens["--rf-bg"].variants.contains_key("light"));
        let dtcg = build_dtcg(&tokens);
        assert_eq!(dtcg["color"]["rf-bg"]["$type"], "color");
    }

    #[tokio::test]
    async fn component_inventory_uses_codegraph_usage_counts_when_available() {
        let root = tempfile::tempdir().unwrap();
        let component_path = root.path().join("Button.tsx");
        let page_path = root.path().join("Page.tsx");
        let component = "export interface ButtonProps { variant?: 'primary' | 'ghost' }\nexport function Button({ variant }: ButtonProps) { return variant }";
        let page = "export function Page() { return Button({ variant: 'primary' }) }";
        std::fs::write(&component_path, component).unwrap();
        std::fs::write(&page_path, page).unwrap();
        let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
        service
            .index_file(&component_path.to_string_lossy(), component, "typescript")
            .await
            .unwrap();
        service
            .index_file(&page_path.to_string_lossy(), page, "typescript")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();
        let graph = load_graph_facts(Some(&service)).await.unwrap();
        let files = vec![
            source(root.path(), "Button.tsx", component),
            source(root.path(), "Page.tsx", page),
        ];
        let (inventory_source, _, _, components) =
            inventory_components(root.path(), root.path(), &files, Some(&graph));
        let button = components
            .iter()
            .find(|component| component.name == "Button")
            .unwrap();
        assert_eq!(inventory_source, "codegraph+filesystem");
        assert_eq!(button.approx_usage_count, 1);
        assert_eq!(button.method, "codegraph-incoming-edges");
        assert!(!button.heuristic);
        assert!(button.source.starts_with("codegraph-generation-"));
    }
}
