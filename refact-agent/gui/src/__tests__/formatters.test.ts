import { describe, it, expect } from "vitest";
import {
  formatTokenCount,
  formatCost,
  formatDuration,
} from "../features/StatsDashboard/utils/formatters";

describe("formatTokenCount", () => {
  it("formats small numbers as-is", () => {
    expect(formatTokenCount(0)).toBe("0");
    expect(formatTokenCount(999)).toBe("999");
    expect(formatTokenCount(500)).toBe("500");
  });

  it("formats thousands with K suffix", () => {
    expect(formatTokenCount(1000)).toBe("1.0K");
    expect(formatTokenCount(1500)).toBe("1.5K");
    expect(formatTokenCount(999999)).toBe("1000.0K");
  });

  it("formats millions with M suffix", () => {
    expect(formatTokenCount(1_000_000)).toBe("1.0M");
    expect(formatTokenCount(614_600_000)).toBe("614.6M");
    expect(formatTokenCount(999_999_999)).toBe("1000.0M");
  });

  it("formats billions with B suffix", () => {
    expect(formatTokenCount(1_000_000_000)).toBe("1.0B");
    expect(formatTokenCount(2_500_000_000)).toBe("2.5B");
  });
});

describe("formatCost", () => {
  it("returns em dash for null", () => {
    expect(formatCost(null)).toBe("—");
  });

  it("formats zero cost", () => {
    expect(formatCost(0)).toBe("$0.00");
  });

  it("formats positive cost with 2 decimal places", () => {
    expect(formatCost(1.5)).toBe("$1.50");
    expect(formatCost(0.12345)).toBe("$0.12");
    expect(formatCost(100)).toBe("$100.00");
  });
});

describe("formatDuration", () => {
  it("formats sub-minute durations in seconds", () => {
    expect(formatDuration(1000)).toBe("1.0s");
    expect(formatDuration(500)).toBe("0.5s");
    expect(formatDuration(59999)).toBe("60.0s");
  });

  it("formats durations >= 1 minute in minutes", () => {
    expect(formatDuration(60000)).toBe("1.0min");
    expect(formatDuration(90000)).toBe("1.5min");
    expect(formatDuration(120000)).toBe("2.0min");
  });
});
