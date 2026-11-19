use std::collections::{BTreeMap, BTreeSet};

use refact_tool_api::tool_desc::ToolDesc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PYTHON_TOOL_NAME: &str = "python";
pub const NATIVE_ONLY_TOOLS: &[&str] = &[
    "python",
    "ask_questions",
    "task_done",
    "handoff_to_mode",
    "set_goal",
    "update_goal",
    "get_goal",
    "validate_goal",
    "tasks_set",
    "tool_job",
    "mcp_tool_search",
    "mcp_call",
    "finish",
    "agent_finish",
];
const CONSTANTS: &[&str] = &[
    "PROJECT_ROOT",
    "CHAT_ID",
    "WORKSPACE_DIR",
    "ARTIFACTS_DIR",
    "RUNS_DIR",
    "RUN_ID",
    "RUN_DIR",
    "PARENT_ARTIFACTS_DIR",
];
const HELPERS: &[&str] = &[
    "help",
    "tools",
    "host_path",
    "virtual_path",
    "mcp_search",
    "mcp_call",
    "llm",
    "show",
    "engine",
    "read",
    "grep",
    "replace",
    "git",
    "run",
];
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogOptions {
    pub mcp_lazy: bool,
    pub max_chars: usize,
    /// Bind and describe the `llm()` helper.
    #[serde(default)]
    pub llm: bool,
    /// Engine functions reachable through `engine(name, **kwargs)`; empty leaves `engine` unbound.
    #[serde(default)]
    pub engine: Vec<EngineFunctionDoc>,
}

/// Plain documentation of one engine function; the engine owns the behaviour.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EngineFunctionDoc {
    /// `area.function`, e.g. `settings.get`.
    pub name: String,
    pub area: String,
    /// `read`, `redacted_read`, `change`, `risky` or `destructive`.
    pub class: String,
    pub summary: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogFunction {
    pub py_name: String,
    pub tool_name: String,
    pub summary: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub is_async: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PythonCatalog {
    pub functions: Vec<CatalogFunction>,
    pub helpers: Vec<CatalogFunction>,
    pub description: String,
    #[serde(default)]
    pub engine: Vec<EngineFunctionDoc>,
}

impl PythonCatalog {
    pub fn resolve(&self, py_name: &str) -> Option<&CatalogFunction> {
        self.functions
            .iter()
            .chain(&self.helpers)
            .find(|f| f.py_name == py_name)
    }

    /// Tier 2: the typed stub, full description and input schema of one function.
    pub fn help_text(&self, py_name: &str) -> Option<String> {
        if !self.engine.is_empty() {
            if let Some(text) = engine_help(&self.engine, py_name) {
                return Some(text);
            }
            if py_name.starts_with("engine.") {
                return None;
            }
        }
        if let Some(f) = self.functions.iter().find(|f| f.py_name == py_name) {
            let mut used = reserved_names();
            used.extend(self.functions.iter().map(|f| f.py_name.clone()));
            return Some(format!(
                "{}\n{}\n\nInput schema: {}",
                function_help(f, &used),
                f.description,
                serde_json::to_string(&canonical(&f.input_schema)).ok()?
            ));
        }
        let f = self.helpers.iter().find(|f| f.py_name == py_name)?;
        Some(format!(
            "{}\n\nInput schema:\n{}\n\nOutput schema:\n{}",
            f.description,
            serde_json::to_string_pretty(&canonical(&f.input_schema)).ok()?,
            serde_json::to_string_pretty(&canonical(
                f.output_schema.as_ref().unwrap_or(&Value::Null)
            ))
            .ok()?
        ))
    }

    /// `help()` without a name: every function with its one-sentence summary, at most `max_chars`.
    pub fn help_index(&self, max_chars: usize) -> String {
        let head = "help(\"name\") prints one function's full typed signature, parameter docs and data types.\n";
        let entries: Vec<_> = self
            .helpers
            .iter()
            .chain(&self.functions)
            .map(|f| format!("{}: {}\n", f.py_name, cut(&f.summary, 100)))
            .collect();
        let mut s = head.to_string();
        for (i, entry) in entries.iter().enumerate() {
            let rest = entries.len() - i - 1;
            let tail = if rest == 0 {
                String::new()
            } else {
                format!("… and {rest} more; tools() lists every name.\n")
            };
            let fits =
                s.chars().count() + entry.chars().count() + tail.chars().count() <= max_chars;
            if !fits {
                s.push_str(&format!(
                    "… and {} more; tools() lists every name.\n",
                    entries.len() - i
                ));
                break;
            }
            s.push_str(entry);
        }
        s
    }
}

fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        _ => v.clone(),
    }
}

fn identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        && !KEYWORDS.contains(&s)
}
fn sanitize(s: &str) -> String {
    let mut result: String = s
        .chars()
        .map(|c| {
            if c == '_' || c.is_ascii_alphanumeric() {
                c
            } else {
                '_'
            }
        })
        .collect();
    if result.is_empty() || result.starts_with(|c: char| c.is_ascii_digit()) {
        result.insert(0, '_');
    }
    if KEYWORDS.contains(&result.as_str()) {
        result.push('_');
    }
    result
}
fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}
fn camel(s: &str) -> String {
    let mut result = String::new();
    for part in s.split('_').filter(|s| !s.is_empty()) {
        let mut cs = part.chars();
        if let Some(c) = cs.next() {
            result.extend(c.to_uppercase());
            result.extend(cs);
        }
    }
    sanitize(&result)
}
fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
fn summary(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = s
        .char_indices()
        .find(|(i, c)| matches!(c, '.' | '!' | '?') && s[i + c.len_utf8()..].starts_with(' '))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(s.len());
    cut(&s[..end], 160)
}
fn quoted(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}
fn props(v: &Value) -> Vec<(&str, &Value)> {
    let mut p: Vec<_> = v
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|m| m.iter().map(|(k, v)| (k.as_str(), v)))
        .collect();
    p.sort_by_key(|(k, _)| *k);
    p
}
fn required(v: &Value, name: &str) -> bool {
    v.get("required")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(name)))
}
fn union(types: impl IntoIterator<Item = String>) -> String {
    let mut result = Vec::new();
    for t in types {
        if !result.contains(&t) {
            result.push(t);
        }
    }
    if result.is_empty() {
        "Any".into()
    } else {
        result.join(" | ")
    }
}
fn optional(t: String) -> String {
    if t.split(" | ").any(|s| s == "None") {
        t
    } else {
        format!("{t} | None")
    }
}

fn reserved_names() -> BTreeSet<String> {
    let mut used: BTreeSet<_> = CONSTANTS
        .iter()
        .chain(HELPERS)
        .chain(NATIVE_ONLY_TOOLS)
        .map(|s| s.to_string())
        .collect();
    used.extend(
        [
            "ToolResult",
            "LlmResult",
            "LlmUsage",
            "RunResult",
            "FileChunk",
            "ImageRef",
            "T",
            "Any",
            "TypedDict",
            "Generic",
            "Literal",
            "NotRequired",
            "Unpack",
            "asyncio",
            "os",
            "re",
            "json",
            "Path",
        ]
        .map(str::to_string),
    );
    used
}

