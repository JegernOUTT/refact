//! `show()`: normalization, caps, display files and the per-run display log (contract C13).
use std::path::{Path, PathBuf};

use refact_chat_api::PythonDisplay;
use serde_json::{json, Map, Value};

pub const KINDS: &[&str] = &["table", "markdown", "bar", "line", "area", "scatter", "pie"];
pub const FILE_MAX_ROWS: usize = 10_000;
pub const FILE_MAX_COLUMNS: usize = 200;
pub const FILE_MAX_CELL_CHARS: usize = 10_000;
pub const INLINE_TABLE_ROWS: usize = 200;
pub const INLINE_CHART_ROWS: usize = 500;
pub const INLINE_MAX_COLUMNS: usize = 30;
pub const INLINE_CELL_CHARS: usize = 200;
pub const INLINE_MARKDOWN_CHARS: usize = 20_000;
pub const RUN_INLINE_DISPLAYS: usize = 12;
pub const RUN_INLINE_BYTES: usize = 512 * 1024;
pub const MODEL_LINES: usize = 12;
pub const TITLE_CHARS: usize = 120;
/// Per-run limits of `show()` calls and of display file bytes.
pub const RUN_MAX_SHOWS: u32 = 200;
pub const RUN_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const FORMS: &str = "show() accepts a list of dicts, a list of lists or tuples, a dict of lists (columns), a dict of scalars (key/value table), a list of scalars, or a str for markdown";

/// A display normalized within the file caps.
#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    pub kind: String,
    pub title: Option<String>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub rows_total: u64,
    pub x: Option<String>,
    pub y: Vec<String>,
    pub text: Option<String>,
    pub truncated: bool,
}

fn type_error<T>(message: impl Into<String>) -> Result<T, String> {
    Err(message.into())
}

fn chars_cut(text: &str, cap: usize) -> (String, bool) {
    if text.chars().count() <= cap {
        (text.to_owned(), false)
    } else {
        (
            text.chars()
                .take(cap.saturating_sub(1))
                .chain(['…'])
                .collect(),
            true,
        )
    }
}

fn is_repr(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    (object.len() == 1).then(|| object.get("__repr__")?.as_str())?
}

fn cell(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => value.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if !f.is_finite() => Value::Null,
            _ => value.clone(),
        },
        _ => match is_repr(value) {
            Some(repr) if matches!(repr, "nan" | "inf" | "-inf") => Value::Null,
            Some(repr) => Value::String(repr.to_owned()),
            None => Value::String(serde_json::to_string(value).unwrap_or_default()),
        },
    }
}

fn string_list(value: Option<&Value>, name: &str) -> Result<Option<Vec<String>>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if name == "y" => Ok(Some(vec![s.clone()])),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("show(): {name} must be a list of str"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) if name == "y" => type_error("show(): y must be a str or a list of str"),
        Some(_) => type_error(format!("show(): {name} must be a list of str")),
    }
}

fn optional_str(value: Option<&Value>, name: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => type_error(format!("show(): {name} must be a str")),
    }
}

