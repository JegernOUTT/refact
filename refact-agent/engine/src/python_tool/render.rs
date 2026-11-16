use serde_json::Value;
pub fn cut(text: &str, cap: usize) -> String {
    let count = text.chars().count();
    if count <= cap {
        return text.into();
    }
    let mut keep = cap;
    loop {
        let marker = format!("... {} more chars", count - keep);
        let next = cap.saturating_sub(marker.chars().count());
        if next >= keep {
            return format!("{}{}", text.chars().take(keep).collect::<String>(), marker)
                .chars()
                .take(cap)
                .collect();
        }
        keep = next;
    }
}
fn cut_title(text: &str, cap: usize) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= cap {
        return Some(text.into());
    }
    Some(text.chars().take(cap - 1).chain(['…']).collect())
}
pub fn run_description(description: Option<&str>) -> Option<String> {
    cut_title(
        &description?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        160,
    )
}
pub fn code_preview(code: &str) -> Option<String> {
    let lines: Vec<&str> = code
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !l.trim().is_empty())
        .collect();
    let first = lines.first()?.trim();
    let title = if first.starts_with('#')
        && !first.starts_with("#!")
        && !first.trim_start_matches('#').trim().is_empty()
    {
        Some(first)
    } else {
        const SKIP: [&str; 9] = [
            "#",
            "import ",
            "from ",
            "def ",
            "async def ",
            "class ",
            "@",
            "\"\"\"",
            "'''",
        ];
        lines
            .iter()
            .find(|l| !l.starts_with(char::is_whitespace) && !SKIP.iter().any(|p| l.starts_with(p)))
            .or_else(|| lines.iter().find(|l| !l.trim().starts_with('#')))
            .map(|l| l.trim())
    };
    cut_title(title?, 120)
}
pub fn run_labels(description: Option<&str>, code: &str) -> Value {
    let mut labels = serde_json::Map::new();
    if let Some(d) = run_description(description) {
        labels.insert("description".into(), d.into());
    }
    if let Some(c) = code_preview(code) {
        labels.insert("code_preview".into(), c.into());
    }
    Value::Object(labels)
}
pub fn render(result: &str, stdout: &str, extra: &Value, cap: usize) -> String {
    let mut rest = String::new();
    if !extra["error"].is_null() {
        let e = &extra["error"];
        rest.push_str(&format!(
            "Error: {}: {}\n{}\n",
            e["type"].as_str().unwrap_or("Error"),
            e["message"].as_str().unwrap_or(""),
            e["traceback"].as_str().unwrap_or("")
        ));
    }
    if let Some(calls) = extra["calls"].as_array().filter(|a| !a.is_empty()) {
        rest.push_str("Nested tool calls:\n");
        for c in calls {
            rest.push_str(&format!(
                "- {}: {} ({} ms) {}\n",
                c["tool"].as_str().unwrap_or(""),
                c["status"].as_str().unwrap_or(""),
                c["duration_ms"],
                c["summary"].as_str().unwrap_or("").replace('\n', " ")
            ));
        }
    }
    if let Some(llm) = extra.get("llm").filter(|v| v.is_object()) {
        rest.push_str(&format!(
            "Model calls: {} ({} input / {} output tokens; {})\n",
            llm["calls"],
            llm["input_tokens"],
            llm["output_tokens"],
            llm["models"]
                .as_array()
                .map(|m| m
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default()
        ));
    }
    if let Some(files) = extra["files_written"].as_array().filter(|a| !a.is_empty()) {
        rest.push_str("Files written:\n");
        for f in files {
            rest.push_str(&format!(
                "- {} ({} bytes)\n",
                f["path"].as_str().unwrap_or(""),
                f["bytes"]
            ));
        }
    }
    let state = &extra["state"];
    if state["restored"] == true {
        rest.push_str("State: restored from disk\n");
    }
    let limit = extra["status"] == "limit_exceeded";
    if let Some(reason) = state["reset_reason"].as_str() {
        if limit {
            rest.push_str(&format!("State: {reason}\n"));
        } else {
            rest.push_str(&format!("State: reset ({reason})\n"));
        }
    }
    if state["persisted"] != true && !limit {
        rest.push_str("State: not persisted\n");
    }
    let a = if result.is_empty() {
        String::new()
    } else {
        format!("Result:\n{result}")
    };
    let b = if stdout.is_empty() {
        String::new()
    } else {
        format!("Printed output:\n{stdout}")
    };
    let parts: Vec<_> = [
        cut(&a, cap * 50 / 100),
        cut(&b, cap * 30 / 100),
        cut(rest.trim_end(), cap * 20 / 100),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    cut(&parts.join("\n\n"), cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_run_has_exact_rendering() {
        assert_eq!(
            render(
                "3",
                "hello\n",
                &serde_json::json!({"error":null,"state":{"persisted":true}}),
                20_000
            ),
            "Result:\n3\n\nPrinted output:\nhello\n"
        );
    }
    #[test]
    fn run_titles_are_collapsed_and_cut_on_char_boundaries() {
        assert_eq!(
            run_description(Some("  Count   TODO\n\tmarkers ")).as_deref(),
            Some("Count TODO markers")
        );
        assert_eq!(run_description(Some(" \n ")), None);
        assert_eq!(run_description(None), None);
        let long = run_description(Some(&"é".repeat(400))).unwrap();
        assert_eq!(long.chars().count(), 160);
        assert_eq!(long, format!("{}…", "é".repeat(159)));
        assert_eq!(
            code_preview("\n   \n  x = 1  \ny = 2").as_deref(),
            Some("y = 2")
        );
        assert_eq!(code_preview(" \n\t\n"), None);
        let preview = code_preview(&"ж".repeat(121)).unwrap();
        assert_eq!(preview, format!("{}…", "ж".repeat(119)));
        assert_eq!(code_preview(&"ж".repeat(120)).unwrap(), "ж".repeat(120));
        assert_eq!(
            run_labels(Some("d"), "c"),
            serde_json::json!({"description":"d","code_preview":"c"})
        );
        assert_eq!(run_labels(None, ""), serde_json::json!({}));
    }
    #[test]
    fn llm_calls_are_listed_with_a_totals_line() {
        let extra = serde_json::json!({"error":null,"state":{"persisted":true},"calls":[{"tool":"llm","status":"ok","duration_ms":5,"summary":"m · 3 in / 1 out tokens"}],"llm":{"calls":1,"input_tokens":3,"output_tokens":1,"models":["m"]}});
        let text = render("", "", &extra, 20_000);
        assert!(
            text.contains("- llm: ok (5 ms) m · 3 in / 1 out tokens\n"),
            "{text}"
        );
        assert!(
            text.contains("Model calls: 1 (3 input / 1 output tokens; m)"),
            "{text}"
        );
    }
    #[test]
    fn code_preview_selects_the_title_line() {
        let vectors: &[(&str, Option<&str>)] = &[
            (
                "# Count TODO markers\nimport os\nx = 1",
                Some("# Count TODO markers"),
            ),
            (
                "#!/usr/bin/env python\nimport os\nfrom a import b\n\nx = compute()\n",
                Some("x = compute()"),
            ),
            (
                "import os\n\ndef f():\n    return 1\n\nresult = f()\nprint(result)",
                Some("result = f()"),
            ),
            ("import os\nimport json", Some("import os")),
            ("#\n#  \nvalue = 1", Some("value = 1")),
            ("   \n\t\n", None),
            ("# only a comment", Some("# only a comment")),
            ("## Heading style\nx=1", Some("## Heading style")),
            (
                "\"\"\"Docstring\"\"\"\nimport os\nrows = load()",
                Some("rows = load()"),
            ),
            ("@decorator\ndef f(): pass\nawait f()", Some("await f()")),
            ("if x:\n    y = 1", Some("if x:")),
            ("# Title\r\nx = 1\r\n", Some("# Title")),
            ("x = 1  # trailing", Some("x = 1  # trailing")),
            ("  # indented comment\nx = 1", Some("# indented comment")),
            (
                "async def main():\n    pass\nawait main()",
                Some("await main()"),
            ),
            ("# a\n# b", Some("# a")),
            ("#!/bin/sh\n# note", None),
        ];
        for (i, (code, expected)) in vectors.iter().enumerate() {
            assert_eq!(code_preview(code).as_deref(), *expected, "vector {}", i + 1);
        }
    }
}