pub fn build_catalog(tools: &[ToolDesc], options: &CatalogOptions) -> PythonCatalog {
    let mut sorted: Vec<_> = tools
        .iter()
        .filter(|t| !NATIVE_ONLY_TOOLS.contains(&t.name.as_str()))
        .collect();
    sorted.sort_by_cached_key(|t| {
        (
            t.name.clone(),
            serde_json::to_string(&canonical(&json!([
                t.description,
                t.input_schema,
                t.output_schema
            ])))
            .unwrap(),
        )
    });
    let mut used = reserved_names();
    let mut entries: Vec<_> = sorted
        .into_iter()
        .map(|t| {
            let f = CatalogFunction {
                py_name: allocate(&sanitize(&t.name), &mut used),
                tool_name: t.name.clone(),
                summary: summary(&t.description),
                description: t.description.clone(),
                input_schema: canonical(&t.input_schema),
                output_schema: t.output_schema.as_ref().map(canonical),
                is_async: true,
            };
            let line = function_line(&f, t);
            (f, line)
        })
        .collect();
    entries.sort_by(|a, b| a.0.py_name.cmp(&b.0.py_name));
    let (functions, lines): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
    let run = tools.iter().any(|t| t.name == RUN_TOOL);
    let mut helpers = helper_functions(options.mcp_lazy, options.llm, run);
    let mut engine = options.engine.clone();
    engine.sort_by(|a, b| a.name.cmp(&b.name));
    if !engine.is_empty() {
        helpers.push(engine_helper());
    }
    let engine_text = engine_description(&engine);
    let fits = |text: &String| text.chars().count() <= options.max_chars;
    let mut text = String::new();
    for detail in [
        Detail::Full,
        Detail::TopLevel,
        Detail::NoShape,
        Detail::NoParams,
    ] {
        text = render(
            &lines,
            options.mcp_lazy,
            options.llm,
            run,
            &engine_text,
            detail,
        );
        if fits(&text) {
            break;
        }
    }
    if !fits(&text) {
        let notice = "# Catalog omitted to fit budget; use help(name) and tools().\n";
        text = if notice.chars().count() <= options.max_chars {
            notice.into()
        } else {
            String::new()
        };
    }
    PythonCatalog {
        functions,
        helpers,
        description: text,
        engine,
    }
}

const SHAPE_MAX_CHARS: usize = 160;

/// Tier-1 parts of one function line, precomputed for every degradation level.
struct FunctionLine {
    name: String,
    params: String,
    full_shape: Option<String>,
    top_shape: Option<String>,
    comment: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Detail {
    Full,
    TopLevel,
    NoShape,
    NoParams,
}

fn function_line(f: &CatalogFunction, t: &ToolDesc) -> FunctionLine {
    let input = &t.input_schema;
    let names: Vec<&str> = input
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|m| m.keys().map(String::as_str))
        .collect();
    let listed: Vec<&str> = input
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut required_names: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| listed.contains(n))
        .collect();
    for name in listed {
        if !required_names.contains(&name) {
            required_names.push(name);
        }
    }
    let params = if names.iter().chain(&required_names).any(|n| !identifier(n)) {
        "**kwargs".to_string()
    } else {
        let mut params: Vec<String> = required_names.iter().map(|n| n.to_string()).collect();
        if names.iter().any(|n| !required_names.contains(n)) {
            params.push("…".into());
        }
        params.join(", ")
    };
    let (full_shape, top_shape) = match &t.output_schema {
        Some(schema) => {
            let shapes = Shapes {
                root: schema,
            };
            let full = shapes.data(schema, false);
            let top = shapes.data(schema, true);
            let full = if full.chars().count() <= SHAPE_MAX_CHARS {
                full
            } else {
                top.clone()
            };
            (Some(full), Some(top))
        }
        None => (None, None),
    };
    FunctionLine {
        name: f.py_name.clone(),
        params,
        full_shape,
        top_shape,
        comment: if f.py_name == f.tool_name {
            String::new()
        } else {
            format!("  # tool {}", quoted(&f.tool_name))
        },
    }
}

enum Kind<'a> {
    Object(&'a Value),
    Map,
    Array(Option<&'a Value>),
    Other,
}

struct Shapes<'a> {
    root: &'a Value,
}
impl<'a> Shapes<'a> {
    fn lookup(&self, reference: &str) -> Option<&'a Value> {
        if reference == "#" {
            return Some(self.root);
        }
        let path = reference.strip_prefix("#/")?;
        let mut node = self.root;
        for part in path.split('/') {
            node = node.get(part.replace("~1", "/").replace("~0", "~"))?;
        }
        Some(node)
    }
    /// Follows `$ref`, nullability and single-branch unions; returns the node and whether it may be None.
    fn resolve(&self, mut node: &'a Value) -> (Kind<'a>, bool) {
        let mut nullable = false;
        for _ in 0..32 {
            if node.get("nullable") == Some(&Value::Bool(true)) {
                nullable = true;
            }
            if let Some(r) = node.get("$ref").and_then(Value::as_str) {
                match self.lookup(r) {
                    Some(target) => {
                        node = target;
                        continue;
                    }
                    None => return (Kind::Other, nullable),
                }
            }
            if let Some(branches) = ["anyOf", "oneOf"]
                .iter()
                .find_map(|k| node.get(*k).and_then(Value::as_array))
            {
                let non_null: Vec<_> = branches.iter().filter(|b| !is_null_schema(b)).collect();
                nullable |= non_null.len() < branches.len();
                if non_null.len() == 1 {
                    node = non_null[0];
                    continue;
                }
                return (Kind::Other, nullable);
            }
            break;
        }
        if node
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|a| a.contains(&Value::Null))
        {
            nullable = true;
        }
        let types: Vec<&str> = match node.get("type") {
            Some(Value::String(t)) => vec![t.as_str()],
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if types.contains(&"null") {
            nullable = true;
        }
        let types: Vec<_> = types.into_iter().filter(|t| *t != "null").collect();
        let has_properties = node.get("properties").and_then(Value::as_object).is_some();
        let kind = match types.as_slice() {
            ["array"] => Kind::Array(node.get("items")),
            ["object"] | [] if has_properties => Kind::Object(node),
            ["object"] => Kind::Map,
            [] if node
                .get("additionalProperties")
                .is_some_and(Value::is_object) =>
            {
                Kind::Map
            }
            _ => Kind::Other,
        };
        (kind, nullable)
    }
    fn keys(&self, object: &'a Value, level: usize, top: bool) -> Vec<String> {
        object
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(name, schema)| {
                let (kind, nullable) = self.resolve(schema);
                let mark = if nullable || !required(object, name) {
                    "?"
                } else {
                    ""
                };
                let suffix = if top {
                    String::new()
                } else {
                    self.suffix(kind, level)
                };
                let name = if identifier(name) {
                    name.clone()
                } else {
                    quoted(name)
                };
                format!("{name}{mark}{suffix}")
            })
            .collect()
    }
    fn nested(&self, object: &'a Value, level: usize) -> String {
        if level < 2 {
            format!("{{{}}}", self.keys(object, level + 1, false).join(", "))
        } else {
            "{…}".into()
        }
    }
    fn suffix(&self, kind: Kind<'a>, level: usize) -> String {
        match kind {
            Kind::Object(o) => self.nested(o, level),
            Kind::Map => "{*}".into(),
            Kind::Array(items) => match items.map(|i| self.resolve(i).0) {
                Some(Kind::Object(o)) => format!("[{}]", self.nested(o, level)),
                Some(Kind::Map) => "[{*}]".into(),
                _ => "[]".into(),
            },
            Kind::Other => String::new(),
        }
    }
    fn data(&self, schema: &'a Value, top: bool) -> String {
        let (open, close, keys) = match self.resolve(schema).0 {
            Kind::Object(o) => ("{", "}", self.keys(o, 1, top)),
            Kind::Array(items) => match items.map(|i| self.resolve(i).0) {
                Some(Kind::Object(o)) => ("[{", "}]", self.keys(o, 1, top)),
                Some(Kind::Map) => return "[{*}]".into(),
                _ => return "[]".into(),
            },
            Kind::Map => return "{*}".into(),
            Kind::Other => return "Any".into(),
        };
        let all = format!("{open}{}{close}", keys.join(", "));
        if !top || all.chars().count() <= SHAPE_MAX_CHARS {
            return all;
        }
        let mut kept = Vec::new();
        for key in keys {
            kept.push(key);
            let candidate = format!("{open}{}, …{close}", kept.join(", "));
            if candidate.chars().count() > SHAPE_MAX_CHARS {
                kept.pop();
                break;
            }
        }
        if kept.is_empty() {
            format!("{open}…{close}")
        } else {
            format!("{open}{}, …{close}", kept.join(", "))
        }
    }
}
fn is_null_schema(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some("null")
        || v.get("enum").and_then(Value::as_array) == Some(&vec![Value::Null])
}

