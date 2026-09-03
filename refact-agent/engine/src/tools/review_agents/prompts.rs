use crate::tools::review_agents::stages::{StageContract, StageSpec};
use crate::tools::review_scope::ReviewScope;
use crate::tools::review_types::ReviewFinding;

const MAX_LISTED_FILES: usize = 400;
const MAX_LISTED_FINDINGS: usize = 60;

fn diff_char_cap() -> usize {
    crate::runtime_settings::current().review_diff_char_cap
}

pub const REVIEW_SYSTEM_PROMPT: &str = r#"You are one stage of a code review. You investigate with tools and report what you can prove.

Rules that override anything else:
- NO_FINDING is a valid, frequent and respected result. Never invent a finding to look useful.
- Every finding must quote the exact source lines it is about, copied verbatim from the file. A finding whose quote cannot be found in the file is discarded by the pipeline.
- Report facts, not impressions. If you did not read the code that would refute your claim, read it before filing.
- reproduction is a command you actually ran, a test that actually exists, or a call trace through functions you actually read. If you have none of those, set it to null - that is expected and honest, not a failure.
- Do not report on generated files, lockfiles, snapshots, fixtures, vendored code or build output unless the change alters the generator or its configuration.
- Do not report style, naming or formatting preferences. Linters and formatters own those.
- Stay inside your stage's job. Another stage is covering the rest.
- Work fast and stop when you have checked what you were asked to check. Depth beats breadth.

You are running with tools enabled and confirmations disabled. Use them: read files, run commands, follow references. An answer that only reasons about code you did not open is worthless."#;

const FINDINGS_CONTRACT: &str = r#"# Output contract

Your FINAL message must contain exactly one fenced json block and nothing that matters outside it:

```json
{"stage":"<stage id>",
 "findings":[{"title":"<short label>",
   "severity":"blocker|high|medium|low|note",
   "file":"<path as given in scope>",
   "line_start":10,
   "line_end":14,
   "claim":"<one falsifiable sentence>",
   "evidence":"<verbatim lines copied from the file or verbatim tool output>",
   "reproduction":"<command, test name, or call trace> or null",
   "fix":"<the concrete change to make>"}],
 "summary":"<what you checked and what you could not check>",
 "coverage":{"files_read":["<path>"],
   "commands_run":[{"cmd":"<command>","exit":0}],
   "tools_unavailable":["<tool or command you could not use>"],
   "stopped_early":"<reason> or null"}}
```

Use an empty findings array when you found nothing. Fill coverage honestly: it is how the caller
knows what this review did and did not look at."#;

const VERDICTS_CONTRACT: &str = r#"# Output contract

Your FINAL message must contain exactly one fenced json block and nothing that matters outside it:

```json
{"stage":"adversarial",
 "verdicts":[{"id":"<finding id>","verdict":"supported|unsupported","reason":"<one sentence naming the file and line that settles it>"}],
 "summary":"<how you checked>",
 "coverage":{"files_read":["<path>"],"commands_run":[{"cmd":"<command>","exit":0}],"tools_unavailable":[],"stopped_early":null}}
```

Return exactly one verdict per finding id you were given, and no ids that were not given to you."#;

fn push_file_list(prompt: &mut String, heading: &str, files: &[String]) {
    if files.is_empty() {
        return;
    }
    prompt.push_str(&format!("\n# {heading} ({})\n", files.len()));
    for file in files.iter().take(MAX_LISTED_FILES) {
        prompt.push_str(&format!("- {file}\n"));
    }
    if files.len() > MAX_LISTED_FILES {
        prompt.push_str(&format!(
            "⚠️ showing {MAX_LISTED_FILES} of {} paths; the remaining {} are not listed here.\n",
            files.len(),
            files.len() - MAX_LISTED_FILES
        ));
    }
}

fn file_boundary_before(text: &str, cap: usize) -> usize {
    let mut end = cap.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = &text[..end];
    match head.rfind("\ndiff --git ") {
        Some(index) => index + 1,
        None => end,
    }
}

fn truncate_diff(text: &str, cap: usize, total_bytes: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let end = file_boundary_before(text, cap);
    let total = total_bytes.max(text.len());
    format!(
        "{}\n⚠️ showing {} of {} bytes (limit: review_diff_char_cap = {}). 💡 Raise review_diff_char_cap in trajectory settings to hand the whole diff to every review stage; the omitted files are listed above and you can read them with cat or git diff.",
        &text[..end],
        end,
        total,
        cap
    )
}

