import { describe, it, expect } from "vitest";
import {
  parseLine,
  parseLines,
  parseLineRange,
  formatLineRange,
} from "./parseAtCommands";

describe("parseLineRange", () => {
  it("parses single line", () => {
    const result = parseLineRange("file.ts:10");
    expect(result.path).toBe("file.ts");
    expect(result.lineRange).toEqual({ line1: 10, kind: "single" });
  });

  it("parses range", () => {
    const result = parseLineRange("file.ts:10-20");
    expect(result.path).toBe("file.ts");
    expect(result.lineRange).toEqual({ line1: 10, line2: 20, kind: "range" });
  });

  it("parses to-end range", () => {
    const result = parseLineRange("file.ts:10-");
    expect(result.path).toBe("file.ts");
    expect(result.lineRange).toEqual({ line1: 10, kind: "to-end" });
  });

  it("parses from-start range", () => {
    const result = parseLineRange("file.ts:-20");
    expect(result.path).toBe("file.ts");
    expect(result.lineRange).toEqual({
      line1: 1,
      line2: 20,
      kind: "from-start",
    });
  });

  it("returns path only when no range", () => {
    const result = parseLineRange("file.ts");
    expect(result.path).toBe("file.ts");
    expect(result.lineRange).toBeUndefined();
  });
});

describe("formatLineRange", () => {
  it("formats single line", () => {
    expect(formatLineRange({ line1: 10, kind: "single" })).toBe(":10");
  });

  it("formats range", () => {
    expect(formatLineRange({ line1: 10, line2: 20, kind: "range" })).toBe(
      ":10-20",
    );
  });
});

describe("parseLine", () => {
  it("parses simple @file command", () => {
    const result = parseLine("@file src/main.rs");
    expect(result.tokens).toHaveLength(1);
    expect(result.tokens[0]).toMatchObject({
      kind: "at",
      type: "file",
      arg: "src/main.rs",
    });
  });

  it("parses @file with line range", () => {
    const result = parseLine("@file src/main.rs:10-20");
    expect(result.tokens[0]).toMatchObject({
      kind: "at",
      type: "file",
      arg: "src/main.rs",
      lineRange: { line1: 10, line2: 20, kind: "range" },
    });
  });

  it("parses @web command", () => {
    const result = parseLine("@web https://example.com");
    expect(result.tokens[0]).toMatchObject({
      kind: "at",
      type: "web",
      arg: "https://example.com",
    });
  });

  it("parses mixed text and commands", () => {
    const result = parseLine("check @file main.rs @web docs.rs");
    const atTokens = result.tokens.filter((t) => t.kind === "at");
    expect(atTokens).toHaveLength(2);
    expect(atTokens[0]).toMatchObject({
      kind: "at",
      type: "file",
      arg: "main.rs",
    });
    expect(atTokens[1]).toMatchObject({
      kind: "at",
      type: "web",
      arg: "docs.rs",
    });
  });

  it("handles trailing punctuation", () => {
    const result = parseLine("@file main.rs, @web example.com!");
    expect(result.tokens[0]).toMatchObject({ kind: "at", arg: "main.rs" });
    expect(result.tokens[2]).toMatchObject({ kind: "at", arg: "example.com" });
  });

  it("parses @tree without args", () => {
    const result = parseLine("@tree");
    expect(result.tokens[0]).toMatchObject({
      kind: "at",
      type: "tree",
      arg: undefined,
    });
  });

  it("parses @search with multiple words", () => {
    const result = parseLine("@search auth bug fix");
    expect(result.tokens[0]).toMatchObject({
      kind: "at",
      type: "search",
      arg: "auth bug fix",
    });
  });
});

describe("parseLines", () => {
  it("skips parsing inside code fences", () => {
    const text = "before\n```\n@file inside.rs\n```\nafter @file outside.rs";
    const result = parseLines(text);

    expect(result[2].tokens[0]).toMatchObject({
      kind: "text",
      text: "@file inside.rs",
    });
    expect(result[4].tokens[1]).toMatchObject({ kind: "at", type: "file" });
  });

  it("handles multiple code fences", () => {
    const text = "```\ncode1\n```\n@file test.rs\n```\ncode2\n```";
    const result = parseLines(text);

    expect(result[3].tokens[0]).toMatchObject({ kind: "at", type: "file" });
  });
});
