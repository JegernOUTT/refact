//! Pure parts of the Python `run()` helper: its arguments, the shell call it makes, and the
//! result it builds from the output the shell kept.

use std::sync::LazyLock;

use regex::{Regex, RegexSet};
use serde_json::{json, Value};

pub const MAX_TIMEOUT_SECS: u64 = 3600;
pub const DEFAULT_TAIL_LINES: usize = 60;
pub const MAX_TAIL_LINES: usize = 2000;
pub const TAIL_LINE_CHARS: usize = 1000;
pub const TAIL_MAX_BYTES: usize = 64 * 1024;
pub const MAX_PROBLEMS: usize = 20;
pub const PROBLEM_CHARS: usize = 300;
const DESCRIPTION_CHARS: usize = 80;
const ARGS: &[&str] = &["cmd", "timeout", "cwd", "tail"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunArgs {
    pub cmd: String,
    pub timeout: Option<u64>,
    pub cwd: Option<String>,
    pub tail: usize,
}

/// Checks the keyword arguments of one `run()` call; `Err` is the TypeError text.
pub fn parse_args(kwargs: &Value) -> Result<RunArgs, String> {
    let args = kwargs
        .as_object()
        .ok_or("run() arguments must be keywords")?;
    if let Some(unknown) = args.keys().find(|k| !ARGS.contains(&k.as_str())) {
        return Err(format!(
            "run() got an unexpected keyword argument '{unknown}'"
        ));
    }
    let cmd = match args.get("cmd") {
        None | Some(Value::Null) => return Err("run() missing required argument: 'cmd'".into()),
        Some(Value::String(cmd)) if !cmd.trim().is_empty() => cmd.clone(),
        Some(_) => return Err("run() cmd must be a non-empty string".into()),
    };
    let timeout = match args.get("timeout") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n))
            if n.as_u64()
                .is_some_and(|n| (1..=MAX_TIMEOUT_SECS).contains(&n)) =>
        {
            n.as_u64()
        }
        Some(_) => {
            return Err(format!(
                "run() timeout must be whole seconds between 1 and {MAX_TIMEOUT_SECS}, or None"
            ))
        }
    };
    let cwd = match args.get("cwd") {
        None | Some(Value::Null) => None,
        Some(Value::String(cwd)) if !cwd.is_empty() => Some(cwd.clone()),
        Some(_) => return Err("run() cwd must be a non-empty str or None".into()),
    };
    let tail = match args.get("tail") {
        None | Some(Value::Null) => DEFAULT_TAIL_LINES,
        Some(Value::Number(n)) if n.as_u64().is_some_and(|n| n <= MAX_TAIL_LINES as u64) => {
            n.as_u64().unwrap() as usize
        }
        Some(_) => {
            return Err(format!(
                "run() tail must be an integer between 0 and {MAX_TAIL_LINES}"
            ))
        }
    };
    Ok(RunArgs {
        cmd,
        timeout,
        cwd,
        tail,
    })
}

/// Arguments of the one nested `shell` call: the command in the foreground, no output options.
pub fn shell_kwargs(args: &RunArgs) -> Value {
    let joined = args.cmd.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut short = cut_chars(&joined, DESCRIPTION_CHARS);
    if short.len() < joined.len() {
        short.push('…');
    }
    let mut kwargs = json!({
        "command": args.cmd,
        "description": format!("run(): {short}"),
    });
    if let Some(timeout) = args.timeout {
        kwargs["timeout"] = json!(timeout);
    }
    if let Some(cwd) = &args.cwd {
        kwargs["workdir"] = json!(cwd);
    }
    kwargs
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]").unwrap()
});

static PROBLEM: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        r"^\s*error(\[[A-Z]\d+\])?:",
        r"^\S+:\d+(:\d+)?:\s*(fatal )?error\b",
        r"\berror TS\d+:",
        r"^test \S+ \.\.\. FAILED$",
        r"^failures:$",
        r"panicked at",
        r"^(FAILED|ERROR) \S",
        r"^E\s{2,}\S",
        r"^Traceback \(most recent call last\):",
        r"^\w*(Error|Exception)(: |$)",
        r"^--- FAIL:",
        r"^\s*(FAIL|✕|×) \S",
        r"^npm ERR!",
        r"(?i)^\s*fatal:",
    ])
    .unwrap()
});

static NOT_PROBLEM: LazyLock<RegexSet> =
    LazyLock::new(|| RegexSet::new([r"\b0 (failed|errors?)\b", r"^warning"]).unwrap());

/// One output line as a terminal shows it: escape sequences removed and, after a carriage
/// return, only the text that overwrote the start of the line.
pub fn visible_line(line: &str) -> String {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let line = line
        .rsplit('\r')
        .find(|part| !part.is_empty())
        .unwrap_or_default();
    ANSI.replace_all(line, "").into_owned()
}

fn cut_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => text[..end].to_string(),
        None => text.to_string(),
    }
}

/// The first distinct error lines in output order: compiler errors, failed tests, panics,
/// tracebacks and similar; warnings and "0 failed" summaries are left out.
pub fn problems(output: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in output.lines() {
        let line = visible_line(line);
        if !PROBLEM.is_match(&line) || NOT_PROBLEM.is_match(&line) {
            continue;
        }
        let line = cut_chars(line.trim_end(), PROBLEM_CHARS);
        if !found.contains(&line) {
            found.push(line);
            if found.len() == MAX_PROBLEMS {
                break;
            }
        }
    }
    found
}