fn scope_block(scope: &ReviewScope) -> String {
    let mut prompt = String::new();
    if let Some(focus) = scope.focus.as_deref() {
        prompt.push_str(&format!("# What the caller asked for\n{focus}\n"));
    }
    if let Some(plan) = scope.plan.as_deref() {
        prompt.push_str(&format!("\n# Plan / acceptance criteria\n{plan}\n"));
    }
    push_file_list(&mut prompt, "Files in review scope", &scope.file_strings());
    let changed: Vec<String> = scope
        .changed_files
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect();
    push_file_list(&mut prompt, "Files changed by the diff", &changed);
    push_file_list(
        &mut prompt,
        "Files NOT reviewed (dropped by the max_files cap)",
        &scope.dropped_file_strings(),
    );
    if let Some(patch) = scope.diff_patch.as_deref() {
        let base = scope.base.as_deref().unwrap_or("unknown");
        prompt.push_str(&format!(
            "\n# Diff under review (base {base})\n```diff\n{}\n```\n",
            truncate_diff(patch, diff_char_cap(), scope.patch_total_bytes)
        ));
    } else {
        prompt.push_str(
            "\n# Diff under review\nNo diff is available; review the files in scope as they stand.\n",
        );
    }
    prompt.push_str(&format!(
        "\n# Scope policy\nScope mode is {}. ",
        scope.mode.as_str()
    ));
    prompt.push_str(match scope.mode {
        crate::tools::review_types::ScopeMode::Strict => "Report only on the files listed above. You may read anything to understand them.",
        crate::tools::review_types::ScopeMode::Adjacent => "Report on the files listed above and their direct dependents. You may read anything to understand them.",
        crate::tools::review_types::ScopeMode::Broad => "You may report on any file you can justify from the change.",
    });
    prompt.push('\n');
    prompt
}

pub fn build_stage_prompt(spec: &StageSpec, scope: &ReviewScope, scenario: Option<&str>) -> String {
    let mut prompt = format!("# Your job this run: {}\n{}\n", spec.id, spec.task.trim());
    if let Some(scenario) = scenario.map(str::trim).filter(|value| !value.is_empty()) {
        prompt.push_str(&format!(
            "\n# Scenario the caller requires you to run\nThis is authoritative. Drive exactly this and report what you observed; do not substitute a scenario of your own.\n{scenario}\n"
        ));
    }
    if let Some(fallback) = spec
        .fallback
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
    {
        prompt.push_str(&format!(
            "\n# If a preferred tool is unavailable\n{fallback}\n"
        ));
    }
    if !spec.preferred_tools.is_empty() {
        prompt.push_str(&format!(
            "\n# Preferred tools for this stage\n{}\n",
            spec.preferred_tools.join(", ")
        ));
    }
    prompt.push('\n');
    prompt.push_str(&scope_block(scope));
    prompt.push('\n');
    prompt.push_str(match spec.contract {
        StageContract::Findings => FINDINGS_CONTRACT,
        StageContract::Verdicts => VERDICTS_CONTRACT,
    });
    prompt.push('\n');
    prompt
}

