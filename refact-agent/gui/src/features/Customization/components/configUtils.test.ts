import { describe, it, expect } from "vitest";
import {
  applyPatch,
  applyPatches,
  isPlainObject,
  sanitizeObject,
  safeString,
  safeBoolean,
  safeNumber,
  safeArray,
  safeObject,
  safeMessageArray,
  safeSelectionRange,
  safeToolConfirmRules,
  parseIntSafe,
  parseFloatSafe,
  validateConfigId,
  extractSubagentExtra,
  computeExtraPatches,
  isMessageTemplate,
  isToolConfirmRule,
} from "./configUtils";

describe("applyPatch", () => {
  it("sets a top-level field", () => {
    const obj = { a: 1 };
    const result = applyPatch(obj, { path: ["b"], value: 2 });
    expect(result).toEqual({ a: 1, b: 2 });
  });

  it("sets a nested field", () => {
    const obj = { a: { b: 1 } };
    const result = applyPatch(obj, { path: ["a", "c"], value: 2 });
    expect(result).toEqual({ a: { b: 1, c: 2 } });
  });

  it("creates intermediate objects", () => {
    const obj = {};
    const result = applyPatch(obj, { path: ["a", "b", "c"], value: 1 });
    expect(result).toEqual({ a: { b: { c: 1 } } });
  });

  it("creates intermediate arrays for numeric keys", () => {
    const obj = {};
    const result = applyPatch(obj, { path: ["items", 0], value: "first" });
    expect(result).toEqual({ items: ["first"] });
  });

  it("deletes field when value is undefined", () => {
    const obj = { a: 1, b: 2 };
    const result = applyPatch(obj, { path: ["a"], value: undefined });
    expect(result).toEqual({ b: 2 });
  });

  it("blocks __proto__ in path", () => {
    const obj = { a: 1 };
    const result = applyPatch(obj, {
      path: ["__proto__", "polluted"],
      value: true,
    });
    expect(result).toEqual({ a: 1 });
    expect(Object.prototype).not.toHaveProperty("polluted");
  });

  it("blocks constructor in path", () => {
    const obj = { a: 1 };
    const result = applyPatch(obj, { path: ["constructor"], value: "bad" });
    expect(result).toEqual({ a: 1 });
  });

  it("blocks prototype in path", () => {
    const obj = { a: 1 };
    const result = applyPatch(obj, { path: ["prototype"], value: "bad" });
    expect(result).toEqual({ a: 1 });
  });

  it("handles array updates correctly", () => {
    const obj = { items: ["a", "b", "c"] };
    const result = applyPatch(obj, { path: ["items", 1], value: "updated" });
    expect(result).toEqual({ items: ["a", "updated", "c"] });
  });

  it("does not mutate original object", () => {
    const obj = { a: { b: 1 } };
    const result = applyPatch(obj, { path: ["a", "b"], value: 2 });
    expect(obj.a.b).toBe(1);
    expect((result.a as { b: number }).b).toBe(2);
  });
});

describe("applyPatches", () => {
  it("applies multiple patches in order", () => {
    const obj = { a: 1 };
    const result = applyPatches(obj, [
      { path: ["b"], value: 2 },
      { path: ["c"], value: 3 },
      { path: ["a"], value: 10 },
    ]);
    expect(result).toEqual({ a: 10, b: 2, c: 3 });
  });
});

describe("representative customization config patches", () => {
  it("patches a mode message template field", () => {
    const modeConfig = {
      id: "agent",
      messages: [{ role: "system", content: "old prompt" }],
    };

    expect(
      applyPatch(modeConfig, {
        path: ["messages", 0, "content"],
        value: "new prompt",
      }),
    ).toEqual({
      id: "agent",
      messages: [{ role: "system", content: "new prompt" }],
    });
  });

  it("patches subagent tool confirmation rules without changing siblings", () => {
    const subagentConfig = {
      id: "reviewer",
      title: "Reviewer",
      subchat: {
        tool_confirmation: [{ match: "cat", action: "auto" }],
      },
    };

    expect(
      applyPatch(subagentConfig, {
        path: ["subchat", "tool_confirmation", 1],
        value: { match: "shell", action: "ask" },
      }),
    ).toEqual({
      id: "reviewer",
      title: "Reviewer",
      subchat: {
        tool_confirmation: [
          { match: "cat", action: "auto" },
          { match: "shell", action: "ask" },
        ],
      },
    });
  });
});

describe("sanitizeObject", () => {
  it("removes dangerous keys", () => {
    const obj = { a: 1, __proto__: "bad", constructor: "bad" };
    const result = sanitizeObject(obj);
    expect(result).toEqual({ a: 1 });
  });

  it("sanitizes nested objects", () => {
    const obj = { a: { __proto__: "bad", b: 1 } };
    const result = sanitizeObject(obj) as Record<string, unknown>;
    expect(result).toEqual({ a: { b: 1 } });
  });

  it("sanitizes arrays", () => {
    const arr = [{ __proto__: "bad", a: 1 }, { b: 2 }];
    const result = sanitizeObject(arr);
    expect(result).toEqual([{ a: 1 }, { b: 2 }]);
  });

  it("passes through primitives", () => {
    expect(sanitizeObject("string")).toBe("string");
    expect(sanitizeObject(123)).toBe(123);
    expect(sanitizeObject(null)).toBe(null);
  });
});

