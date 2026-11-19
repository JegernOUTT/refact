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
/// Weights of the result, printed output and remaining sections when they share the budget.
const WEIGHTS: [usize; 3] = [5, 3, 2];

/// Splits `budget` by weight; a section that needs less than its share passes the rest on.
fn allocate(needs: [usize; 3], budget: usize) -> [usize; 3] {
    let mut alloc = [0; 3];
    let mut active: Vec<usize> = (0..3).filter(|&i| needs[i] > 0).collect();
    let mut left = budget;
    while !active.is_empty() {
        let weight: usize = active.iter().map(|&i| WEIGHTS[i]).sum();
        let shares: Vec<usize> = active.iter().map(|&i| left * WEIGHTS[i] / weight).collect();
        let fits: Vec<usize> = active
            .iter()
            .zip(&shares)
            .filter(|(i, share)| needs[**i] <= **share)
            .map(|(i, _)| *i)
            .collect();
        if fits.is_empty() {
            let mut spare = left - shares.iter().sum::<usize>();
            for (&i, &share) in active.iter().zip(&shares) {
                alloc[i] = share + usize::from(spare > 0);
                spare = spare.saturating_sub(1);
            }
            break;
        }
        for &i in &fits {
            alloc[i] = needs[i];
            left -= needs[i];
        }
        active.retain(|i| !fits.contains(i));
    }
    alloc
}

