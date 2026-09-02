import { describe, expect, test } from "vitest";
import {
  extractReviewReport,
  isHypothesis,
  parseReviewReport,
  reviewReportFromExtra,
  severityOrder,
  severityTone,
  stageStatusLabel,
  stageStatusTone,
  type ReviewFinding,
} from "./reviewReportJson";

function reportValue(overrides: Record<string, unknown> = {}) {
  return {
    depth: "deep",
    scope: {
      mode: "strict",
      requested_files: 2,
      reviewed_files: 3,
      files: ["src/a.ts"],
      focus: "safety",
      expansion: "+1 dependency edges",
      out_of_scope_findings: 1,
    },
    diff: { base: "abc123", head: "HEAD", changed_files: 2, hunks: 9 },
    stages: [
      {
        name: "diff",
        model: "model-a",
        status: "ok",
        duration_ms: 4000,
        findings: 1,
        summary: "read five files",
        coverage: {
          files_read: ["src/a.ts"],
          commands_run: [{ cmd: "cargo check", exit: 0 }],
          tools_unavailable: [],
          stopped_early: null,
        },
      },
      { name: "tests", status: "timed_out", reason: "stage budget" },
    ],
    findings: [
      {
        id: "rf-1",
        stage: "diff",
        title: "Race",
        severity: "high",
        file: "src/a.ts",
        line_start: 42,
        line_end: 46,
        claim: "A race is possible",
        evidence: "writerA();",
        evidence_present: true,
        reproduction: "npm test -- cache",
        fix: "Serialize updates",
        introduced_by_diff: true,
        out_of_scope: false,
        reported_by: ["diff", "impact"],
        locations: [{ file: "src/b.ts", line_start: 4, line_end: 6 }],
      },
    ],
    duration_ms: 252000,
    duplicates_merged: 2,
    scratch_dir: ".refact/review_scratch/rv-1",
    ...overrides,
  };
}

function fenced(value: unknown): string {
  return `\`\`\`json\n${JSON.stringify(value)}\n\`\`\``;
}

function finding(overrides: Partial<ReviewFinding> = {}): ReviewFinding {
  const parsed = parseReviewReport(reportValue());
  if (!parsed) throw new Error("expected the fixture report to parse");
  return { ...parsed.findings[0], ...overrides };
}

describe("parseReviewReport", () => {
  test("parses the full report shape", () => {
    const report = parseReviewReport(reportValue());

    expect(report?.depth).toBe("deep");
    expect(report?.scope.mode).toBe("strict");
    expect(report?.scope.expansion).toBe("+1 dependency edges");
    expect(report?.diff.hunks).toBe(9);
    expect(report?.stages).toHaveLength(2);
    expect(report?.stages[0].coverage.commands_run[0].cmd).toBe("cargo check");
    expect(report?.stages[1].status).toBe("timed_out");
    expect(report?.stages[1].coverage.files_read).toEqual([]);
    expect(report?.findings[0].reported_by).toEqual(["diff", "impact"]);
    expect(report?.findings[0].locations[0].file).toBe("src/b.ts");
    expect(report?.duplicates_merged).toBe(2);
  });

  test.each([
    ["not an object", "nope"],
    ["missing findings", { scope: {}, stages: [] }],
    ["missing stages", { scope: {}, findings: [] }],
  ])("returns null for %s", (_label, value) => {
    expect(parseReviewReport(value)).toBeNull();
  });

  test("normalizes missing optional fields", () => {
    const report = parseReviewReport({ scope: {}, stages: [], findings: [{}] });

    expect(report?.findings[0]).toMatchObject({
      stage: "unknown",
      severity: "low",
      line_start: 1,
      line_end: 1,
      evidence_present: false,
      reproduction: null,
      fix: null,
      out_of_scope: false,
      reported_by: [],
      locations: [],
      disputed: null,
    });
    expect(report?.scope.mode).toBe("broad");
    expect(report?.diff.base).toBeNull();
  });

  test("reads the report from tool result metering", () => {
    expect(reviewReportFromExtra({ review_report: reportValue() })?.depth).toBe(
      "deep",
    );
    expect(reviewReportFromExtra(undefined)).toBeNull();
    expect(reviewReportFromExtra({ other: 1 })).toBeNull();
  });

  test("still reads a fenced json block when one is present", () => {
    const content = ["Review result", fenced(reportValue())].join("\n");

    expect(extractReviewReport(content)?.findings).toHaveLength(1);
    expect(extractReviewReport("plain text")).toBeNull();
    expect(extractReviewReport("```json\n{broken\n```")).toBeNull();
  });
});

describe("review report helpers", () => {
  test("a finding is a hypothesis without reproduction and without evidence", () => {
    expect(isHypothesis(finding())).toBe(false);
    expect(isHypothesis(finding({ reproduction: null }))).toBe(false);
    expect(isHypothesis(finding({ evidence_present: false }))).toBe(false);
    expect(
      isHypothesis(finding({ reproduction: null, evidence_present: false })),
    ).toBe(true);
    expect(
      isHypothesis(
        finding({ disputed: { stage: "adversarial", reason: "x" } }),
      ),
    ).toBe(true);
  });

  test("maps severities to badge tones in display order", () => {
    expect(severityOrder).toEqual(["blocker", "high", "medium", "low", "note"]);
    expect(severityTone("blocker")).toBe("danger");
    expect(severityTone("high")).toBe("danger");
    expect(severityTone("medium")).toBe("warning");
    expect(severityTone("low")).toBe("muted");
    expect(severityTone("note")).toBe("muted");
  });

  test("maps stage statuses to tones and labels", () => {
    expect(stageStatusTone("ok")).toBe("success");
    expect(stageStatusTone("timed_out")).toBe("warning");
    expect(stageStatusTone("failed")).toBe("danger");
    expect(stageStatusTone("not_run")).toBe("muted");
    expect(stageStatusLabel("timed_out")).toBe("timed out");
    expect(stageStatusLabel("not_run")).toBe("not run");
    expect(stageStatusLabel("ok")).toBe("ok");
  });
});