pub fn build_adversarial_prompt(
    spec: &StageSpec,
    scope: &ReviewScope,
    findings: &[ReviewFinding],
) -> String {
    let mut prompt = format!("# Your job this run: {}\n{}\n", spec.id, spec.task.trim());
    prompt.push('\n');
    prompt.push_str(&scope_block(scope));
    prompt.push_str("\n# Findings to check\n");
    for finding in findings.iter().take(MAX_LISTED_FINDINGS) {
        prompt.push_str(&format!(
            "\n## {} — {}:{}-{}\n- stage: {}\n- severity: {}\n- claim: {}\n- evidence quoted by that stage:\n```\n{}\n```\n- reproduction: {}\n",
            finding.id,
            finding.file,
            finding.line_start,
            finding.line_end,
            finding.stage,
            finding.severity.as_str(),
            finding.claim,
            finding.evidence.trim(),
            finding.reproduction.as_deref().unwrap_or("none given"),
        ));
    }
    if findings.len() > MAX_LISTED_FINDINGS {
        prompt.push_str(&format!(
            "\n… and {} more findings that you should return as supported without checking.\n",
            findings.len() - MAX_LISTED_FINDINGS
        ));
    }
    prompt.push('\n');
    prompt.push_str(VERDICTS_CONTRACT);
    prompt.push('\n');
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::review_agents::stages::embedded_catalog;
    use crate::tools::review_scope::DiffHunks;
    use crate::tools::review_types::{ReviewSeverity, ScopeMode};
    use std::path::PathBuf;

    fn scope(mode: ScopeMode) -> ReviewScope {
        ReviewScope {
            mode,
            requested: vec![PathBuf::from("src/lib.rs")],
            files: vec![PathBuf::from("src/lib.rs")],
            dropped_files: vec![],
            changed_files: vec![PathBuf::from("src/lib.rs")],
            focus: Some("browser lifecycle".to_string()),
            plan: Some("must never leak a profile".to_string()),
            base: Some("abc123".to_string()),
            head: Some("def456".to_string()),
            diff_patch: Some("@@ -1,2 +1,3 @@\n+let value = 1;\n".to_string()),
            patch_total_bytes: 0,
            hunks: DiffHunks::default(),
            repo_root: None,
            expansion: None,
        }
    }

    fn stage(id: &str) -> StageSpec {
        embedded_catalog()
            .into_iter()
            .find(|spec| spec.id == id)
            .unwrap()
    }

    fn finding() -> ReviewFinding {
        ReviewFinding {
            id: "rf-1234abcd".to_string(),
            stage: "diff".to_string(),
            model: None,
            title: "Error dropped".to_string(),
            severity: ReviewSeverity::High,
            file: "src/lib.rs".to_string(),
            line_start: 10,
            line_end: 12,
            claim: "The error arm returns success.".to_string(),
            evidence: "return Ok(());".to_string(),
            evidence_present: true,
            reproduction: None,
            fix: None,
            introduced_by_diff: true,
            out_of_scope: false,
            reported_by: vec!["diff".to_string()],
            locations: vec![],
            disputed: None,
        }
    }

    #[test]
    fn review_prompt_carries_task_scope_diff_and_contract() {
        let prompt = build_stage_prompt(&stage("mechanical"), &scope(ScopeMode::Strict), None);

        assert!(prompt.contains("# Your job this run: mechanical"));
        assert!(prompt.contains("browser lifecycle"));
        assert!(prompt.contains("must never leak a profile"));
        assert!(prompt.contains("```diff"));
        assert!(prompt.contains("base abc123"));
        assert!(prompt.contains("Scope mode is strict"));
        assert!(prompt.contains("Report only on the files listed above"));
        assert!(prompt.contains("\"line_start\":10"));
        assert!(prompt.contains("commands_run"));
        assert!(!prompt.contains("confidence"));
    }

    #[test]
    fn review_prompt_uses_verdict_contract_for_the_adversarial_stage() {
        let prompt = build_adversarial_prompt(
            &stage("adversarial"),
            &scope(ScopeMode::Broad),
            &[finding()],
        );

        assert!(prompt.contains("rf-1234abcd"));
        assert!(prompt.contains("src/lib.rs:10-12"));
        assert!(prompt.contains("return Ok(());"));
        assert!(prompt.contains("reproduction: none given"));
        assert!(prompt.contains("\"verdict\":\"supported|unsupported\""));
        assert!(!prompt.contains("\"findings\":["));
    }

    #[test]
    fn review_prompt_truncates_a_huge_diff_on_a_file_boundary_and_quantifies_it() {
        let first = format!("diff --git a/a.rs b/a.rs\n{}", "+é\n".repeat(200));
        let second = format!("diff --git a/b.rs b/b.rs\n{}", "+x\n".repeat(200));
        let patch = format!("{first}{second}");
        let total = patch.len();

        let rendered = truncate_diff(&patch, first.len() + 20, total + 5_000);

        assert!(rendered.starts_with("diff --git a/a.rs b/a.rs"));
        assert!(!rendered.contains("diff --git a/b.rs b/b.rs"));
        assert!(rendered.contains(&format!(
            "⚠️ showing {} of {} bytes (limit: review_diff_char_cap = {})",
            first.len(),
            total + 5_000,
            first.len() + 20
        )));
        assert!(rendered.contains("💡 Raise review_diff_char_cap in trajectory settings"));
        assert!(!rendered.contains("[diff truncated]"));
    }

    #[test]
    fn review_prompt_keeps_a_diff_that_fits_under_the_cap_intact() {
        let patch = "diff --git a/a.rs b/a.rs\n+one\n";

        assert_eq!(truncate_diff(patch, 4_096, patch.len()), patch);
    }

    #[test]
    fn review_prompt_reads_the_diff_cap_from_the_runtime_setting() {
        assert_eq!(
            diff_char_cap(),
            crate::runtime_settings::current().review_diff_char_cap
        );
        assert!(diff_char_cap() >= 4_096);
    }

    #[test]
    fn review_prompt_names_the_files_that_were_dropped_from_scope() {
        let mut truncated = scope(ScopeMode::Strict);
        truncated.dropped_files = vec![
            PathBuf::from("src/dropped_a.rs"),
            PathBuf::from("src/dropped_b.rs"),
        ];

        let prompt = build_stage_prompt(&stage("diff"), &truncated, None);

        assert!(prompt.contains("# Files NOT reviewed (dropped by the max_files cap) (2)"));
        assert!(prompt.contains("- src/dropped_a.rs"));
        assert!(prompt.contains("- src/dropped_b.rs"));
    }

    #[test]
    fn review_prompt_makes_the_browser_scenario_authoritative() {
        let prompt = build_stage_prompt(
            &stage("browser"),
            &scope(ScopeMode::Broad),
            Some("  open /settings, toggle dark mode, expect no console errors  "),
        );

        assert!(prompt.contains("# Scenario the caller requires you to run"));
        assert!(prompt.contains("open /settings, toggle dark mode"));
        assert!(prompt.contains("do not substitute a scenario of your own"));
        assert!(
            !build_stage_prompt(&stage("browser"), &scope(ScopeMode::Broad), Some("   "))
                .contains("# Scenario the caller requires")
        );
    }

    #[test]
    fn review_system_prompt_states_the_no_finding_and_evidence_rules() {
        assert!(REVIEW_SYSTEM_PROMPT.contains("NO_FINDING is a valid"));
        assert!(REVIEW_SYSTEM_PROMPT.contains("quote the exact source lines"));
        assert!(REVIEW_SYSTEM_PROMPT.contains("set it to null"));
    }
}
