import { describe, expect, it } from "vitest";
import {
  anxietyFromNeglect,
  computeXpFill,
  formatReportTime,
  humanizeResourceReference,
  xpDisplay,
} from "../buddyUtils";
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

describe("report labels", () => {
  it("formats relative report times with local absolute titles", () => {
    const now = Date.parse("2026-09-24T00:00:00Z");
    expect(formatReportTime("2026-09-23T23:59:00Z", now)).toEqual({
      relative: "1 min ago",
      absolute: new Date("2026-09-23T23:59:00Z").toLocaleString(),
    });
    expect(formatReportTime("invalid", now)).toEqual({
      relative: "Unknown time",
      absolute: null,
    });
  });

  it("uses the resource kind for opaque ids and humanizes readable ids", () => {
    expect(
      humanizeResourceReference(
        "saved_report",
        "producer:0123456789abcdef0123456789abcdef",
      ),
    ).toBe("Saved report");
    expect(humanizeResourceReference("settings", "provider-config")).toBe(
      "Provider config",
    );
    expect(
      humanizeResourceReference("chat", "550e8400-e29b-41d4-a716-446655440000"),
    ).toBe("Chat");
  });
});
