import { describe, expect, test } from "vitest";
import {
  agentStatusTone,
  extractReviewReport,
  severityTone,
  tierLabel,
  tierOrder,
} from "./reviewReportJson";

function fenced(value: unknown): string {
  return `\`\`\`json\n${JSON.stringify(value)}\n\`\`\``;
}

describe("extractReviewReport", () => {
  test("extracts the last trailing json block", () => {
    const content = [
      "Earlier data",
      fenced({ unrelated: true }),
      "Review result",
      fenced({
        scope: { files_reviewed: ["src/a.ts"], focus: "safety" },
        findings: [{ claim: "A race is possible", rank_tier: "verified" }],
        summary: "One issue",
      }),
      "  ",
    ].join("\n");

    const report = extractReviewReport(content);

    expect(report?.summary).toBe("One issue");
    expect(report?.findings[0].claim).toBe("A race is possible");
    expect(report?.scope.files_reviewed).toEqual(["src/a.ts"]);
  });

  test.each([
    ["missing block", "plain text"],
    ["broken json", "```json\n{broken\n```"],
    ["non-report json", fenced({ value: true })],
  ])("returns null for %s", (_label, content) => {
    expect(extractReviewReport(content)).toBeNull();
  });

  test("normalizes missing optional fields", () => {
    const report = extractReviewReport(fenced({ scope: {}, findings: [{}] }));

    expect(report).not.toBeNull();
    expect(report?.scope).toEqual({
      files_reviewed: [],
      focus: null,
      diff_base: null,
    });
    expect(report?.checks_performed).toEqual([]);
    expect(report?.assumed_intent).toBeNull();
    expect(report?.pipeline).toBeNull();
    expect(report?.findings[0]).toMatchObject({
      category: "uncategorized",
      severity: "low",
      rank_tier: "unverified",
      sources: [],
      evidence: [],
      impact: null,
      remediation: null,
      checks_performed: [],
    });
  });
});

describe("review report helpers", () => {
  test("maps severities to badge tones", () => {
    expect(severityTone("critical")).toBe("danger");
    expect(severityTone("high")).toBe("danger");
    expect(severityTone("medium")).toBe("warning");
    expect(severityTone("low")).toBe("muted");
  });

  test("labels tiers in display order", () => {
    expect(tierOrder).toEqual([
      "execution_reproduced",
      "corroborated",
      "verified",
      "needs_human_validation",
      "unverified",
      "downgraded",
    ]);
    expect(tierLabel("execution_reproduced")).toBe("execution-reproduced");
    expect(tierLabel("needs_human_validation")).toBe("needs human validation");
    expect(tierLabel("corroborated")).toBe("corroborated");
  });

  test("maps agent statuses to badge tones", () => {
    expect(agentStatusTone("ran")).toBe("success");
    expect(agentStatusTone("skipped")).toBe("muted");
    expect(agentStatusTone("failed")).toBe("danger");
  });
});