pub const SHOW_HELPER: &str = "show";
const SHOW_STUB: &str = "def show(value: Any, *, kind: Literal[\"table\", \"markdown\", \"bar\", \"line\", \"area\", \"scatter\", \"pie\"] | None = None, title: str | None = None, x: str | None = None, y: str | list[str] | None = None, columns: list[str] | None = None) -> None: ...  # show a table, chart or markdown to the user; you only get a one-line note back\n";
const SHOW_HELP: &str = "show(value, *, kind=None, title=None, x=None, y=None, columns=None) displays a table, a chart or markdown to the user in the chat. Nothing of it enters your context: you get back only a one-line note such as `shown to the user: table \"Slowest tests\" (12000 rows × 4 columns)`. `value` may be positional; everything else is keyword-only. Returns None.\n\nkind: \"table\", \"markdown\", \"bar\", \"line\", \"area\", \"scatter\" or \"pie\". Without kind a str is markdown and anything else is a table.\nvalue for tables and charts: a list of dicts (columns are the keys in first-seen order, missing keys are empty), a list of lists or tuples (columns col1..colN, or `columns`), a dict of lists (one column per key), a dict of scalars (a key/value table), or a list of scalars (one `value` column). `columns` selects and orders columns; a name that is not a column raises TypeError. Cells are str, int, float, bool or None; NaN becomes empty and other values become text.\nCharts: x defaults to the first column, y to every other numeric column (pie: the first one); y may be a name or a list of names.\nCaps: up to 10000 rows and 200 columns are saved in the run folder (`RUN_DIR/displays/`), cells up to 10000 characters; the chat shows up to 200 table rows, 500 chart rows, 30 columns and 20000 markdown characters, and up to 12 displays per run. Aggregate large data first.\nErrors: an unknown kind, a wrong value type, a missing x or y column, a y column holding anything but numbers and None or more than 200 columns raise TypeError.";

fn show_helper() -> CatalogFunction {
    CatalogFunction {
        py_name: SHOW_HELPER.into(),
        tool_name: SHOW_HELPER.into(),
        summary: "Show a table, chart or markdown to the user.".into(),
        description: SHOW_HELP.into(),
        input_schema: json!({"type":"object","additionalProperties":false,"required":["value"],"properties":{"value":{},"kind":{"enum":["table","markdown","bar","line","area","scatter","pie",null]},"title":{"type":["string","null"]},"x":{"type":["string","null"]},"y":{"anyOf":[{"type":"string"},{"type":"array","items":{"type":"string"}},{"type":"null"}]},"columns":{"type":["array","null"],"items":{"type":"string"}}}}),
        output_schema: None,
        is_async: false,
    }
}

/// Sync helpers the worker runs on its own file system access, next to `open()`.
pub const FILE_HELPERS: &[&str] = &["read", "grep", "replace"];
const READ_HELP: &str = "read(path, start=1, end=None, *, numbered=True) returns lines start..end (1-based, inclusive) of a UTF-8 text file. `path` is a str or Path; a relative path starts at PROJECT_ROOT, whatever os.chdir() set. A negative start returns the last lines: read(p, -40) is the last 40 lines (all lines of a shorter file), and end must be None then. An end past the last line is clamped; start=0, an end before start and a start past the last line raise ValueError that states the line count. An empty file gives \"\".\nnumbered=True prefixes every line with its number, `  12 | text`, without its line ending. numbered=False returns the exact text of those lines with their line endings: read(p, numbered=False) is the whole file, and its slices are valid `old` values for replace().\nErrors as for open(): FileNotFoundError, IsADirectoryError, PermissionError for protected paths, and UnicodeDecodeError for a file that is not UTF-8 (use open(path, \"rb\")). A result over the limit a script can receive raises RuntimeError; read a smaller range. The read is listed in files_read with its privacy record, like open(). It reads the file on disk, not unsaved editor buffers.";
const GREP_HELP: &str = "grep(pattern, path=\".\", *, context=0, glob=None, ignore_case=False, limit=200, data=False) searches a file or a directory for a regular expression in Rust regex syntax, the syntax of search_pattern (no look-around or back-references; the `re` module uses Python syntax instead). Each line is matched without its line ending; a UTF-8 byte order mark is ignored. A relative path starts at PROJECT_ROOT, whatever os.chdir() set.\nA directory is searched with search_pattern's file list: ignore rules apply and files the privacy policy blocks are never read. glob filters that list, relative to path with / separators: a pattern without / matches file names, ** crosses directories. Files over 4 MiB, binary files and files that are not UTF-8 are skipped and counted, as are protected and privacy-blocked files. The search stops early after 20,000 listed entries or 256 MiB read and says so.\nThe text result has one `path:line: text` line per match, `path-line- text` context lines and `--` between groups; lines are cut at 400 characters and the text at 200,000. Final lines say how many matches the limit left out, why the search stopped early and what was skipped, or `no matches (searched N files)`. data=True returns {matches: [{path, line, text}], total, truncated, files_searched, files_skipped} without context lines; truncated is true when more matches exist than were returned or the search stopped early, and total then counts only the files searched.\ncontext must be 0..20 and limit 1..10000; these, an invalid pattern or glob, and a glob with a file path raise ValueError. A missing or protected path raises the error open() raises. Privacy records: in a directory only files with matches are recorded, as for search_pattern, and a named file always is; a result that depends on files without matches (for example no matches) is accepted.";
const REPLACE_HELP: &str = "replace(path, old, new, *, count=1) replaces exact text in a UTF-8 text file and returns a unified diff of the change, cut at 60 lines (the run card shows all of it). `old` must occur exactly `count` times, counted left to right without overlaps (\"aa\" occurs once in \"aaa\"); count=0 replaces every occurrence and needs at least one. Otherwise nothing is written and ValueError names the real number of occurrences. old must not be empty or equal to new; files over 32 MiB and results over 32 MiB are refused.\nThe new text goes to a temporary file in the same directory with the same mode, which then replaces the file; when the file changed on disk meanwhile, nothing is written and RuntimeError says so. A symlinked path replaces the real file and keeps the link (in a read-only chat a symlinked path is refused); a file with more than one hard link is refused. Line endings and every byte outside the replaced text stay as they were; read(path, numbered=False) shows the exact text to match.\nA relative path starts at PROJECT_ROOT, whatever os.chdir() set. Protected paths raise PermissionError; so does every path outside ARTIFACTS_DIR in a read-only chat (one without file-editing, rm, mv, shell or process_start tools), before the file is read. The edit is listed in files_written with its diff, like an open() write. It works on the file on disk, not on unsaved editor buffers, and undo_textdoc does not undo it.";

