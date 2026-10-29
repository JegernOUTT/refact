import { describe, expect, it } from "vitest";
import {
  formatBuddyAbsolute,
  formatBuddyDate,
  formatBuddyRange,
  formatBuddyRelative,
  parseBuddyTime,
  toBuddyIso,
} from "../buddyTime";

const NOW = new Date(2026, 4, 15, 12, 0, 0).getTime();
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const at = (offset: number) => new Date(NOW - offset).toISOString();

describe("formatBuddyRelative", () => {
  it.each([
    [0, "now"],
    [59_999, "now"],
    [MIN, "1m ago"],
    [MIN * 59 + 59_999, "59m ago"],
    [HOUR, "1h ago"],
    [HOUR * 23 + MIN * 59, "23h ago"],
    [DAY, "1d ago"],
    [DAY * 6 + HOUR * 23, "6d ago"],
  ])("formats %d ms in the past as %s (floor)", (offset, expected) => {
    expect(formatBuddyRelative(at(offset), { now: NOW })).toBe(expected);
  });

  it("switches to a short absolute date at seven days", () => {
    const value = new Date(2026, 4, 8, 11, 0).toISOString();
    expect(formatBuddyRelative(value, { now: NOW })).toBe("May 8");
  });

  it("includes the year for another year", () => {
    const value = new Date(2025, 9, 26, 9, 30).toISOString();
    expect(formatBuddyRelative(value, { now: NOW })).toBe("Oct 26, 2025");
    expect(formatBuddyDate(value, { now: NOW })).toBe("Oct 26, 2025");
  });

  it("formats future values for snooze-until", () => {
    expect(formatBuddyRelative(at(-30_000), { now: NOW })).toBe("now");
    expect(formatBuddyRelative(at(-5 * MIN - 30_000), { now: NOW })).toBe(
      "in 5m",
    );
    expect(formatBuddyRelative(at(-3 * HOUR - 10 * MIN), { now: NOW })).toBe(
      "in 3h",
    );
    expect(
      formatBuddyRelative(new Date(2026, 4, 20, 9).toISOString(), {
        now: NOW,
      }),
    ).toBe("May 20");
  });

  it("supports a prefix", () => {
    expect(
      formatBuddyRelative(at(2 * HOUR), { now: NOW, prefix: "Changed" }),
    ).toBe("Changed 2h ago");
  });

  it("returns an empty string for missing or invalid values", () => {
    expect(formatBuddyRelative(null)).toBe("");
    expect(formatBuddyRelative("nope")).toBe("");
    expect(parseBuddyTime("")).toBeNull();
    expect(toBuddyIso(undefined)).toBeNull();
  });

  it("accepts Date and epoch inputs and a Date now", () => {
    expect(
      formatBuddyRelative(new Date(NOW - 3 * MIN), { now: new Date(NOW) }),
    ).toBe("3m ago");
    expect(formatBuddyRelative(NOW - 3 * HOUR, { now: NOW })).toBe("3h ago");
  });
});

describe("formatBuddyAbsolute and formatBuddyRange", () => {
  const start = new Date(2026, 4, 15, 9, 5).toISOString();

  it("renders an absolute local datetime", () => {
    expect(formatBuddyAbsolute(start)).toBe("May 15, 2026, 9:05 AM");
  });

  it("collapses equal start and end", () => {
    expect(formatBuddyRange(start, start)).toBe("May 15, 2026, 9:05 AM");
    expect(
      formatBuddyRange(start, new Date(2026, 4, 15, 9, 5, 40).toISOString()),
    ).toBe("May 15, 2026, 9:05 AM");
  });

  it("shortens a same-day range and keeps a multi-day range full", () => {
    expect(
      formatBuddyRange(start, new Date(2026, 4, 15, 10, 30).toISOString()),
    ).toBe("May 15, 2026, 9:05 AM – 10:30 AM");
    expect(
      formatBuddyRange(start, new Date(2026, 4, 16, 10, 30).toISOString()),
    ).toBe("May 15, 2026, 9:05 AM – May 16, 2026, 10:30 AM");
  });

  it("handles a missing side", () => {
    expect(formatBuddyRange(start, null)).toBe("May 15, 2026, 9:05 AM");
    expect(formatBuddyRange(null, start)).toBe("May 15, 2026, 9:05 AM");
    expect(formatBuddyRange(null, null)).toBe("");
  });
});
