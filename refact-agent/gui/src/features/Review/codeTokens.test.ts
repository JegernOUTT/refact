import { describe, expect, it } from "vitest";

import { codeTokens, lineCommentOf } from "./codeTokens";

const text = (line: string, comment: string) =>
  codeTokens(line, comment)
    .map(([, part]) => part)
    .join("");

describe("code tint", () => {
  it("picks the comment marker from the file name", () => {
    expect(lineCommentOf("src/a.rs")).toBe("//");
    expect(lineCommentOf("tools/run.py")).toBe("#");
    expect(lineCommentOf("Dockerfile")).toBe("#");
    expect(lineCommentOf("db/schema.sql")).toBe("--");
  });

  it("splits keywords, strings, numbers and trailing comments", () => {
    expect(codeTokens('let x = "a // b"; // why 42', "//")).toEqual([
      ["k", "let"],
      ["", " x = "],
      ["s", '"a // b"'],
      ["", "; "],
      ["c", "// why 42"],
    ]);
    expect(codeTokens("return 0x1f + 2.5e3", "//")).toEqual([
      ["k", "return"],
      ["", " "],
      ["n", "0x1f"],
      ["", " + "],
      ["n", "2.5e3"],
    ]);
    expect(codeTokens("name = 'it''s' # note", "#")).toEqual([
      ["", "name = "],
      ["s", "'it''s'"],
      ["", " "],
      ["c", "# note"],
    ]);
  });

  it("leaves lifetimes alone and treats doc-comment bodies as comments", () => {
    expect(codeTokens("fn f<'a>(x: &'a str) -> char { 'x' }", "//")).toEqual([
      ["k", "fn"],
      ["", " f<'a>(x: &'a str) -> char { "],
      ["s", "'x'"],
      ["", " }"],
    ]);
    expect(codeTokens("   * @param x the value", "//")).toEqual([
      ["c", "   * @param x the value"],
    ]);
  });

  it("never alters the text it is given", () => {
    for (const [line, comment] of [
      ['weird "unterminated string', "//"],
      ["x".repeat(500), "//"],
      ["  # only a comment", "#"],
      ["a -- b", "--"],
      ["", "//"],
    ] as const) {
      expect(text(line, comment)).toBe(line);
    }
  });
});