fn file_helpers() -> Vec<CatalogFunction> {
    [
        (
            "read",
            "Read lines of a text file with line numbers.",
            READ_HELP,
            json!({"type":"object","additionalProperties":false,"required":["path"],"properties":{"path":{"type":"string"},"start":{"type":"integer"},"end":{"type":["integer","null"]},"numbered":{"type":"boolean"}}}),
            json!({"type":"string"}),
        ),
        (
            "grep",
            "Search files for a regex.",
            GREP_HELP,
            json!({"type":"object","additionalProperties":false,"required":["pattern"],"properties":{"pattern":{"type":"string"},"path":{"type":"string"},"context":{"type":"integer","minimum":0,"maximum":20},"glob":{"type":["string","null"]},"ignore_case":{"type":"boolean"},"limit":{"type":"integer","minimum":1,"maximum":10000},"data":{"type":"boolean"}}}),
            json!({"anyOf":[{"type":"string"},{"type":"object","required":["matches","total","truncated","files_searched","files_skipped"],"properties":{"matches":{"type":"array","items":{"type":"object","required":["path","line","text"],"properties":{"path":{"type":"string"},"line":{"type":"integer"},"text":{"type":"string"}}}},"total":{"type":"integer"},"truncated":{"type":"boolean"},"files_searched":{"type":"integer"},"files_skipped":{"type":"integer"}}}]}),
        ),
        (
            "replace",
            "Replace exact text in a file and return the diff.",
            REPLACE_HELP,
            json!({"type":"object","additionalProperties":false,"required":["path","old","new"],"properties":{"path":{"type":"string"},"old":{"type":"string","minLength":1},"new":{"type":"string"},"count":{"type":"integer","minimum":0}}}),
            json!({"type":"string"}),
        ),
    ]
    .into_iter()
    .map(|(name, summary, help, input, output)| CatalogFunction {
        py_name: name.into(),
        tool_name: name.into(),
        summary: summary.into(),
        description: help.into(),
        input_schema: input,
        output_schema: Some(output),
        is_async: false,
    })
    .collect()
}

pub const LLM_HELPER: &str = "llm";
pub const LLM_POSITIONAL: &[&str] = &["prompt"];
const LLM_STUB: &str = "async def llm(prompt: str, *, system: str | None = None, schema: dict[str, Any] | None = None, model: Literal[\"light\", \"default\", \"thinking\", \"chat\"] | None = None, max_tokens: int | None = None) -> LlmResult: ...  # one model question without tools or chat history; cheap model by default; budgeted per run. LlmResult = {ok: bool, text: str, data: Any | None, model: str, usage: {input_tokens: int, output_tokens: int}, error: str | None}\n";
const LLM_HELP: &str = "llm(prompt, *, system=None, schema=None, model=None, max_tokens=None) asks a model one question from inside the script. The call has no tools, no chat history and no chat system prompt: the messages are the optional `system` text and one user message with `prompt`. `prompt` may be positional; every other argument is keyword-only.\n\nmodel: None uses the cheap `light` model when the data this run has read may be sent to its provider, otherwise this chat's model. \"light\", \"default\" and \"thinking\" name configured model slots (an unconfigured slot means this chat's model); \"chat\" is this chat's model. An explicit model is strict.\nschema: a JSON Schema dict (at most 16 KB). The model is asked for exactly one JSON value matching it; the answer is parsed and validated, retried once on mismatch, and returned in `data`.\n\nReturns {ok, text, data, model, usage: {input_tokens, output_tokens}, error}. max_tokens: output tokens for the answer; 8192 when omitted, capped at the model's output limit.\n\nProvider errors, timeouts (180 s for the whole call, retry included) and schema mismatches return ok=False with `error`. `prompt` plus `system` may have at most 400000 characters. Bad arguments raise TypeError, an exhausted per-run call or token budget raises RuntimeError (the token budget counts finished calls only, so calls already running in parallel can overshoot it), and data that the privacy policy does not allow to reach the model's provider raises PermissionError before anything is sent. Use asyncio.gather to run many calls in parallel.";

fn llm_helper() -> CatalogFunction {
    CatalogFunction {
        py_name: LLM_HELPER.into(),
        tool_name: LLM_HELPER.into(),
        summary: "Ask a model one question without tools or chat history.".into(),
        description: LLM_HELP.into(),
        input_schema: json!({"type":"object","additionalProperties":false,"required":["prompt"],"properties":{"prompt":{"type":"string","minLength":1},"system":{"type":["string","null"]},"schema":{"type":["object","null"]},"model":{"enum":["light","default","thinking","chat",null]},"max_tokens":{"type":["integer","null"],"minimum":1}}}),
        output_schema: Some(
            json!({"type":"object","required":["ok","text","data","model","usage","error"],"properties":{"ok":{"type":"boolean"},"text":{"type":"string"},"data":{},"model":{"type":"string"},"usage":{"type":"object","required":["input_tokens","output_tokens"],"properties":{"input_tokens":{"type":"integer"},"output_tokens":{"type":"integer"}}},"error":{"type":["string","null"]}}}),
        ),
        is_async: true,
    }
}

