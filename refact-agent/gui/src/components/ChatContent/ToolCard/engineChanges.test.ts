import { describe, expect, it } from "vitest";
import {
  ENGINE_CLASS_LABEL,
  engineCallClass,
  engineChangeCount,
  engineValueView,
  engineWarning,
  hasNoValue,
  textPairHunks,
  isTextPair,
} from "./engineChanges";
import {
  engineCall,
  engineChange,
  unknownOutcomeChange,
} from "./engineFixtures";
import {
  normalizeEngineChanges,
  normalizePythonRunExtra,
} from "../../../services/refact/types";
import { flattenedPythonToolMessage } from "../../../__tests__/pythonWireFixtures";

describe("engine change and result guards", () => {
  it("keeps an unknown outcome, drops any other value and keeps the entry", () => {
    const wire = (outcome: unknown) => ({ ...engineChange(2), outcome });
    expect(normalizeEngineChanges([wire("unknown")])).toEqual([
      unknownOutcomeChange(2),
    ]);
    for (const outcome of ["applied", "UNKNOWN", true, 1, null, {}])
      expect(normalizeEngineChanges([wire(outcome)])).toEqual([
        engineChange(2),
      ]);
    expect(normalizeEngineChanges([engineChange(3)])[0]).not.toHaveProperty(
      "outcome",
    );
  });
  it("reads result_truncated only when it is true", () => {
    const run = (value: unknown) =>
      normalizePythonRunExtra({
        ...flattenedPythonToolMessage.python,
        result_truncated: value,
      });
    expect(run(true)?.result_truncated).toBe(true);
    for (const value of [false, "true", 1, null, {}]) {
      const extra = run(value);
      expect(extra).toBeDefined();
      expect(extra).not.toHaveProperty("result_truncated");
    }
    expect(
      normalizePythonRunExtra(flattenedPythonToolMessage.python),
    ).not.toHaveProperty("result_truncated");
  });
});

describe("engine value rules", () => {
  it("renders scalars and short JSON inline, absent sides in words", () => {
    expect(engineValueView(25, "old")).toEqual({ kind: "inline", text: "25" });
    expect(engineValueView("gpt-5", "new")).toEqual({
      kind: "inline",
      text: '"gpt-5"',
    });
    expect(engineValueView({ a: 1 }, "new")).toEqual({
      kind: "inline",
      text: '{ "a": 1 }',
    });
    expect(engineValueView(undefined, "old").text).toBe("not set");
    expect(engineValueView(undefined, "new").text).toBe("removed");
    expect(engineValueView(null, "new")).toEqual({
      kind: "inline",
      text: "null",
    });
  });
  it("turns multi-line or long values into blocks and long string pairs into a diff", () => {
    expect(engineValueView("a\nb", "old").kind).toBe("block");
    expect(engineValueView("x".repeat(81), "old").kind).toBe("block");
    expect(engineValueView({ list: Array(40).fill("item") }, "old").kind).toBe(
      "block",
    );
    expect(isTextPair({ label: "p", old: "a\nb", new: "a\nc" })).toBe(true);
    expect(isTextPair({ label: "p", old: "a", new: "b" })).toBe(false);
    expect(isTextPair({ label: "p", old: "a\nb" })).toBe(false);
    expect(
      isTextPair({ label: "p", old: "a\nb", new: "a\nc", truncated: true }),
    ).toBe(false);
    expect(hasNoValue({ label: "k" })).toBe(true);
    expect(hasNoValue({ label: "k", old: 1 })).toBe(false);
  });
  it("adds the undo sentence to destructive warnings only when missing", () => {
    expect(engineWarning("change", "x")).toBeUndefined();
    expect(engineWarning("risky", undefined)).toMatch(/safety boundary/);
    expect(engineWarning("destructive", "Deletes the chat.")).toBe(
      "Deletes the chat. This cannot be undone.",
    );
    expect(engineWarning("destructive", "This is irreversible.")).toBe(
      "This is irreversible.",
    );
    expect(engineWarning("destructive", "It cannot be undone.")).toBe(
      "It cannot be undone.",
    );
    for (const warning of [
      "The data is deleted permanently.",
      "This is not permanent.",
      "This is not irreversible.",
    ])
      expect(engineWarning("destructive", warning)).toBe(
        `${warning} This cannot be undone.`,
      );
  });
  it("derives a call's kind from the applied change, else not applied, else reads, else nothing", () => {
    const changes = [engineChange(2, { class: "destructive" })];
    expect(
      engineCallClass(engineCall(2, "engine.chats.delete", 1), changes),
    ).toBe("destructive");
    expect(engineCallClass(engineCall(1, "engine.settings.get", 1), [])).toBe(
      "read",
    );
    for (const approval of ["denied", "cancelled"] as const)
      expect(
        engineCallClass(
          engineCall(3, "engine.settings.set", 1, { approval }),
          changes,
        ),
      ).toBe("not_applied");
    expect(
      engineCallClass(
        engineCall(2, "engine.chats.delete", 1, { approval: "denied" }),
        changes,
      ),
    ).toBe("destructive");
    expect(
      engineCallClass(
        engineCall(3, "engine.settings.set", 1, { approval: "allowed" }),
        changes,
      ),
    ).toBeUndefined();
    expect(
      engineCallClass(engineCall(4, "cat", 1, { approval: "denied" }), []),
    ).toBeUndefined();
    expect(engineCallClass(engineCall(4, "cat", 1), changes)).toBeUndefined();
    expect(ENGINE_CLASS_LABEL.not_applied).toBe("Change (not applied)");
  });
  it("counts changes from the result, or approved engine calls while live", () => {
    const calls = [
      engineCall(1, "engine.settings.get", 1),
      engineCall(2, "engine.settings.set", 1, { approval: "allowed" }),
      engineCall(3, "engine.settings.set", 1, {
        approval: "allowed",
        status: "running",
      }),
    ];
    expect(engineChangeCount(undefined, undefined, calls, false)).toBe(1);
    expect(engineChangeCount([engineChange(2)], 60, calls, true)).toBe(60);
    expect(engineChangeCount(undefined, undefined, calls, true)).toBe(0);
  });
  it("computes real line hunks for a text pair", () => {
    const before = Array.from({ length: 40 }, (_, i) => `line ${i}`).join("\n");
    const after = before.replace("line 20", "line twenty");
    const hunks = textPairHunks("prompt", before, after);
    expect(hunks).toHaveLength(1);
    expect(hunks[0].lines_remove).toBe(
      "line 18\nline 19\nline 20\nline 21\nline 22\n",
    );
    expect(hunks[0].lines_add).toBe(
      "line 18\nline 19\nline twenty\nline 21\nline 22\n",
    );
    expect(hunks[0].line1).toBe(19);
  });
  it("shows invisible format characters in values as markers", () => {
    expect(engineValueView("a\u202Eb", "new").text).toBe('"a⟨U+202E⟩b"');
    expect(engineValueView({ k: "\u200B" }, "new").text).toBe(
      '{ "k": "⟨U+200B⟩" }',
    );
  });
});