fn select(
    columns: &[String],
    rows: Vec<Vec<Value>>,
    wanted: Option<Vec<String>>,
) -> Result<Table, String> {
    let Some(wanted) = wanted else {
        return Ok((columns.to_vec(), rows));
    };
    let picks = wanted
        .iter()
        .map(|name| known_column(columns, name))
        .collect::<Result<Vec<usize>, String>>()?;
    let rows = rows
        .into_iter()
        .map(|row| {
            picks
                .iter()
                .map(|&i| row.get(i).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    Ok((wanted, rows))
}

fn known_column(columns: &[String], name: &str) -> Result<usize, String> {
    columns.iter().position(|c| c == name).ok_or_else(|| {
        format!(
            "show(): column '{name}' does not exist; columns are {}",
            columns.join(", ")
        )
    })
}

type Table = (Vec<String>, Vec<Vec<Value>>);

fn is_scalar(value: &Value) -> bool {
    !value.is_array() && (!value.is_object() || is_repr(value).is_some())
}

fn tabular(value: &Value, columns_arg: Option<Vec<String>>) -> Result<Table, String> {
    match value {
        Value::Array(items) if items.is_empty() => Ok((columns_arg.unwrap_or_default(), vec![])),
        Value::Array(items) if items.iter().all(|v| v.is_object() && is_repr(v).is_none()) => {
            let mut keys: Vec<String> = vec![];
            for item in items {
                for key in item.as_object().unwrap().keys() {
                    if !keys.contains(key) {
                        keys.push(key.clone());
                    }
                }
            }
            let columns = match columns_arg {
                Some(wanted) => {
                    for name in &wanted {
                        known_column(&keys, name)?;
                    }
                    wanted
                }
                None => keys,
            };
            let rows = items
                .iter()
                .map(|item| {
                    let item = item.as_object().unwrap();
                    columns
                        .iter()
                        .map(|c| item.get(c).cloned().unwrap_or(Value::Null))
                        .collect()
                })
                .collect();
            Ok((columns, rows))
        }
        Value::Array(items) if items.iter().all(Value::is_array) => {
            let longest = items
                .iter()
                .map(|v| v.as_array().unwrap().len())
                .max()
                .unwrap_or(0);
            let columns =
                columns_arg.unwrap_or_else(|| (1..=longest).map(|i| format!("col{i}")).collect());
            let width = columns.len();
            let rows = items
                .iter()
                .map(|v| {
                    let mut row = v.as_array().unwrap().clone();
                    row.resize(width, Value::Null);
                    row
                })
                .collect();
            Ok((columns, rows))
        }
        Value::Array(items) if items.iter().all(is_scalar) => select(
            &["value".to_owned()],
            items.iter().map(|v| vec![v.clone()]).collect(),
            columns_arg,
        ),
        Value::Object(map)
            if is_repr(value).is_none() && !map.is_empty() && map.values().all(Value::is_array) =>
        {
            let columns: Vec<String> = map.keys().cloned().collect();
            let len = map
                .values()
                .map(|v| v.as_array().unwrap().len())
                .max()
                .unwrap_or(0);
            let rows = (0..len)
                .map(|i| {
                    map.values()
                        .map(|v| v.as_array().unwrap().get(i).cloned().unwrap_or(Value::Null))
                        .collect()
                })
                .collect();
            select(&columns, rows, columns_arg)
        }
        Value::Object(map) if is_repr(value).is_none() && !map.values().any(Value::is_array) => {
            select(
                &["key".to_owned(), "value".to_owned()],
                map.iter()
                    .map(|(k, v)| vec![Value::String(k.clone()), v.clone()])
                    .collect(),
                columns_arg,
            )
        }
        _ => type_error(FORMS),
    }
}

fn is_number(value: &Value) -> bool {
    value.is_number()
}

/// Normalizes the arguments of one `show()` call; `Err` is a TypeError message.
pub fn normalize(
    args: &Value,
    kwargs: &Value,
    max_markdown_bytes: usize,
) -> Result<Normalized, String> {
    let empty = Map::new();
    let kwargs = kwargs.as_object().unwrap_or(&empty);
    let positional: &[Value] = args.as_array().map(Vec::as_slice).unwrap_or(&[]);
    for key in kwargs.keys() {
        if !["value", "kind", "title", "x", "y", "columns"].contains(&key.as_str()) {
            return type_error(format!("show() got an unexpected keyword argument '{key}'"));
        }
    }
    if positional.len() > 1 {
        return type_error(
            "show() takes 1 positional argument; kind, title, x, y and columns are keyword-only",
        );
    }
    let value = match (positional.first(), kwargs.get("value")) {
        (Some(_), Some(_)) => return type_error("show() got multiple values for argument 'value'"),
        (Some(v), None) | (None, Some(v)) => v,
        (None, None) => return type_error("show() missing required argument: 'value'"),
    };
    let kind = match optional_str(kwargs.get("kind"), "kind")? {
        Some(kind) if KINDS.contains(&kind.as_str()) => kind,
        Some(kind) => {
            return type_error(format!(
                "show(): unknown kind '{kind}'; kinds are {}",
                KINDS.join(", ")
            ))
        }
        None if value.is_string() => "markdown".to_owned(),
        None => "table".to_owned(),
    };
    let title = optional_str(kwargs.get("title"), "title")?
        .filter(|t| !t.is_empty())
        .map(|t| chars_cut(&t, TITLE_CHARS).0);
    let columns_arg = string_list(kwargs.get("columns"), "columns")?;
    let x_arg = optional_str(kwargs.get("x"), "x")?;
    let y_arg = string_list(kwargs.get("y"), "y")?;
    if kind == "markdown" {
        let Some(text) = value.as_str() else {
            return type_error("show(kind='markdown') needs a str value");
        };
        let mut end = text.len().min(max_markdown_bytes);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        return Ok(Normalized {
            kind,
            title,
            columns: vec![],
            rows: vec![],
            rows_total: 0,
            x: None,
            y: vec![],
            text: Some(text[..end].to_owned()),
            truncated: end < text.len(),
        });
    }
    let (columns, rows) = tabular(value, columns_arg)?;
    if columns.len() > FILE_MAX_COLUMNS {
        return type_error(format!(
            "show(): {} columns is more than {FILE_MAX_COLUMNS}; select the columns to show with columns=[...]",
            columns.len()
        ));
    }
    let rows_total = rows.len() as u64;
    let mut truncated = rows.len() > FILE_MAX_ROWS;
    let rows: Vec<Vec<Value>> = rows
        .into_iter()
        .take(FILE_MAX_ROWS)
        .map(|row| {
            row.iter()
                .map(|v| match cell(v) {
                    Value::String(s) => {
                        let (s, cut) = chars_cut(&s, FILE_MAX_CELL_CHARS);
                        truncated |= cut;
                        Value::String(s)
                    }
                    other => other,
                })
                .collect()
        })
        .collect();
    let (x, y) = if kind == "table" {
        (None, vec![])
    } else {
        let all_numeric = |name: &str| -> bool {
            let i = columns.iter().position(|c| c == name).unwrap();
            let mut any = false;
            for row in &rows {
                match row.get(i) {
                    Some(Value::Null) | None => {}
                    Some(v) if is_number(v) => any = true,
                    Some(_) => return false,
                }
            }
            any
        };
        let x = match x_arg {
            Some(x) if columns.contains(&x) => x,
            Some(x) => {
                return type_error(format!(
                    "show(): x column '{x}' does not exist; columns are {}",
                    columns.join(", ")
                ))
            }
            None => match columns.first() {
                Some(first) => first.clone(),
                None => {
                    return type_error(format!("show(kind='{kind}') needs at least one column"))
                }
            },
        };
        let y = match y_arg {
            Some(y) if !y.is_empty() => {
                for name in &y {
                    if !columns.contains(name) {
                        return type_error(format!(
                            "show(): y column '{name}' does not exist; columns are {}",
                            columns.join(", ")
                        ));
                    }
                    if !all_numeric(name) {
                        return type_error(format!(
                            "show(): y column '{name}' is not numeric: it must hold only numbers and None, and at least one number"
                        ));
                    }
                }
                y
            }
            _ => {
                let mut y: Vec<String> = columns
                    .iter()
                    .filter(|c| **c != x && all_numeric(c))
                    .cloned()
                    .collect();
                if kind == "pie" {
                    y.truncate(1);
                }
                if y.is_empty() {
                    return type_error(format!(
                        "show(kind='{kind}') needs a numeric column for y; columns are {}",
                        columns.join(", ")
                    ));
                }
                y
            }
        };
        (Some(x), y)
    };
    Ok(Normalized {
        kind,
        title,
        columns,
        rows,
        rows_total,
        x,
        y,
        text: None,
        truncated,
    })
}

/// The display file content (`displays/<index>.json`).
pub fn file_json(display: &Normalized, index: u32) -> Value {
    let mut out =
        json!({"schema": 1, "index": index, "kind": display.kind, "truncated": display.truncated});
    if let Some(title) = &display.title {
        out["title"] = json!(title);
    }
    if let Some(text) = &display.text {
        out["text"] = json!(text);
    } else {
        out["columns"] = json!(display.columns);
        out["rows"] = json!(display.rows);
        out["rows_total"] = json!(display.rows_total);
    }
    if let Some(x) = &display.x {
        out["x"] = json!(x);
    }
    if !display.y.is_empty() {
        out["y"] = json!(display.y);
    }
    out
}

/// Display files up to this size are written indented, one row per line.
pub const READABLE_FILE_BYTES: usize = 4 * 1024 * 1024;

/// The bytes of a display file: indented with each row on one line while that stays
/// under `READABLE_FILE_BYTES`, otherwise compact.
pub fn file_bytes(file: &Value) -> Vec<u8> {
    let compact = serde_json::to_vec(file).unwrap_or_default();
    let Value::Object(map) = file else {
        return compact;
    };
    let mut out = String::from("{\n");
    for (i, (key, value)) in map.iter().enumerate() {
        out.push_str("  ");
        out.push_str(&Value::String(key.clone()).to_string());
        out.push_str(": ");
        match value {
            Value::Array(rows) if key == "rows" && !rows.is_empty() => {
                out.push_str("[\n");
                for (j, row) in rows.iter().enumerate() {
                    out.push_str("    ");
                    out.push_str(&row.to_string());
                    out.push_str(if j + 1 < rows.len() { ",\n" } else { "\n" });
                }
                out.push_str("  ]");
            }
            _ => out.push_str(&value.to_string()),
        }
        out.push_str(if i + 1 < map.len() { ",\n" } else { "\n" });
        if out.len() >= READABLE_FILE_BYTES {
            return compact;
        }
    }
    out.push_str("}\n");
    if out.len() >= READABLE_FILE_BYTES {
        return compact;
    }
    out.into_bytes()
}

/// Reads a display file back into its normalized form (for the workspace view).
pub fn from_file(value: &Value) -> Option<(u32, Normalized)> {
    let index = u32::try_from(value["index"].as_u64()?).ok()?;
    let kind = value["kind"].as_str()?.to_owned();
    let strings = |v: &Value| -> Option<Vec<String>> {
        v.as_array()?
            .iter()
            .map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    let rows: Vec<Vec<Value>> = match value.get("rows") {
        Some(rows) => rows
            .as_array()?
            .iter()
            .map(|r| r.as_array().map(|r| r.iter().map(cell).collect()))
            .collect::<Option<_>>()?,
        None => vec![],
    };
    Some((
        index,
        Normalized {
            kind,
            title: value["title"].as_str().map(|t| chars_cut(t, TITLE_CHARS).0),
            columns: value.get("columns").map_or(Some(vec![]), strings)?,
            rows_total: value["rows_total"].as_u64().unwrap_or(rows.len() as u64),
            rows,
            x: value["x"].as_str().map(str::to_owned),
            y: value.get("y").map_or(Some(vec![]), strings)?,
            text: value["text"].as_str().map(str::to_owned),
            truncated: value["truncated"] == true,
        },
    ))
}

/// The inline form sent to the GUI, with the inline caps applied.
pub fn inline(display: &Normalized, index: u32, path: &Path) -> PythonDisplay {
    let mut truncated = display.truncated;
    let mut out = PythonDisplay {
        index,
        kind: display.kind.clone(),
        title: display.title.clone(),
        columns: vec![],
        rows: None,
        rows_total: None,
        truncated: false,
        x: display.x.clone(),
        y: display.y.clone(),
        text: None,
        path: path.to_string_lossy().into_owned(),
    };
    if let Some(text) = &display.text {
        let (text, cut) = chars_cut(text, INLINE_MARKDOWN_CHARS);
        truncated |= cut;
        out.text = Some(text);
    } else {
        let max_rows = if display.kind == "table" {
            INLINE_TABLE_ROWS
        } else {
            INLINE_CHART_ROWS
        };
        let mut keep: Vec<usize> = (0..display.columns.len()).collect();
        if keep.len() > INLINE_MAX_COLUMNS {
            truncated = true;
            let mut wanted: Vec<usize> = display
                .x
                .iter()
                .chain(&display.y)
                .filter_map(|name| display.columns.iter().position(|c| c == name))
                .collect();
            for i in 0..display.columns.len() {
                if wanted.len() >= INLINE_MAX_COLUMNS {
                    break;
                }
                if !wanted.contains(&i) {
                    wanted.push(i);
                }
            }
            wanted.truncate(INLINE_MAX_COLUMNS);
            wanted.sort_unstable();
            keep = wanted;
        }
        out.columns = keep.iter().map(|&i| display.columns[i].clone()).collect();
        out.y.retain(|name| out.columns.contains(name));
        if display.rows.len() > max_rows || display.rows_total > display.rows.len() as u64 {
            out.rows_total = Some(display.rows_total);
        }
        truncated |= display.rows.len() > max_rows;
        out.rows = Some(
            display
                .rows
                .iter()
                .take(max_rows)
                .map(|row| {
                    keep.iter()
                        .map(|&i| match row.get(i).cloned().unwrap_or(Value::Null) {
                            Value::String(s) => {
                                let (s, cut) = chars_cut(&s, INLINE_CELL_CHARS);
                                truncated |= cut;
                                Value::String(s)
                            }
                            other => other,
                        })
                        .collect()
                })
                .collect(),
        );
    }
    out.truncated = truncated;
    out
}

/// The model-visible line of one display: shape only, never data.
pub fn note_line(display: &Normalized) -> String {
    let title = display
        .title
        .as_ref()
        .map(|t| format!(" {}", serde_json::to_string(t).unwrap_or_default()))
        .unwrap_or_default();
    if let Some(text) = &display.text {
        return format!(
            "shown to the user: markdown{title} ({} characters)",
            text.chars().count()
        );
    }
    let rows = match display.rows_total {
        1 => "1 row".to_owned(),
        n => format!("{n} rows"),
    };
    if display.kind == "table" {
        let columns = match display.columns.len() {
            1 => "1 column".to_owned(),
            n => format!("{n} columns"),
        };
        format!("shown to the user: table{title} ({rows} × {columns})")
    } else {
        format!(
            "shown to the user: {} chart{title} ({rows}; x={}; y={})",
            display.kind,
            display.x.as_deref().unwrap_or(""),
            display.y.join(", ")
        )
    }
}

/// Displays of one run: every `show()` call counts, at most 12 / 512 KB are kept inline.
#[derive(Debug, Default)]
pub struct DisplayLog {
    pub total: u32,
    pub inline: Vec<PythonDisplay>,
    pub inline_bytes: usize,
    pub truncated: bool,
    pub notes: Vec<String>,
    pub file_bytes: u64,
}

/// `show()` failure: a TypeError for bad arguments or a RuntimeError for a run limit.
#[derive(Debug, PartialEq, Eq)]
pub enum ShowError {
    Type(String),
    Limit(String),
}

impl DisplayLog {
    /// Writes the display file and returns the inline display when it is kept inline.
    pub async fn show(
        &mut self,
        run_dir: &Path,
        args: &Value,
        kwargs: &Value,
        max_markdown_bytes: usize,
    ) -> Result<Option<PythonDisplay>, ShowError> {
        if self.total >= RUN_MAX_SHOWS {
            return Err(ShowError::Limit(format!(
                "show(): the limit for this run is reached ({RUN_MAX_SHOWS} show calls per run)"
            )));
        }
        let display = normalize(args, kwargs, max_markdown_bytes).map_err(ShowError::Type)?;
        let content = file_bytes(&file_json(&display, self.total + 1));
        if self.file_bytes.saturating_add(content.len() as u64) > RUN_MAX_FILE_BYTES {
            return Err(ShowError::Limit(format!(
                "show(): the limit for this run is reached ({} MiB of display files per run)",
                RUN_MAX_FILE_BYTES / (1024 * 1024)
            )));
        }
        let index = self.total + 1;
        let dir = run_dir.join("displays");
        let path: PathBuf = dir.join(format!("{index}.json"));
        let save_error =
            |e: String| ShowError::Type(format!("show(): cannot save the display: {e}"));
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| save_error(e.to_string()))?;
        super::workspace::atomic_write(&path, &content)
            .await
            .map_err(save_error)?;
        self.file_bytes += content.len() as u64;
        self.total = index;
        self.notes.push(note_line(&display));
        let inline = inline(&display, index, &path);
        let bytes = serde_json::to_vec(&inline).map_or(usize::MAX, |b| b.len());
        if self.inline.len() >= RUN_INLINE_DISPLAYS
            || self.inline_bytes.saturating_add(bytes) > RUN_INLINE_BYTES
        {
            self.truncated = true;
            return Ok(None);
        }
        self.inline_bytes += bytes;
        self.inline.push(inline.clone());
        Ok(Some(inline))
    }

    /// Adds `displays`, `displays_total` and `displays_truncated` when anything was shown.
    pub fn extend_extra(&self, extra: &mut Value) {
        if self.total == 0 {
            return;
        }
        extra["displays"] = json!(self.inline);
        extra["displays_total"] = json!(self.total);
        extra["displays_truncated"] = json!(self.truncated);
    }

    /// Model-visible lines, at most 12 then `… and N more`.
    pub fn model_text(&self) -> String {
        let mut lines: Vec<String> = self.notes.iter().take(MODEL_LINES).cloned().collect();
        if self.notes.len() > MODEL_LINES {
            lines.push(format!("… and {} more", self.notes.len() - MODEL_LINES));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;

#[cfg(test)]
mod file_form_tests {
    use super::*;

    async fn written(rows: Value) -> (DisplayLog, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut log = DisplayLog::default();
        log.show(dir.path(), &json!([rows]), &json!({"title":"T"}), 1 << 20)
            .await
            .unwrap();
        let text = std::fs::read_to_string(dir.path().join("displays/1.json")).unwrap();
        (log, text)
    }

    #[tokio::test]
    async fn small_files_are_indented_with_one_row_per_line() {
        let (log, text) = written(json!([{"a":1,"b":"x"},{"a":2,"b":"y"}])).await;
        assert_eq!(log.file_bytes, text.len() as u64);
        assert!(
            text.contains("\n  \"rows\": [\n    [1,\"x\"],\n    [2,\"y\"]\n  ],\n"),
            "{text}"
        );
        assert!(text.contains("\n  \"columns\": [\"a\",\"b\"],\n"), "{text}");
        let value: Value = serde_json::from_str(&text).unwrap();
        let (index, back) = from_file(&value).unwrap();
        assert_eq!(
            (index, back.rows.len(), back.title.as_deref()),
            (1, 2, Some("T"))
        );
        let (_, empty) = written(json!([])).await;
        assert!(empty.contains("\"rows\": [],"), "{empty}");
        assert!(serde_json::from_str::<Value>(&empty).is_ok());
    }

    #[tokio::test]
    async fn files_over_4_mib_stay_compact() {
        let rows: Vec<Value> = (0..1000).map(|i| json!([i, "w".repeat(5000)])).collect();
        let (log, text) = written(json!(rows)).await;
        assert!(text.len() > READABLE_FILE_BYTES);
        assert!(!text.contains('\n'));
        assert_eq!(log.file_bytes, text.len() as u64);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(from_file(&value).unwrap().1.rows.len(), 1000);
    }
}
