import { describe, expect, test } from "vitest";

import { normalizeReadPaths } from "./readToolPaths";

describe("normalizeReadPaths", () => {
  test("splits comma-separated paths", () => {
    expect(normalizeReadPaths({ paths: "src/a.ts, src/b.ts" })).toEqual([
      "src/a.ts",
      "src/b.ts",
    ]);
  });

  test("accepts path arrays and ignores non-string entries", () => {
    expect(
      normalizeReadPaths({ paths: ["src/a.ts", 42, "src/b.ts, src/c.ts"] }),
    ).toEqual(["src/a.ts", "src/b.ts", "src/c.ts"]);
  });

  test("falls back to singular path", () => {
    expect(normalizeReadPaths({ path: "src/one.ts" })).toEqual(["src/one.ts"]);
  });

  test("ignores non-string path values", () => {
    expect(normalizeReadPaths({ paths: { file: "src/a.ts" } })).toEqual([]);
    expect(normalizeReadPaths({ paths: true })).toEqual([]);
  });
});