describe("isPlainObject", () => {
  it("returns true for plain objects", () => {
    expect(isPlainObject({})).toBe(true);
    expect(isPlainObject({ a: 1 })).toBe(true);
  });

  it("returns false for arrays", () => {
    expect(isPlainObject([])).toBe(false);
  });

  it("returns false for null", () => {
    expect(isPlainObject(null)).toBe(false);
  });

  it("returns false for primitives", () => {
    expect(isPlainObject("string")).toBe(false);
    expect(isPlainObject(123)).toBe(false);
  });
});

describe("safe type guards", () => {
  describe("safeString", () => {
    it("returns string for string input", () => {
      expect(safeString("hello")).toBe("hello");
    });

    it("returns empty string for non-string", () => {
      expect(safeString(123)).toBe("");
      expect(safeString(null)).toBe("");
      expect(safeString(undefined)).toBe("");
      expect(safeString({})).toBe("");
    });
  });

  describe("safeBoolean", () => {
    it("returns boolean for boolean input", () => {
      expect(safeBoolean(true)).toBe(true);
      expect(safeBoolean(false)).toBe(false);
    });

    it("returns false for non-boolean", () => {
      expect(safeBoolean("true")).toBe(false);
      expect(safeBoolean(1)).toBe(false);
      expect(safeBoolean(null)).toBe(false);
    });
  });

  describe("safeNumber", () => {
    it("returns number for valid number", () => {
      expect(safeNumber(42)).toBe(42);
      expect(safeNumber(3.14)).toBe(3.14);
      expect(safeNumber(0)).toBe(0);
    });

    it("returns undefined for non-number", () => {
      expect(safeNumber("42")).toBeUndefined();
      expect(safeNumber(NaN)).toBeUndefined();
      expect(safeNumber(Infinity)).toBeUndefined();
      expect(safeNumber(null)).toBeUndefined();
    });
  });

  describe("safeArray", () => {
    it("filters array with guard", () => {
      const isNum = (v: unknown): v is number => typeof v === "number";
      expect(safeArray([1, "a", 2, null, 3], isNum)).toEqual([1, 2, 3]);
    });

    it("returns empty array for non-array", () => {
      const isNum = (v: unknown): v is number => typeof v === "number";
      expect(safeArray("not array", isNum)).toEqual([]);
      expect(safeArray(null, isNum)).toEqual([]);
    });
  });

  describe("safeObject", () => {
    it("returns object for plain object", () => {
      expect(safeObject({ a: 1 })).toEqual({ a: 1 });
    });

    it("returns empty object for non-object", () => {
      expect(safeObject(null)).toEqual({});
      expect(safeObject([])).toEqual({});
      expect(safeObject("string")).toEqual({});
    });
  });
});

describe("safeMessageArray", () => {
  it("filters valid messages", () => {
    const input = [
      { role: "user", content: "hello" },
      { role: 123, content: "bad" },
      { role: "assistant", content: "hi" },
      "not an object",
      { role: "system" },
    ];
    expect(safeMessageArray(input)).toEqual([
      { role: "user", content: "hello" },
      { role: "assistant", content: "hi" },
    ]);
  });

  it("returns empty array for non-array", () => {
    expect(safeMessageArray(null)).toEqual([]);
    expect(safeMessageArray("string")).toEqual([]);
  });
});

describe("safeSelectionRange", () => {
  it("returns tuple for valid range", () => {
    expect(safeSelectionRange([1, 100])).toEqual([1, 100]);
    expect(safeSelectionRange([0, 0])).toEqual([0, 0]);
  });

  it("returns null for invalid input", () => {
    expect(safeSelectionRange(null)).toBeNull();
    expect(safeSelectionRange([1])).toBeNull();
    expect(safeSelectionRange([1, 2, 3])).toBeNull();
    expect(safeSelectionRange(["a", "b"])).toBeNull();
    expect(safeSelectionRange([1, NaN])).toBeNull();
  });
});

describe("safeToolConfirmRules", () => {
  it("filters valid rules", () => {
    const input = [
      { match: "tree", action: "auto" },
      { match_pattern: "cat", action: "auto" },
      { match: "shell", action: "ask" },
      "not an object",
    ];
    expect(safeToolConfirmRules(input)).toEqual([
      { match: "tree", action: "auto" },
      { match: "shell", action: "ask" },
    ]);
  });

  it("returns empty array for non-array", () => {
    expect(safeToolConfirmRules(null)).toEqual([]);
  });
});