pub const GIT_HELPER: &str = "git";
/// Keyword names the kernel gives the leading positional arguments of `git(...)`.
pub const GIT_POSITIONAL: &[&str] = &["subcommand", "#1", "#2"];
const GIT_STUB: &str = "async def git(subcommand: Literal[\"status\", \"diff\", \"log\", \"show\", \"branches\"], *args, **kwargs) -> ToolResult[dict]: ...  # read-only git data, no shell needed: git(\"status\"), git(\"diff\", rev=None, path=None, staged=False), git(\"log\", n=20, rev=None, path=None), git(\"show\", rev, path=None), git(\"branches\")\n";
const GIT_HELP: &str = "git(subcommand, ...) returns git data of the repository that holds PROJECT_ROOT, limited to the chat's workspace folders. It only reads: it runs no git program, fetches nothing, never refreshes or writes the index and works in read-only chats. Returns ToolResult: `data` as below, `text` a one-line summary.\n\ngit(\"status\"): {root, branch, head_oid, head_detached, unborn, upstream, ahead, behind, repo_state, changes: [{path, old_path?, status, staged, unstaged}], untracked: [path], conflicted: [{path, ours, theirs, ancestor}], truncated, withheld_count}.\ngit(\"diff\", rev=None, path=None, staged=False): what `git diff` shows: index to worktree; staged=True: HEAD (or rev) to index; rev=\"X\": X to worktree; \"A..B\": A to B; \"A...B\": the merge base of A and B to B. path is a str or a list of str. {root, base, files: [{path, old_path?, status, link, binary, additions?, deletions?, patch, patch_truncated}], truncated, withheld_count}. Untracked files and submodules are left out.\ngit(\"log\", n=20, rev=None, path=None): up to n (1-200) commits, newest first; rev is \"X\" or \"A..B\"; with path, only commits that changed a file under it that git() may show. {root, rev, commits: [{hash, short, author, email, date, subject, message, parents, refs, files: [{path, status}], files_truncated, withheld_count}], scan_truncated, shallow, withheld_count}.\ngit(\"show\", rev, path=None): with path, that file at rev: {root, rev, path, exists, link, binary, size, text, truncated}; without, the commit with its patches: {root, rev, commit, files, truncated, withheld_count}. rev and path may be positional here only.\ngit(\"branches\"): {root, current, head_detached, head_oid, branches: [{name, is_head, upstream, tip_oid, ahead, behind, upstream_gone}], remote_branches: [{name, remote, short_name, tip_oid}], tags: [{name, target_oid}], remotes, truncated}.\n\nrev names a commit: a hash, a branch, a tag or a form like HEAD~2. Blob (X:path), index, reflog (@{...}) and peel (^{...}) forms and values that start with '-' raise TypeError, on each side of a range too. Paths are literal and relative to the repository root.\nLimits: 300 files per diff; a file over 1 MiB is not loaded (no patch, patch_truncated); renames are detected only among at most 200 added and deleted files, none of them over 1 MiB; patches are cut at 64 KiB per file and 512 KiB per call (patch_truncated, truncated); a shown file over 2 MiB is not loaded; 100 files per logged commit; log reads at most 50,000 commits (scan_truncated); branches lists at most 1,000 branches, 1,000 remote branches and 1,000 tags, the first by name (truncated). A call over a very large change set can take long; Stop is honoured between files.\nLinks: a symbolic link stored in the repository is listed with link=True and has no patch and no text; its target is never returned.\nPrivacy: a file the privacy policy or the engine's rules keep from scripts, or whose name is not valid UTF-8 or holds a backslash, is left out of every list, with both sides of a rename, and only counted in withheld_count; naming it as path in diff, log or show raises PermissionError, whether or not it exists. A path is checked as written and as the file it resolves to now through links; both must pass. Files whose content is returned carry privacy records into this run, as reads do. Commit messages, authors and refs are returned as they are.\nErrors: bad arguments raise TypeError; a hook or a mode rule that refuses `git` raises PermissionError; an exhausted call budget raises RuntimeError; a missing repository or revision returns ok=False with `error`, which says so when the repository is shallow; a repository that borrows objects from another one (objects/info/alternates) is not read: ok=False.";

fn git_helper() -> CatalogFunction {
    CatalogFunction {
        py_name: GIT_HELPER.into(),
        tool_name: GIT_HELPER.into(),
        summary: "Read git status, diffs, history, files at a revision and branches.".into(),
        description: GIT_HELP.into(),
        input_schema: json!({"type":"object","additionalProperties":false,"required":["subcommand"],"properties":{"subcommand":{"enum":["status","diff","log","show","branches"]},"rev":{"type":["string","null"]},"path":{"anyOf":[{"type":"string"},{"type":"array","items":{"type":"string"}},{"type":"null"}]},"staged":{"type":"boolean"},"n":{"type":"integer","minimum":1,"maximum":200}}}),
        output_schema: None,
        is_async: true,
    }
}

/// The tool `run()` calls; it is bound only in chats that have this tool.
pub const RUN_TOOL: &str = "shell";
pub const RUN_HELPER: &str = "run";
pub const RUN_POSITIONAL: &[&str] = &["cmd"];
const RUN_STUB: &str = "async def run(cmd: str, *, timeout: int | None = None, cwd: str | None = None, tail: int = 60) -> RunResult: ...  # runs a build or test command to its end through the shell tool (same approvals and rules). RunResult = {ok: bool, exit_code: int | None, status: str, duration_s: float, tail: str, problems: list[str], log: str | None, truncated: bool, error: str | None}\n";
const RUN_HELP: &str = "run(cmd, *, timeout=None, cwd=None, tail=60) runs one shell command to its end through the shell tool, with the same approvals, hooks and rules as calling shell, and returns {ok, exit_code, status, duration_s, tail, problems, log, truncated, error}. It waits as long as the command takes (or `timeout` seconds, 1-3600), so use it for builds and tests instead of starting a process and polling it. `cmd` may be positional; cwd is the shell tool's workdir.\nstatus is \"exited\", \"timed_out\", \"killed\", \"failed\" or \"withheld\"; ok is True only for exit code 0. tail: the last `tail` lines (0-2000), each cut at 1000 characters, at most 64 KiB. problems: up to 20 distinct error lines in output order (compiler errors, failed tests, panics, tracebacks, FAIL and npm ERR! lines), each cut at 300 characters; warnings are left out. log: the path of a file under RUN_DIR/logs with the output the shell kept, stdout and stderr in arrival order; truncated is True when that is not everything the command printed. Read it with read(log, -200) or grep(pattern, log).\nWhen the privacy policy withholds the output, status is \"withheld\", tail and problems are empty, log is None and error says so. A hook that turns the command into a background one makes run() raise RuntimeError. A refusal (allowed tools, a rule, a hook or the user) raises PermissionError, as for a nested shell call. A command that waits for input waits until timeout or Stop; pass a timeout.";

fn run_helper() -> CatalogFunction {
    CatalogFunction {
        py_name: RUN_HELPER.into(),
        tool_name: RUN_HELPER.into(),
        summary:
            "Run a build or test command to its end; exit code, last lines, problems, log path."
                .into(),
        description: RUN_HELP.into(),
        input_schema: json!({"type":"object","additionalProperties":false,"required":["cmd"],"properties":{"cmd":{"type":"string","minLength":1},"timeout":{"type":["integer","null"],"minimum":1,"maximum":3600},"cwd":{"type":["string","null"]},"tail":{"type":"integer","minimum":0,"maximum":2000}}}),
        output_schema: Some(
            json!({"type":"object","required":["ok","exit_code","status","duration_s","tail","problems","log","truncated","error"],"properties":{"ok":{"type":"boolean"},"exit_code":{"type":["integer","null"]},"status":{"enum":["exited","timed_out","killed","failed","withheld"]},"duration_s":{"type":"number"},"tail":{"type":"string"},"problems":{"type":"array","items":{"type":"string"}},"log":{"type":["string","null"]},"truncated":{"type":"boolean"},"error":{"type":["string","null"]}}}),
        ),
        is_async: true,
    }
}

