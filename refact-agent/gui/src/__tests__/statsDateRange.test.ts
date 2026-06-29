import { describe, it, expect } from "vitest";
import {
  dateRangeToApiArgs,
  dateRangeSpanDays,
} from "../features/StatsDashboard/utils/dateRange";

function utcDayDiffFromToday(fromIso: string): number {
  const from = new Date(`${fromIso}T00:00:00Z`).getTime();
  const todayIso = new Date().toISOString().slice(0, 10);
  const today = new Date(`${todayIso}T00:00:00Z`).getTime();
  return Math.round((today - from) / 86_400_000);
}

describe("dateRangeToApiArgs", () => {
  it("returns no bounds for 'all'", () => {
    expect(dateRangeToApiArgs({ preset: "all" })).toEqual({});
  });

  it("'7d' starts 6 days ago so the window covers 7 inclusive days", () => {
    const { from } = dateRangeToApiArgs({ preset: "7d" });
    expect(from).toBeTruthy();
    if (from) expect(utcDayDiffFromToday(from)).toBe(6);
  });

  it("'30d' starts 29 days ago", () => {
    const { from } = dateRangeToApiArgs({ preset: "30d" });
    expect(from).toBeTruthy();
    if (from) expect(utcDayDiffFromToday(from)).toBe(29);
  });

  it("'90d' starts 89 days ago", () => {
    const { from } = dateRangeToApiArgs({ preset: "90d" });
    expect(from).toBeTruthy();
    if (from) expect(utcDayDiffFromToday(from)).toBe(89);
  });

  it("passes custom from/to through unchanged", () => {
    expect(
      dateRangeToApiArgs({
        preset: "custom",
        from: "2026-01-01",
        to: "2026-01-31",
      }),
    ).toEqual({ from: "2026-01-01", to: "2026-01-31" });
  });

  it("omits empty custom bounds", () => {
    expect(dateRangeToApiArgs({ preset: "custom" })).toEqual({});
  });
});

describe("dateRangeSpanDays", () => {
  it("returns the preset day counts", () => {
    expect(dateRangeSpanDays({ preset: "7d" }, 0)).toBe(7);
    expect(dateRangeSpanDays({ preset: "30d" }, 0)).toBe(30);
    expect(dateRangeSpanDays({ preset: "90d" }, 0)).toBe(90);
  });

  it("computes inclusive span for custom ranges", () => {
    expect(
      dateRangeSpanDays(
        { preset: "custom", from: "2026-01-01", to: "2026-01-07" },
        0,
      ),
    ).toBe(7);
  });

  it("falls back to active days for 'all'", () => {
    expect(dateRangeSpanDays({ preset: "all" }, 12)).toBe(12);
  });
});
