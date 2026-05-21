import { describe, test, expect } from "vitest";
import { computeXpFill } from "../features/Buddy/buddyUtils";

describe("XP bar math", () => {
  test("XP fill is 0% at start of level", () => {
    expect(computeXpFill(0, 30)).toBe(0);
  });

  test("XP fill is 50% at half level", () => {
    expect(computeXpFill(15, 30)).toBe(50);
  });

  test("XP fill is 100% at full level", () => {
    expect(computeXpFill(30, 30)).toBe(100);
  });

  test("XP fill never exceeds 100%", () => {
    expect(computeXpFill(200, 30)).toBe(100);
  });

  test("XP fill never goes negative", () => {
    expect(computeXpFill(-10, 30)).toBe(0);
  });

  test("handles max stage zero xp_next clearly", () => {
    expect(computeXpFill(10, 0)).toBe(100);
    expect(computeXpFill(0, 0)).toBe(0);
  });

  test("handles negative xp_next gracefully", () => {
    expect(computeXpFill(10, -5)).toBe(100);
  });

  test("over-threshold XP clamps to 100%", () => {
    expect(computeXpFill(211, 210)).toBe(100);
  });

  test("non-finite values render safely", () => {
    expect(computeXpFill(Number.NaN, 20)).toBe(0);
    expect(computeXpFill(20, Number.POSITIVE_INFINITY)).toBe(100);
  });
});
