import { describe, expect, test } from "vitest";
import {
  evidenceVerdicts,
  goalEvidenceVerdict,
  parseEvidenceVerdict,
} from "./evidenceTypes";

describe("Evidence verdict contract", () => {
  test("names every verdict the engine can report, with no synonym", () => {
    expect([...evidenceVerdicts]).toEqual([
      "not_run",
      "passed",
      "failed",
      "blocked",
      "unavailable",
      "superseded",
    ]);
  });

  test("rejects synonyms, wrong case and non-strings rather than guessing", () => {
    for (const value of [
      "ok",
      "met",
      "unmet",
      "success",
      "PASSED",
      "pass",
      "",
      1,
      null,
      undefined,
      {},
      ["passed"],
    ]) {
      expect(parseEvidenceVerdict(value), String(value)).toBeNull();
    }
  });

  test("reads only the verdict the receipt carries as a value", () => {
    expect(
      goalEvidenceVerdict({ validate_goal: { evidence_verdict: "blocked" } }),
    ).toBe("blocked");
    expect(
      goalEvidenceVerdict({ validate_goal: { verdict: "met" } }),
    ).toBeNull();
    expect(goalEvidenceVerdict({ validate_goal: [] })).toBeNull();
    expect(goalEvidenceVerdict({ evidence_verdict: "passed" })).toBeNull();
    expect(goalEvidenceVerdict(null)).toBeNull();
    expect(goalEvidenceVerdict("passed")).toBeNull();
  });
});
