import { describe, expect, test } from "vitest";
import {
  findPlaceholderRanges,
  nextPlaceholder,
  parseHintPlaceholders,
  placeholderAt,
  previousPlaceholder,
  selectionIsPlaceholder,
  stripUnfilledPlaceholders,
} from "./argumentPlaceholders";

describe("argumentPlaceholders", () => {
  test("parseHintPlaceholders extracts bracket groups", () => {
    expect(parseHintPlaceholders("<file-path>")).toEqual(["<file-path>"]);
    expect(parseHintPlaceholders("<from> <to>")).toEqual(["<from>", "<to>"]);
    expect(parseHintPlaceholders("[optional]")).toEqual(["[optional]"]);
    expect(parseHintPlaceholders("<req> [opt]")).toEqual(["<req>", "[opt]"]);
    expect(parseHintPlaceholders("")).toEqual([]);
  });

  test("findPlaceholderRanges returns ordered ranges", () => {
    const text = "/cmd <from> <to>";
    expect(findPlaceholderRanges(text)).toEqual([
      { start: 5, end: 11 },
      { start: 12, end: 16 },
    ]);
  });

  test("nextPlaceholder finds the next group at or after a position", () => {
    const text = "/cmd <from> <to>";
    expect(nextPlaceholder(text, 0)).toEqual({ start: 5, end: 11 });
    // From the end of the first placeholder it should skip to the second.
    expect(nextPlaceholder(text, 11)).toEqual({ start: 12, end: 16 });
    expect(nextPlaceholder(text, 12)).toEqual({ start: 12, end: 16 });
    expect(nextPlaceholder(text, 13)).toBeNull();
  });

  test("previousPlaceholder finds the prior group", () => {
    const text = "/cmd <from> <to>";
    expect(previousPlaceholder(text, 16)).toEqual({ start: 12, end: 16 });
    expect(previousPlaceholder(text, 12)).toEqual({ start: 5, end: 11 });
    expect(previousPlaceholder(text, 5)).toBeNull();
  });

  test("placeholderAt locates the group under a caret", () => {
    const text = "/cmd <from> <to>";
    expect(placeholderAt(text, 6)).toEqual({ start: 5, end: 11 });
    expect(placeholderAt(text, 5)).toEqual({ start: 5, end: 11 });
    expect(placeholderAt(text, 11)).toEqual({ start: 5, end: 11 });
    expect(placeholderAt(text, 4)).toBeNull();
  });

  test("selectionIsPlaceholder detects an exact selection", () => {
    const text = "/cmd <from> <to>";
    expect(selectionIsPlaceholder(text, 5, 11)).toBe(true);
    expect(selectionIsPlaceholder(text, 5, 10)).toBe(false);
    expect(selectionIsPlaceholder(text, 7, 7)).toBe(false);
  });

  describe("stripUnfilledPlaceholders", () => {
    test("removes a trailing placeholder with its leading space", () => {
      expect(
        stripUnfilledPlaceholders("/review <file-path>", ["<file-path>"]),
      ).toBe("/review");
    });

    test("removes the only placeholder leaving just the command", () => {
      expect(
        stripUnfilledPlaceholders("/optimize [file-path]", ["[file-path]"]),
      ).toBe("/optimize");
    });

    test("keeps neighbouring words separated", () => {
      expect(stripUnfilledPlaceholders("/cmd <a> rest", ["<a>"])).toBe(
        "/cmd rest",
      );
    });

    test("strips multiple placeholders", () => {
      expect(
        stripUnfilledPlaceholders("/cmd <from> <to>", ["<from>", "<to>"]),
      ).toBe("/cmd");
    });

    test("only strips a partially-filled set", () => {
      expect(
        stripUnfilledPlaceholders("/cmd foo <to>", ["<from>", "<to>"]),
      ).toBe("/cmd foo");
    });

    test("never touches lookalike user text", () => {
      const text = "implement Vec<T> and read arr[0] for /review";
      expect(stripUnfilledPlaceholders(text, ["<file-path>"])).toBe(text);
    });

    test("leaves filled-in values intact", () => {
      expect(
        stripUnfilledPlaceholders("/review src/app.ts", ["<file-path>"]),
      ).toBe("/review src/app.ts");
    });
  });
});