describe("parseIntSafe", () => {
  it("parses valid integers", () => {
    expect(parseIntSafe("42")).toBe(42);
    expect(parseIntSafe("0")).toBe(0);
    expect(parseIntSafe("-10")).toBe(-10);
  });

  it("returns undefined for invalid input", () => {
    expect(parseIntSafe("")).toBeUndefined();
    expect(parseIntSafe("abc")).toBeUndefined();
    expect(parseIntSafe("3.14")).toBe(3);
  });
});

describe("parseFloatSafe", () => {
  it("parses valid floats", () => {
    expect(parseFloatSafe("3.14")).toBe(3.14);
    expect(parseFloatSafe("42")).toBe(42);
    expect(parseFloatSafe("0.5")).toBe(0.5);
  });

  it("returns undefined for invalid input", () => {
    expect(parseFloatSafe("")).toBeUndefined();
    expect(parseFloatSafe("abc")).toBeUndefined();
  });
});

describe("validateConfigId", () => {
  it("returns null for valid IDs", () => {
    expect(validateConfigId("my_mode")).toBeNull();
    expect(validateConfigId("agent")).toBeNull();
    expect(validateConfigId("mode-123")).toBeNull();
    expect(validateConfigId("a")).toBeNull();
  });

  it("returns error for empty ID", () => {
    expect(validateConfigId("")).toBe("ID is required");
    expect(validateConfigId("   ")).toBe("ID is required");
  });

  it("returns error for path traversal", () => {
    expect(validateConfigId("../bad")).toBe("ID contains invalid characters");
    expect(validateConfigId("a/b")).toBe("ID contains invalid characters");
    expect(validateConfigId("a\\b")).toBe("ID contains invalid characters");
  });

  it("returns error for invalid characters", () => {
    expect(validateConfigId("MyMode")).toBe(
      "ID must contain only lowercase letters, digits, underscore, or hyphen",
    );
    expect(validateConfigId("my mode")).toBe(
      "ID must contain only lowercase letters, digits, underscore, or hyphen",
    );
    expect(validateConfigId("mode!")).toBe(
      "ID must contain only lowercase letters, digits, underscore, or hyphen",
    );
  });
});

describe("extractSubagentExtra", () => {
  it("extracts unknown keys", () => {
    const config = {
      id: "test",
      title: "Test",
      custom_field: "value",
      another: 123,
    };
    expect(extractSubagentExtra(config)).toEqual({
      custom_field: "value",
      another: 123,
    });
  });

  it("excludes known keys", () => {
    const config = {
      id: "test",
      title: "Test",
      description: "desc",
      tools: ["cat"],
      subchat: {},
    };
    expect(extractSubagentExtra(config)).toEqual({});
  });

  it("excludes dangerous keys", () => {
    const config = {
      id: "test",
      __proto__: "bad",
      custom: "ok",
    };
    expect(extractSubagentExtra(config)).toEqual({ custom: "ok" });
  });
});

describe("computeExtraPatches", () => {
  it("computes patches for added keys", () => {
    const oldExtra = {};
    const newExtra = { custom: "value" };
    expect(computeExtraPatches(oldExtra, newExtra)).toEqual([
      { path: ["custom"], value: "value" },
    ]);
  });

  it("computes patches for removed keys", () => {
    const oldExtra = { custom: "value" };
    const newExtra = {};
    expect(computeExtraPatches(oldExtra, newExtra)).toEqual([
      { path: ["custom"], value: undefined },
    ]);
  });

  it("computes patches for changed keys", () => {
    const oldExtra = { custom: "old" };
    const newExtra = { custom: "new" };
    expect(computeExtraPatches(oldExtra, newExtra)).toEqual([
      { path: ["custom"], value: "new" },
    ]);
  });

  it("ignores unchanged keys", () => {
    const oldExtra = { custom: "same" };
    const newExtra = { custom: "same" };
    expect(computeExtraPatches(oldExtra, newExtra)).toEqual([]);
  });
});

describe("isMessageTemplate", () => {
  it("returns true for valid message", () => {
    expect(isMessageTemplate({ role: "user", content: "hello" })).toBe(true);
  });

  it("returns false for invalid message", () => {
    expect(isMessageTemplate({ role: 123, content: "hello" })).toBe(false);
    expect(isMessageTemplate({ role: "user" })).toBe(false);
    expect(isMessageTemplate("string")).toBe(false);
    expect(isMessageTemplate(null)).toBe(false);
  });
});

describe("isToolConfirmRule", () => {
  it("returns true for valid rule", () => {
    expect(isToolConfirmRule({ match: "tree", action: "auto" })).toBe(true);
  });

  it("returns false for invalid rule", () => {
    expect(isToolConfirmRule({ match_pattern: "tree", action: "auto" })).toBe(
      false,
    );
    expect(isToolConfirmRule({ match: "tree" })).toBe(false);
    expect(isToolConfirmRule("string")).toBe(false);
  });
});