pub const ENGINE_HELPER: &str = "engine";
const ENGINE_STUB: &str = "async def engine(name: str, **kwargs) -> ToolResult[Any]: ...  # read and change the engine itself; every change asks the user first\n";
const ENGINE_HELP: &str = "engine(name, **kwargs) calls one engine function: it reads or changes the engine itself (settings, this chat, models, tools, indexes, agents, usage and more). `name` is \"area.function\" and may be positional; arguments are keywords. Returns ToolResult: `data` is the function's output, `text` a short summary.\nReads run freely. Every change asks the user first and is listed on the run card.\nAn unknown name or bad arguments raise TypeError; a refusal (switched off, not allowed in this chat, a hook, a rule or the user) raises PermissionError; an exhausted call budget raises RuntimeError; a function that fails returns ok=False.";

fn engine_helper() -> CatalogFunction {
    CatalogFunction {
        py_name: ENGINE_HELPER.into(),
        tool_name: ENGINE_HELPER.into(),
        summary: "Read and change the engine itself: settings, this chat, models, tools and more."
            .into(),
        description: ENGINE_HELP.into(),
        input_schema: json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"}},"additionalProperties":true}),
        output_schema: None,
        is_async: true,
    }
}

fn engine_areas(docs: &[EngineFunctionDoc]) -> BTreeMap<&str, Vec<&EngineFunctionDoc>> {
    let mut areas: BTreeMap<&str, Vec<&EngineFunctionDoc>> = BTreeMap::new();
    for doc in docs {
        areas.entry(doc.area.as_str()).or_default().push(doc);
    }
    areas
}

fn engine_description(docs: &[EngineFunctionDoc]) -> String {
    if docs.is_empty() {
        return String::new();
    }
    let areas: Vec<&str> = engine_areas(docs).into_keys().collect();
    format!(
        "{ENGINE_STUB}# engine areas: {}; help(\"engine\") lists their functions\n",
        areas.join(", ")
    )
}

fn short_name<'a>(doc: &'a EngineFunctionDoc) -> &'a str {
    doc.name
        .strip_prefix(&format!("{}.", doc.area))
        .unwrap_or(&doc.name)
}

fn class_note(class: &str) -> &'static str {
    match class {
        "read" => "runs freely",
        "redacted_read" => "runs freely; secret-looking values are masked",
        "change" => "asks the user before it runs",
        "risky" => "weakens a safety boundary; asks the user before it runs",
        "destructive" => "cannot be undone; asks the user before it runs",
        _ => "",
    }
}

fn engine_help(docs: &[EngineFunctionDoc], name: &str) -> Option<String> {
    let areas = engine_areas(docs);
    if name == ENGINE_HELPER {
        let mut s = format!("{ENGINE_HELP}\n\nAreas:\n");
        for (area, functions) in &areas {
            let names: Vec<_> = functions.iter().map(|d| short_name(d)).collect();
            s.push_str(&format!("{area}: {}\n", names.join(", ")));
        }
        s.push_str("help(\"engine.<area>\") lists an area's functions; help(\"engine.<area>.<function>\") gives arguments, output and class.");
        return Some(s);
    }
    let rest = name.strip_prefix("engine.")?;
    if let Some(functions) = areas.get(rest) {
        let mut s = format!("engine area {rest}:\n");
        for doc in functions {
            s.push_str(&format!(
                "{} [{}]: {}\n",
                doc.name,
                doc.class,
                cut(&doc.summary, 160)
            ));
        }
        s.push_str("Classes: read and redacted_read run freely; change, risky and destructive ask the user first.");
        return Some(s);
    }
    let doc = docs.iter().find(|d| d.name == rest)?;
    let mut types = Types::new(&camel(&doc.name.replace('.', "_")));
    types.input = true;
    let schema = &doc.input_schema;
    let mut params = props(schema);
    params.sort_by(|(a, _), (b, _)| required(schema, b).cmp(&required(schema, a)).then(a.cmp(b)));
    let mut call = vec![quoted(&doc.name)];
    let mut lines = String::new();
    for (key, s) in &params {
        let t = types.typ(s, &camel(key), false);
        if required(schema, key) {
            call.push(format!("{key}=..."));
            lines.push_str(&format!("  {key}: {t}"));
        } else {
            call.push(format!("{key}=None"));
            lines.push_str(&format!("  {key}: {}", optional(t)));
        }
        if let Some(d) = s.get("description").and_then(Value::as_str) {
            lines.push_str(&format!(
                "  # {}",
                d.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
        }
        lines.push('\n');
    }
    let mut s = format!(
        "await engine({})\n{}\nClass: {} ({})\n",
        call.join(", "),
        doc.summary,
        doc.class,
        class_note(&doc.class)
    );
    if !lines.is_empty() {
        s.push_str(&format!("Arguments:\n{lines}"));
    }
    s.push_str(&format!(
        "Input schema: {}\n",
        serde_json::to_string(&canonical(schema)).ok()?
    ));
    if let Some(output) = &doc.output_schema {
        s.push_str(&format!(
            "Output (data): {}\n",
            serde_json::to_string(&canonical(output)).ok()?
        ));
    }
    Some(s)
}

fn helper_functions(lazy: bool, llm: bool, run: bool) -> Vec<CatalogFunction> {
    let mut entries = vec![
        (
            "help",
            "Return the full description and schemas for a function.",
            json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}),
            false,
        ),
        (
            "tools",
            "List available Python function names.",
            json!({"type":"object","properties":{}}),
            false,
        ),
        (
            "host_path",
            "Translate a virtual path to a host path.",
            json!({"type":"object","properties":{"p":{"type":"string"}},"required":["p"]}),
            false,
        ),
        (
            "virtual_path",
            "Translate a host path to a virtual path.",
            json!({"type":"object","properties":{"p":{"type":"string"}},"required":["p"]}),
            false,
        ),
    ];
    if lazy {
        entries.push((
            "mcp_search",
            "Discover MCP tools on demand.",
            json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
            true,
        ));
        entries.push(("mcp_call","Call a discovered MCP tool.",json!({"type":"object","properties":{"tool_name":{"type":"string"},"args":{"type":"object","additionalProperties":{}}},"required":["tool_name","args"]}),true));
    }
    let mut helpers: Vec<_> = entries
        .into_iter()
        .map(|(name, desc, input, is_async)| CatalogFunction {
            py_name: name.into(),
            tool_name: name.into(),
            summary: desc.into(),
            description: desc.into(),
            input_schema: input,
            output_schema: None,
            is_async,
        })
        .collect();
    helpers.push(show_helper());
    helpers.extend(file_helpers());
    helpers.push(git_helper());
    if run {
        helpers.push(run_helper());
    }
    if llm {
        helpers.push(llm_helper());
    }
    helpers
}

