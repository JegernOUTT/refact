import { describe, expect, it } from "vitest";
import {
  COMPACT_HEADLINE_CHARS,
  COMPACT_LABEL_CHARS,
  COMPACT_MAX_FACTS,
  COMPACT_VALUE_CHARS,
  compactFacts,
  compactText,
  compactTransportPresentation,
  readCompactRecord,
  readCompactText,
  type CompactAgenticInput,
  type CompactAgenticPresentation,
} from "./compactPresentation";

function fixturePresentation(
  input: CompactAgenticInput,
): CompactAgenticPresentation {
  const args = readCompactRecord(input.arguments);
  const result = readCompactRecord(input.result);
  const state =
    input.transportState === "succeeded" && result?.verified === true
      ? { stateLabel: "Verified", tone: "positive" as const }
      : compactTransportPresentation(input.transportState);
  return {
    headline:
      compactText(readCompactText(args, "task"), COMPACT_HEADLINE_CHARS) ??
      "Requested work",
    ...state,
    facts: compactFacts([{ label: "Count", value: result?.count }]),
  };
}

describe("compact presentation helpers", () => {
  it("normalizes whitespace without interpreting hostile text", () => {
    expect(compactText(" \n hello\t\u00a0世界 \r\n again ")).toBe(
      "hello 世界 again",
    );
    expect(
      compactText('<img src=x onerror="alert(1)"><script>bad()</script>'),
    ).toBe('<img src=x onerror="alert(1)"><script>bad()</script>');
  });

  it("bounds large Unicode text without splitting surrogate pairs", () => {
    const text = "🧪世界e\u0301".repeat(100_000);
    const clipped = compactText(text, 8);
    expect(clipped).toBe("🧪世界e\u0301🧪世…");
    expect(Array.from(clipped ?? "")).toHaveLength(8);
    expect(clipped).not.toMatch(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])/u);
    expect(Array.from(compactText(text, 1_000_000) ?? "")).toHaveLength(
      COMPACT_VALUE_CHARS,
    );
    expect(compactText("🧪🧪", 2)).toBe("🧪🧪");
    expect(compactText("🧪🧪", 1)).toBe("…");
  });

  it("preserves zero and false without inventing absent values or counts", () => {
    expect(compactText(0)).toBe("0");
    expect(compactText(false)).toBe("false");
    for (const value of [
      undefined,
      null,
      "",
      " \n\t ",
      [],
      {},
      NaN,
      Infinity,
    ]) {
      expect(compactText(value)).toBeNull();
    }
    for (const limit of [0, -1, NaN, Infinity]) {
      expect(compactText("value", limit)).toBeNull();
    }
  });

  it("reads only object payloads and tolerates absent or malformed arguments", () => {
    const args = { task: "  requested\nintent  ", count: 0 };
    expect(readCompactRecord(args)).toBe(args);
    expect(readCompactRecord(JSON.stringify(args))).toEqual(args);
    for (const value of [
      undefined,
      null,
      "",
      "{",
      "[]",
      "null",
      "true",
      [],
      5,
    ]) {
      expect(readCompactRecord(value)).toBeNull();
    }
    expect(readCompactText(readCompactRecord(args), "task")).toBe(
      "requested intent",
    );
    expect(readCompactText(readCompactRecord(args), "count")).toBe("0");
    expect(
      readCompactText(
        { first: [], second: " ", third: "yes" },
        "first",
        "second",
        "third",
      ),
    ).toBe("yes");
    expect(readCompactText(null, "task")).toBeNull();
    expect(readCompactText({}, "toString", "constructor")).toBeNull();
  });

  it("returns at most three bounded, present facts without mutating inputs", () => {
    const facts = Object.freeze([
      { label: "Missing", value: undefined },
      { label: " ", value: "No label" },
      { label: "Count", value: 0 },
      { label: "Label".repeat(30), value: "🧪".repeat(1_000) },
      { label: "Third", value: false },
      { label: "Fourth", value: "not in first expansion" },
    ]);
    const shown = compactFacts(facts);
    expect(shown).toHaveLength(COMPACT_MAX_FACTS);
    expect(shown[0]).toEqual({ label: "Count", value: "0" });
    expect(Array.from(shown[1].label)).toHaveLength(COMPACT_LABEL_CHARS);
    expect(Array.from(shown[1].value)).toHaveLength(COMPACT_VALUE_CHARS);
    expect(shown[2]).toEqual({ label: "Third", value: "false" });
    expect(facts).toHaveLength(6);
  });

  it.each([
    ["pending", "Awaiting result", "neutral"],
    ["running", "Running", "active"],
    ["succeeded", "Response received", "neutral"],
    ["failed", "Request failed", "danger"],
    ["unknown", "Outcome unknown", "neutral"],
  ] as const)(
    "keeps %s transport state scoped to the request",
    (state, stateLabel, tone) => {
      expect(compactTransportPresentation(state)).toEqual({ stateLabel, tone });
    },
  );

  it("lets recognized result evidence prove a fixture outcome, not intent or prose", () => {
    const base: CompactAgenticInput = {
      toolName: "fixture",
      arguments: { task: " Check\nwork ", verified: true },
      transportState: "succeeded",
    };
    for (const result of [
      undefined,
      "Verified, delivered, merged and successful!",
      { success: true },
      { status: "verified" },
      { note: "verified", count: null },
      { verified: "true" },
      "{broken",
    ]) {
      expect(fixturePresentation({ ...base, result })).toEqual({
        headline: "Check work",
        stateLabel: "Response received",
        tone: "neutral",
        facts: [],
      });
    }
    expect(
      fixturePresentation({ ...base, result: { verified: true, count: 0 } }),
    ).toEqual({
      headline: "Check work",
      stateLabel: "Verified",
      tone: "positive",
      facts: [{ label: "Count", value: "0" }],
    });
    expect(
      fixturePresentation({
        ...base,
        result: { verified: true },
        transportState: "failed",
      }).tone,
    ).toBe("danger");
    expect(
      fixturePresentation({
        toolName: "fixture",
        transportState: "unknown",
        arguments: "{",
      }).tone,
    ).toBe("neutral");
  });
});
