import { describe, expect, it } from "vitest";
import {
  BUDDY_CARE_ACTIVITY_DEFS,
  careActivityTotalMs,
  careActorIntentKind,
  pickCareLine,
} from "../features/Buddy/buddyWorldCareActivities";
import type { BuddyCareAction } from "../features/Buddy/types";

const CARE_ACTIONS: BuddyCareAction[] = [
  "feed",
  "play",
  "clean",
  "sleep",
  "pet",
];

describe("buddy world care activities", () => {
  it("defines a world activity for every care action", () => {
    for (const action of CARE_ACTIONS) {
      const def = BUDDY_CARE_ACTIVITY_DEFS[action];
      expect(def).toBeDefined();
      expect(def.performMs).toBeGreaterThanOrEqual(4_000);
      expect(def.startLines.length).toBeGreaterThanOrEqual(2);
      expect(def.finishLines.length).toBeGreaterThanOrEqual(2);
    }
  });

  it("keeps care spots inside the safe buddy travel range", () => {
    for (const action of CARE_ACTIONS) {
      const def = BUDDY_CARE_ACTIVITY_DEFS[action];
      expect(def.spot.x).toBeGreaterThanOrEqual(33);
      expect(def.spot.x).toBeLessThanOrEqual(67);
      expect(def.spot.y).toBeGreaterThanOrEqual(58);
      expect(def.spot.y).toBeLessThanOrEqual(84);
      expect(def.depthScale).toBeGreaterThanOrEqual(0.7);
      expect(def.depthScale).toBeLessThanOrEqual(1.2);
    }
  });

  it("templates the buddy name into every speech line", () => {
    for (const action of CARE_ACTIONS) {
      const def = BUDDY_CARE_ACTIVITY_DEFS[action];
      for (const line of [...def.startLines, ...def.finishLines]) {
        expect(line("Mochi")).toContain("Mochi");
      }
    }
  });

  it("maps care actions to actor accent intent kinds", () => {
    expect(careActorIntentKind("feed")).toBe("care_feed");
    expect(careActorIntentKind("play")).toBe("care_play");
    expect(careActorIntentKind("clean")).toBe("care_clean");
    expect(careActorIntentKind("sleep")).toBe("care_sleep");
    expect(careActorIntentKind("pet")).toBe("care_pet");
  });

  it("computes total activity duration from travel and perform spans", () => {
    expect(
      careActivityTotalMs({
        action: "feed",
        startedAtMs: 0,
        travelMs: 3_800,
        performMs: 6_800,
      }),
    ).toBe(10_600);
    expect(
      careActivityTotalMs({
        action: "pet",
        startedAtMs: 0,
        travelMs: -100,
        performMs: 5_400,
      }),
    ).toBe(5_400);
  });

  it("picks a templated line from the pool", () => {
    const def = BUDDY_CARE_ACTIVITY_DEFS.feed;
    const rendered = def.startLines.map((line) => line("Pip"));
    for (let attempt = 0; attempt < 12; attempt += 1) {
      expect(rendered).toContain(pickCareLine(def.startLines, "Pip"));
    }
  });
});