struct Types {
    prefix: String,
    defs: BTreeMap<String, String>,
    used: BTreeSet<String>,
    declarations: Vec<String>,
    input: bool,
    input_defs: BTreeMap<String, Value>,
}
impl Types {
    fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.into(),
            defs: BTreeMap::new(),
            used: BTreeSet::new(),
            declarations: Vec::new(),
            input: false,
            input_defs: BTreeMap::new(),
        }
    }
    fn definitions(&mut self, schema: &Value) {
        self.defs.clear();
        if let Some(defs) = schema.get("$defs").and_then(Value::as_object) {
            let sorted: BTreeMap<_, _> = defs.iter().collect();
            for k in sorted.keys() {
                let name = allocate(&format!("{}_{}", self.prefix, sanitize(k)), &mut self.used);
                self.defs.insert(
                    format!("#/$defs/{}", k.replace('~', "~0").replace('/', "~1")),
                    name,
                );
            }
            for (k, v) in sorted {
                let key = format!("#/$defs/{}", k.replace('~', "~0").replace('/', "~1"));
                let name = self.defs[&key].clone();
                let typ = self.typ(v, &name, true);
                if typ != name {
                    self.declarations.push(format!("{name} = {typ}\n"));
                }
            }
        }
    }
    fn typ(&mut self, s: &Value, hint: &str, allocated: bool) -> String {
        if s.get("nullable") == Some(&Value::Bool(true)) {
            let mut node = s.clone();
            node.as_object_mut().unwrap().remove("nullable");
            return optional(self.typ(&node, hint, allocated));
        }
        if ["allOf", "not", "if", "then", "else", "prefixItems"]
            .iter()
            .any(|k| s.get(k).is_some())
        {
            return "Any".into();
        }
        if let Some(r) = s.get("$ref").and_then(Value::as_str) {
            if self.input {
                let Some(def) = self.input_defs.remove(r) else {
                    return "Any".into();
                };
                // Recursive input references need no expanded declarations.
                if def.get("type").and_then(Value::as_str) == Some("object")
                    || def.get("properties").is_some()
                {
                    self.input_defs.insert(r.to_owned(), def);
                    return "dict[str, Any]".into();
                }
                let mut inline = def.clone();
                if let Some(node) = inline.as_object_mut() {
                    node.remove("$ref");
                }
                let typ = self.typ(&inline, hint, false);
                self.input_defs.insert(r.to_owned(), def);
                return typ;
            }
            return self.defs.get(r).cloned().unwrap_or_else(|| "Any".into());
        }
        for key in ["anyOf", "oneOf"] {
            if let Some(a) = s.get(key).and_then(Value::as_array) {
                return union(
                    a.iter()
                        .enumerate()
                        .map(|(i, v)| self.typ(v, &format!("{hint}Variant{}", i + 1), false)),
                );
            }
        }
        if let Some(a) = s.get("enum").and_then(Value::as_array) {
            if !a.is_empty()
                && a.iter().all(|v| {
                    v.is_string() || v.is_i64() || v.is_u64() || v.is_boolean() || v.is_null()
                })
            {
                let values: Vec<_> = a
                    .iter()
                    .map(|v| match v {
                        Value::Bool(true) => "True".into(),
                        Value::Bool(false) => "False".into(),
                        Value::Null => "None".into(),
                        _ => v.to_string(),
                    })
                    .collect();
                return format!("Literal[{}]", values.join(", "));
            }
        }
        if let Some(a) = s.get("type").and_then(Value::as_array) {
            return union(a.iter().map(|t| {
                let mut node = s.clone();
                node["type"] = t.clone();
                self.typ(&node, hint, false)
            }));
        }
        let t = match s.get("type").and_then(Value::as_str) {
            Some("string") => "str".into(),
            Some("integer") => "int".into(),
            Some("number") => "float".into(),
            Some("boolean") => "bool".into(),
            Some("null") => "None".into(),
            Some("array") => format!(
                "list[{}]",
                self.typ(
                    s.get("items").unwrap_or(&Value::Null),
                    &format!("{hint}Item"),
                    false
                )
            ),
            Some("object") | None
                if s.get("properties").is_some() || s.get("type") == Some(&json!("object")) =>
            {
                if self.input {
                    "dict[str, Any]".into()
                } else if let Some(ap) = s.get("additionalProperties").filter(|v| v.is_object()) {
                    format!(
                        "dict[str, {}]",
                        self.typ(ap, &format!("{hint}Value"), false)
                    )
                } else if s.get("properties").and_then(Value::as_object).is_some() {
                    let name = if allocated {
                        hint.into()
                    } else {
                        allocate(hint, &mut self.used)
                    };
                    self.typed_dict(s, &name);
                    name
                } else {
                    "dict[str, Any]".into()
                }
            }
            _ => "Any".into(),
        };
        if s.get("nullable") == Some(&Value::Bool(true)) {
            optional(t)
        } else {
            t
        }
    }
    fn typed_dict(&mut self, s: &Value, name: &str) {
        let properties = props(s);
        let functional = properties.iter().any(|(k, _)| !identifier(k));
        let mut fields = Vec::new();
        for (key, v) in properties {
            let t = self.typ(v, &format!("{name}{}", camel(key)), false);
            let t = if required(s, key) {
                t
            } else {
                format!("NotRequired[{t}]")
            };
            fields.push(if functional {
                format!("    {}: {t},\n", quoted(key))
            } else {
                format!("    {key}: {t}\n")
            });
        }
        let declaration = if functional {
            format!(
                "{name} = TypedDict({}, {{\n{}}})\n",
                quoted(name),
                fields.join("")
            )
        } else {
            format!(
                "class {name}(TypedDict):\n{}",
                if fields.is_empty() {
                    "    pass\n".into()
                } else {
                    fields.join("")
                }
            )
        };
        self.declarations.push(declaration);
    }
    fn take(&mut self) -> String {
        std::mem::take(&mut self.declarations).join("\n")
    }
}
struct Block {
    input: String,
    output: String,
    signature: String,
    docs: String,
}
fn render_function(f: &CatalogFunction, prefix: &str, used: &mut BTreeSet<String>) -> Block {
    let mut types = Types::new(prefix);
    types.used = used.clone();
    types.input = true;
    if let Some(defs) = f.input_schema.get("$defs").and_then(Value::as_object) {
        types.input_defs = defs
            .iter()
            .map(|(key, value)| {
                (
                    format!("#/$defs/{}", key.replace('~', "~0").replace('/', "~1")),
                    value.clone(),
                )
            })
            .collect();
    }
    let properties = props(&f.input_schema);
    let mut docs = String::new();
    for (name, s) in &properties {
        if let Some(d) = s.get("description").and_then(Value::as_str) {
            docs.push_str(&format!(
                "# {}: {}\n",
                quoted(name),
                d.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
        }
    }
    let args = if properties.iter().any(|(k, _)| !identifier(k)) {
        let name = allocate(&format!("{prefix}Args"), &mut types.used);
        types.typed_dict(&f.input_schema, &name);
        format!("**kwargs: Unpack[{name}]")
    } else {
        let mut sorted = properties;
        sorted.sort_by(|(a, _), (b, _)| {
            required(&f.input_schema, b)
                .cmp(&required(&f.input_schema, a))
                .then(a.cmp(b))
        });
        let params: Vec<_> = sorted
            .into_iter()
            .map(|(name, s)| {
                let t = types.typ(s, &format!("{prefix}{}", camel(name)), false);
                if required(&f.input_schema, name) {
                    format!("{name}: {t}")
                } else {
                    format!("{name}: {} = None", optional(t))
                }
            })
            .collect();
        if params.is_empty() {
            String::new()
        } else {
            format!("*, {}", params.join(", "))
        }
    };
    let input = types.take();
    types.input = false;
    let output_type = if let Some(s) = &f.output_schema {
        types.definitions(s);
        types.typ(s, &format!("{prefix}Output"), false)
    } else {
        "None".into()
    };
    let output = types.take();
    *used = types.used;
    let signature = format!(
        "async def {}({args}) -> ToolResult[{output_type}]:\n",
        f.py_name
    );
    Block {
        input,
        output,
        signature,
        docs,
    }
}
fn function_help(f: &CatalogFunction, used: &BTreeSet<String>) -> String {
    let mut used = used.clone();
    let b = render_function(f, &camel(&f.py_name), &mut used);
    let mut s = String::new();
    for part in [&b.input, &b.output] {
        if !part.is_empty() {
            s.push_str(part);
            s.push('\n');
        }
    }
    s.push_str(&b.docs);
    s.push_str(&b.signature);
    s.push_str(&format!("    {}\n    ...\n", quoted(&f.summary)));
    s
}

const CONVENTIONS: &str = "# Await every tool call; arguments are keyword-only and omitted optional arguments default to None. Nested object arguments are plain dicts.\n";
const ENVELOPE: &str = "# ToolResult = {ok: bool, data: the tool's structured result or None, text: str, files: list[{path: str, line1: int, line2: int, content: str}], diffs: list[dict], images: list[{image: str, index: int}], images_dropped: int, error: str | None, truncated: bool}. Check ok and truncated.\n";
const CONSTANTS_TEXT: &str = "# Constants: PROJECT_ROOT, CHAT_ID, WORKSPACE_DIR, ARTIFACTS_DIR, RUNS_DIR, RUN_ID, RUN_DIR: str; PARENT_ARTIFACTS_DIR: str | None (None without a parent or when the parent has no Python workspace yet).\n";
const HELPERS_TEXT: &str = "def help(name: str | None = None) -> str: ...  # one function's full typed signature, parameter docs and data types; no name: the function index\ndef tools() -> list[str]: ...\ndef host_path(p: str) -> str: ...\ndef virtual_path(p: str) -> str: ...\ndef read(path: str, start: int = 1, end: int | None = None, *, numbered: bool = True) -> str: ...  # lines start..end of a text file (1-based, inclusive; start=-N: last N lines) as \"  12 | text\"; numbered=False: the exact text\ndef grep(pattern: str, path: str = \".\", *, context: int = 0, glob: str | None = None, ignore_case: bool = False, limit: int = 200, data: bool = False) -> str | dict: ...  # regex search in a file or directory (search_pattern syntax and ignore rules): \"path:line: text\" lines\ndef replace(path: str, old: str, new: str, *, count: int = 1) -> str: ...  # exact-text edit; ValueError unless exactly count matches (0: all, at least one); returns a unified diff\n";
const MCP_STUB: &str = "async def mcp_search(*, query: str) -> ToolResult[Any]: ...\nasync def mcp_call(*, tool_name: str, args: dict[str, Any]) -> ToolResult[Any]: ...\n";
const FUNCTIONS_HEADER: &str = "# Tool functions are async and take keyword arguments: the same arguments as the direct tool of the same name. Each returns ToolResult. name(a, b, …) lists the required arguments; … means optional ones exist. After -> come the keys of data: key? may be missing or None, key[] is a list, key[{…}] a list of dicts, key{…} a dict, {*} arbitrary keys. help(\"name\") prints the full typed signature and data types.\n";

fn line(l: &FunctionLine, detail: Detail) -> String {
    let params = if detail == Detail::NoParams && !l.params.is_empty() {
        "…"
    } else {
        l.params.as_str()
    };
    let shape = match detail {
        Detail::Full => l.full_shape.as_ref(),
        Detail::TopLevel => l.top_shape.as_ref(),
        Detail::NoShape | Detail::NoParams => None,
    };
    let shape = shape.map(|s| format!(" -> {s}")).unwrap_or_default();
    format!("{}({params}){shape}{}\n", l.name, l.comment)
}

fn render(
    lines: &[FunctionLine],
    lazy: bool,
    llm: bool,
    run: bool,
    engine: &str,
    detail: Detail,
) -> String {
    let mut s = String::new();
    s.push_str(CONVENTIONS);
    s.push_str(ENVELOPE);
    s.push_str(CONSTANTS_TEXT);
    s.push_str(HELPERS_TEXT);
    s.push_str(GIT_STUB);
    if run {
        s.push_str(RUN_STUB);
    }
    if llm {
        s.push_str(LLM_STUB);
    }
    s.push_str(SHOW_STUB);
    s.push_str(engine);
    if lazy {
        s.push_str(MCP_STUB);
    }
    s.push_str(&runtime_note());
    s.push('\n');
    if !lines.is_empty() {
        s.push_str(FUNCTIONS_HEADER);
        for l in lines {
            s.push_str(&line(l, detail));
        }
    }
    s
}

// Keep the description and executable engine import checks on the same inventory.
pub const AVAILABLE_MODULES: &[&str] = &[
    "annotationlib",
    "asyncio",
    "base64",
    "binascii",
    "bisect",
    "collections",
    "copy",
    "csv",
    "dataclasses",
    "datetime",
    "decimal",
    "fnmatch",
    "functools",
    "hashlib",
    "io",
    "itertools",
    "json",
    "math",
    "os",
    "os.path",
    "pathlib",
    "posixpath",
    "random",
    "re",
    "sys",
    "time",
    "tomllib",
    "typing",
    "unicodedata",
];
pub const MISSING_MODULES: &[&str] = &[
    "ast",
    "shutil",
    "socket",
    "subprocess",
    "threading",
    "urllib",
];
pub fn runtime_note() -> String {
    format!("# Monty restricted Python. Imports: {}. Missing: {}. os, re, json, Path and asyncio are imported before every run. io has StringIO, BytesIO and open only (no TextIOWrapper; newline=None does not translate). Path.relative_to missing; paths are not orderable (sort with key=str); with_suffix/rglob, os.walk and os.path.join/relpath/splitext available (partial CPython APIs). globals() is missing. Bound methods cannot be passed as values (max(d, key=d.get) fails; use key=lambda k: d[k]); calling d.get(k, 0) works. No third-party/network/subprocess. A script can import its own .py files from PROJECT_ROOT (no relative imports, no sys.path).", AVAILABLE_MODULES.join(","), MISSING_MODULES.join(","))
}