/// The last `lines` lines of the output, each cut to `TAIL_LINE_CHARS`, dropping the oldest of
/// them until the text fits `TAIL_MAX_BYTES`.
pub fn tail(output: &str, lines: usize) -> String {
    if lines == 0 {
        return String::new();
    }
    let mut kept: Vec<String> = Vec::new();
    let mut bytes = 0usize;
    for line in output.lines().rev().take(lines.min(MAX_TAIL_LINES)) {
        let line = cut_chars(&visible_line(line), TAIL_LINE_CHARS);
        let cost = line.len() + usize::from(!kept.is_empty());
        if bytes + cost > TAIL_MAX_BYTES {
            break;
        }
        bytes += cost;
        kept.push(line);
    }
    kept.reverse();
    kept.join("\n")
}

/// What the shell call ended with, as the exec layer reports it.
pub struct RunEnd<'a> {
    /// `exited`, `timed_out`, `killed` or `failed`.
    pub status: &'a str,
    pub exit_code: Option<i64>,
    pub duration_ms: u64,
    /// The output the shell kept, stdout and stderr in arrival order.
    pub output: &'a str,
    pub log: Option<String>,
    /// The kept output is not everything the command printed.
    pub truncated: bool,
    /// The exec layer's failure text for `failed`.
    pub failure: Option<&'a str>,
}

fn duration_s(duration_ms: u64) -> f64 {
    (duration_ms as f64 / 100.0).round() / 10.0
}

/// The value `run()` returns and the one-line summary of its call record.
pub fn result(end: &RunEnd, tail_lines: usize) -> (Value, String) {
    let status = match end.status {
        "exited" | "timed_out" | "killed" => end.status,
        _ => "failed",
    };
    let ok = status == "exited" && end.exit_code == Some(0);
    let error = match status {
        "exited" if ok => None,
        "exited" => Some(match end.exit_code {
            Some(code) => format!("the command exited with code {code}"),
            None => "the command exited without an exit code".to_string(),
        }),
        "timed_out" => Some("the command timed out and was killed".to_string()),
        "killed" => Some("the command was stopped".to_string()),
        _ => Some(format!(
            "the command failed: {}",
            end.failure.unwrap_or("no exit status")
        )),
    };
    let problems = problems(end.output);
    let seconds = duration_s(end.duration_ms);
    let head = match (status, end.exit_code) {
        ("exited", Some(code)) => format!("exit {code}"),
        ("timed_out", _) => "timed out".to_string(),
        (status, _) => status.to_string(),
    };
    let mut summary = format!("{head} · {seconds:.1} s");
    if !problems.is_empty() {
        summary.push_str(&format!(
            " · {} problem{}",
            problems.len(),
            if problems.len() == 1 { "" } else { "s" }
        ));
    }
    let value = json!({
        "ok": ok,
        "exit_code": end.exit_code,
        "status": status,
        "duration_s": seconds,
        "tail": tail(end.output, tail_lines),
        "problems": problems,
        "log": end.log,
        "truncated": end.truncated,
        "error": error,
    });
    (value, summary)
}

/// The largest `count` or less that `fits` accepts, assuming that fewer always fit when more do.
fn largest_fitting(count: usize, mut fits: impl FnMut(usize) -> bool) -> Option<usize> {
    if !fits(0) {
        return None;
    }
    let (mut low, mut high) = (0, count);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if fits(middle) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    Some(low)
}

/// Fits a `run()` value to what `fits` accepts: the oldest `tail` lines go first, then the last
/// `problems`; status, exit code, log and the other fields always stay. False when the value
/// does not fit even without tail and problems.
pub fn fit_result(value: &mut Value, fits: impl Fn(&Value) -> bool) -> bool {
    if fits(value) {
        return true;
    }
    let tail = value["tail"].as_str().unwrap_or_default().to_string();
    let lines: Vec<&str> = if tail.is_empty() {
        Vec::new()
    } else {
        tail.split('\n').collect()
    };
    let last = |kept: usize| json!(lines[lines.len() - kept..].join("\n"));
    if let Some(kept) = largest_fitting(lines.len(), |kept| {
        value["tail"] = last(kept);
        fits(value)
    }) {
        value["tail"] = last(kept);
        return true;
    }
    value["tail"] = json!("");
    let problems = value["problems"].as_array().cloned().unwrap_or_default();
    match largest_fitting(problems.len(), |kept| {
        value["problems"] = Value::Array(problems[..kept].to_vec());
        fits(value)
    }) {
        Some(kept) => {
            value["problems"] = Value::Array(problems[..kept].to_vec());
            true
        }
        None => {
            value["problems"] = json!([]);
            false
        }
    }
}

/// The value `run()` returns when the shell output may not reach the script.
pub fn withheld(exit_code: Option<i64>, duration_ms: u64, reason: &str) -> (Value, String) {
    let seconds = duration_s(duration_ms);
    (
        json!({
            "ok": false,
            "exit_code": exit_code,
            "status": "withheld",
            "duration_s": seconds,
            "tail": "",
            "problems": [],
            "log": null,
            "truncated": false,
            "error": reason,
        }),
        format!("withheld · {seconds:.1} s"),
    )
}

#[cfg(test)]
#[path = "run_helper_tests.rs"]
mod tests;
