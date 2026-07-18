import { afterEach, describe, expect, it, vi } from "vitest";

import {
  MAX_IDE_LOG_ENTRIES,
  clearIdeLogEntries,
  getIdeLogEntries,
  normalizeIdeLogLines,
  setIdeLogEntries,
  subscribeIdeLog,
} from "./ideLog";

describe("ideLog", () => {
  afterEach(() => {
    clearIdeLogEntries();
  });

  it("replaces entries, caps size, and notifies subscribers", () => {
    const listener = vi.fn();
    const unsubscribe = subscribeIdeLog(listener);
    setIdeLogEntries(
      Array.from({ length: MAX_IDE_LOG_ENTRIES + 2 }, (_, index) => ({
        level: "info",
        message: `line ${index}`,
      })),
    );
    expect(listener).toHaveBeenCalledTimes(1);
    const entries = getIdeLogEntries();
    expect(entries).toHaveLength(MAX_IDE_LOG_ENTRIES);
    expect(entries[0].message).toBe("line 2");
    unsubscribe();
  });

  it("clears entries", () => {
    setIdeLogEntries([{ level: "error", message: "boom" }]);
    clearIdeLogEntries();
    expect(getIdeLogEntries()).toEqual([]);
  });
});

describe("normalizeIdeLogLines", () => {
  it("normalizes object entries", () => {
    expect(
      normalizeIdeLogLines([
        { at: 123, level: "ERROR", message: "boom" },
        { level: "Warning", message: "careful" },
        { level: "TRACE", message: "trace as info" },
      ]),
    ).toEqual([
      { at: 123, level: "error", message: "boom" },
      { level: "info", message: "careful" },
      { level: "info", message: "trace as info" },
    ]);
  });

  it("accepts strings and skips non-conforming values", () => {
    expect(
      normalizeIdeLogLines([
        "raw line",
        { level: "warn", message: 42 },
        null,
        { at: "now", level: "debug", message: "debug line" },
      ]),
    ).toEqual([
      { level: "info", message: "raw line" },
      { level: "debug", message: "debug line" },
    ]);
  });

  it("returns an empty array for non-arrays", () => {
    expect(normalizeIdeLogLines({ lines: [] })).toEqual([]);
  });
});
