import { describe, expect, it } from "vitest";
import { anxietyFromNeglect, computeXpFill, xpDisplay } from "../buddyUtils";
import { clinginessFromAffection } from "../hooks/useBuddyState";

describe("xpDisplay", () => {
  it("shows MAX at max stage regardless of xp_next", () => {
    expect(xpDisplay(3000, 210, true)).toBe("3000 XP · MAX");
    expect(xpDisplay(210, 0, true)).toBe("210 XP · MAX");
    expect(xpDisplay(210, undefined, true)).toBe("210 XP · MAX");
  });

  it("shows MAX when xp_next is missing or zero", () => {
    expect(xpDisplay(42, undefined, false)).toBe("42 XP · MAX");
    expect(xpDisplay(42, 0, false)).toBe("42 XP · MAX");
  });

  it("shows progress fraction below cap", () => {
    expect(xpDisplay(12, 35, false)).toBe("12 / 35 XP");
  });

  it("clamps overflow to the goal", () => {
    expect(xpDisplay(60, 35, false)).toBe("35 / 35 XP");
  });
});

describe("anxietyFromNeglect", () => {
  it("clamps to the 0-100 contract", () => {
    expect(anxietyFromNeglect(0)).toBe(0);
    expect(anxietyFromNeglect(50)).toBe(25);
    expect(anxietyFromNeglect(200)).toBe(100);
    expect(anxietyFromNeglect(925_787)).toBe(100);
    expect(anxietyFromNeglect(-10)).toBe(0);
  });
});

describe("clinginessFromAffection", () => {
  it("inverts affection and clamps to the 0-100 contract", () => {
    expect(clinginessFromAffection(0)).toBe(100);
    expect(clinginessFromAffection(100)).toBe(0);
    expect(clinginessFromAffection(150)).toBe(0);
  });
});

describe("computeXpFill", () => {
  it("fills fully when the goal is absent", () => {
    expect(computeXpFill(10, 0)).toBe(100);
    expect(computeXpFill(0, 0)).toBe(0);
  });

  it("caps at 100 percent", () => {
    expect(computeXpFill(70, 35)).toBe(100);
    expect(computeXpFill(7, 35)).toBe(20);
  });
});
