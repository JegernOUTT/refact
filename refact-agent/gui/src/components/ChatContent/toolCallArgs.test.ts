import { describe, expect, test } from "vitest";

import { toolCallArgsToString } from "./toolCallArgs";

describe("toolCallArgsToString", () => {
  test("formats object arguments", () => {
    expect(toolCallArgsToString('{"path":"src/a.ts","limit":2}')).toBe(
      'path="src/a.ts", limit=2',
    );
  });

  test("formats array arguments", () => {
    expect(toolCallArgsToString('["src/a.ts",2,true]')).toBe(
      '"src/a.ts", 2, true',
    );
  });

  test("formats primitive JSON without throwing", () => {
    expect(toolCallArgsToString('"plain"')).toBe('"plain"');
    expect(toolCallArgsToString("123")).toBe("123");
    expect(toolCallArgsToString("true")).toBe("true");
    expect(toolCallArgsToString("null")).toBe("null");
  });

  test("falls back to raw arguments for invalid JSON", () => {
    expect(toolCallArgsToString("not json")).toBe("not json");
  });
});