/// The whole tool message: notices, then the error and state lines (never cut by long output),
/// then result, printed output and the rest, all within `cap` characters.
pub fn render(
    result: &str,
    stdout: &str,
    extra: &Value,
    notices: &str,
    displays: &str,
    cap: usize,
) -> String {
    let mut error = None;
    let mut hint = None;
    let mut rest = String::new();
    if !extra["error"].is_null() {
        let e = &extra["error"];
        error = Some(format!(
            "Error: {}: {}",
            e["type"].as_str().unwrap_or("Error"),
            e["message"].as_str().unwrap_or("")
        ));
        hint = e["hint"].as_str().map(|h| format!("Hint: {h}"));
        let traceback = e["traceback"].as_str().unwrap_or("");
        if !traceback.is_empty() {
            rest.push_str(traceback);
            rest.push('\n');
        }
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
    if !displays.is_empty() {
        if !rest.is_empty() {
            rest.push('\n');
        }
        rest.push_str(displays);
    }
    let mut state = Vec::new();
    let s = &extra["state"];
    if s["restored"] == true {
        state.push("State: restored from disk".to_string());
    }
    let limit = extra["status"] == "limit_exceeded";
    if let Some(reason) = s["reset_reason"].as_str() {
        if limit {
            state.push(format!("State: {reason}"));
        } else {
            state.push(format!("State: reset ({reason})"));
        }
    }
    for note in s["notes"].as_array().into_iter().flatten() {
        if let Some(note) = note.as_str() {
            state.push(format!("State: {note}"));
        }
    }
    if let Some(text) = s["not_saved"].as_str() {
        state.push(format!("State: {text}"));
    } else if s["persisted"] != true && !limit {
        state.push("State: not persisted".to_string());
    }
    if let Some(note) = s["note"].as_str() {
        state.push(format!("State: {note}"));
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
    let c = rest.trim_end();
    let room = cap.saturating_sub(notices.chars().count());
    let following = [!a.is_empty(), !b.is_empty(), !c.is_empty()]
        .into_iter()
        .filter(|present| *present)
        .count();
    let others: Vec<String> = hint.into_iter().chain(state).collect();
    let whole = error
        .iter()
        .chain(&others)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let lead = if whole.chars().count() <= room {
        whole
    } else {
        let budget = room.saturating_sub(2 * following);
        let others_len: usize = others.iter().map(|line| line.chars().count() + 1).sum();
        let lead = error
            .map(|error| {
                if error.chars().count() + others_len <= budget {
                    error
                } else {
                    cut(&error, budget / 2)
                }
            })
            .into_iter()
            .chain(others)
            .collect::<Vec<_>>()
            .join("\n");
        cut(&lead, budget)
    };
    let separators = 2 * (following + usize::from(!lead.is_empty())).saturating_sub(1);
    let alloc = allocate(
        [a.chars().count(), b.chars().count(), c.chars().count()],
        room.saturating_sub(lead.chars().count() + separators),
    );
    let parts: Vec<_> = [lead, cut(&a, alloc[0]), cut(&b, alloc[1]), cut(c, alloc[2])]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    cut(&format!("{notices}{}", parts.join("\n\n")), cap)
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
                "",
                "",
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
        let text = render("", "", &extra, "", "", 20_000);
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
    fn allocate_shares_by_weight_and_passes_unused_budget_on() {
        for (needs, budget, expected) in [
            ([0, 50_000, 200], 19_998, [0, 19_798, 200]),
            ([30_000, 0, 100], 19_998, [19_898, 0, 100]),
            ([30_000, 30_000, 1_000], 19_996, [11_873, 7_123, 1_000]),
            ([1_000, 40_000, 15_000], 19_996, [1_000, 11_398, 7_598]),
            ([30_000, 30_000, 30_000], 19_996, [9_999, 5_998, 3_999]),
            ([10, 20, 30], 19_996, [10, 20, 30]),
            ([0, 0, 0], 19_996, [0, 0, 0]),
        ] {
            assert_eq!(allocate(needs, budget), expected, "{needs:?}");
        }
    }
    #[test]
    fn notices_error_and_state_lines_lead_and_long_output_cannot_cut_them() {
        let calls: Vec<Value> = (0..200)
            .map(|i| serde_json::json!({"tool":"cat","status":"ok","duration_ms":i,"summary":"s"}))
            .collect();
        let extra = serde_json::json!({"status":"failed","error":{"type":"ValueError","message":"boom","traceback":"t".repeat(5000)},"calls":calls,"state":{"persisted":false,"reset_reason":"Python snapshot digest mismatch"}});
        let notes = "shown to the user: table \"t\" (1 row × 1 column)";
        let text = render(
            "",
            &"o".repeat(100_000),
            &extra,
            "Background runs are not available in this chat.\n",
            notes,
            1000,
        );
        assert!(text.chars().count() <= 1000, "{}", text.chars().count());
        assert!(
            text.starts_with("Background runs are not available in this chat.\nError: ValueError: boom\nState: reset (Python snapshot digest mismatch)\nState: not persisted\n\nPrinted output:\nooo"),
            "{text}"
        );
        assert!(text.contains("\n\ntttt"), "{text}");
    }
    #[test]
    fn an_error_that_fits_the_budget_is_not_cut() {
        let extra = serde_json::json!({"error":{"type":"ValueError","message":"e".repeat(600),"traceback":""},"state":{"persisted":true}});
        for stdout in ["", &"o".repeat(50_000)] {
            let text = render("", stdout, &extra, "", "", 1000);
            assert!(text.chars().count() <= 1000, "{}", text.chars().count());
            assert!(
                text.starts_with(&format!("Error: ValueError: {}", "e".repeat(600))),
                "{text}"
            );
        }
    }
    #[test]
    fn an_oversized_error_message_takes_at_most_half_the_budget() {
        let extra = serde_json::json!({"error":{"type":"ValueError","message":"e".repeat(50_000),"traceback":""},"state":{"persisted":true}});
        let text = render("", &"o".repeat(50_000), &extra, "", "", 1000);
        assert!(text.chars().count() <= 1000, "{}", text.chars().count());
        let lead = text.split("\n\n").next().unwrap();
        assert!(lead.starts_with("Error: ValueError: eee"), "{text}");
        assert!(lead.chars().count() <= 499, "{}", lead.chars().count());
        assert!(text.contains("\n\nPrinted output:\nooo"), "{text}");
    }
    #[test]
    fn a_lead_that_fits_is_never_cut_for_printed_output() {
        let message = "e".repeat(999 - "Error: ValueError: ".len());
        let lead = format!("Error: ValueError: {message}");
        assert_eq!(lead.chars().count(), 999);
        let extra = serde_json::json!({"error":{"type":"ValueError","message":message,"traceback":""},"state":{"persisted":true}});
        assert_eq!(render("", &"o".repeat(5000), &extra, "", "", 1000), lead);
    }
    #[test]
    fn start_notes_and_a_refused_save_are_state_lines_and_not_resets() {
        let extra = serde_json::json!({"error":null,"state":{"restored":false,"reset_reason":null,"persisted":false,"bytes":2000000,"not_saved":"not saved (state is 2.0 MB, over the limit 1.0 MB)","notes":["variables were not copied from chat a: why","2 artifacts were not copied from chat a: why"]}});
        assert_eq!(
            render("1", "", &extra, "", "", 20_000),
            "State: variables were not copied from chat a: why\nState: 2 artifacts were not copied from chat a: why\nState: not saved (state is 2.0 MB, over the limit 1.0 MB)\n\nResult:\n1"
        );
    }
    #[test]
    fn the_whole_message_never_exceeds_the_cap() {
        let notes = (0..13)
            .map(|i| format!("shown to the user: table \"t{i}\" (1 row × 1 column)"))
            .collect::<Vec<_>>()
            .join("\n");
        for cap in [1_000, 1_337, 20_000] {
            for size in [0, 7, 999, 50_000] {
                for ch in ["a", "é", "ж", "😀"] {
                    for error in [
                        Value::Null,
                        serde_json::json!({"type":"E","message":ch.repeat(size),"traceback":ch.repeat(size)}),
                    ] {
                        let extra = serde_json::json!({"error":error,"state":{"persisted":size % 2 == 0,"restored":true}});
                        let text = render(
                            &ch.repeat(size),
                            &ch.repeat(size * 2),
                            &extra,
                            "Background runs are not available in this chat.\n",
                            &notes,
                            cap,
                        );
                        assert!(
                            text.chars().count() <= cap,
                            "cap {cap}, size {size}, {ch}: {}",
                            text.chars().count()
                        );
                    }
                }
            }
        }
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
