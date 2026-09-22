import { describe, expect, test } from "vitest";
import * as fs from "fs";
import * as path from "path";
import {
  createInitialSemanticState,
  reduceSemanticState,
  type SemanticAction,
} from "../state";
import { SIGNALS, TOY_DEFS } from "../constants";

describe("buddy progression is never farmed by activity volume", () => {
  test("repeated signals leave progress untouched", () => {
    let state = createInitialSemanticState();
    state = reduceSemanticState(state, {
      kind: "patch",
      patch: { progress: { xp: 12, stage: 2 } },
    });
    const before = state.progress;

    for (const signalType of Object.keys(SIGNALS)) {
      for (let i = 0; i < 5; i++) {
        state = reduceSemanticState(state, { kind: "signal", signalType });
      }
    }

    expect(state.progress).toEqual(before);
    expect(state.progress.xp).toBe(12);
    expect(state.progress.stage).toBe(2);
  });

  test("signals still drive mood and personality", () => {
    const initial = createInitialSemanticState();
    const errorSignal = Object.keys(SIGNALS).find(
      (key) => SIGNALS[key].isError,
    );
    expect(errorSignal).toBeDefined();

    const next = reduceSemanticState(initial, {
      kind: "signal",
      signalType: errorSignal as string,
    });

    expect(next.mood.anxiety).toBeGreaterThan(initial.mood.anxiety);
    expect(next.personality.resilience).toBeGreaterThan(
      initial.personality.resilience,
    );
    expect(next.progress).toEqual(initial.progress);
  });

  test("later snapshots replace progress without accumulating activity rewards", () => {
    let state = createInitialSemanticState();
    for (const progress of [
      { xp: 5, stage: 1 },
      { xp: 12, stage: 2 },
      { xp: 0, stage: 3 },
    ]) {
      state = reduceSemanticState(state, {
        kind: "patch",
        patch: { progress },
      });
      for (const signalType of Object.keys(SIGNALS)) {
        state = reduceSemanticState(state, { kind: "signal", signalType });
      }
      expect(state.progress).toEqual(progress);
    }
  });

  test("backend snapshot patch remains the only authority for progress", () => {
    let state = createInitialSemanticState();

    state = reduceSemanticState(state, {
      kind: "patch",
      patch: { progress: { xp: 33, stage: 4 } },
    });

    expect(state.progress).toEqual({ xp: 33, stage: 4 });

    state = reduceSemanticState(state, {
      kind: "signal",
      signalType: "stage_up",
    });

    expect(state.progress).toEqual({ xp: 33, stage: 4 });
  });

  test("toys and play keep cosmetics without awarding progression", () => {
    let state = createInitialSemanticState();
    state = reduceSemanticState(state, {
      kind: "patch",
      patch: { progress: { xp: 7, stage: 1 } },
    });
    const paletteIndex = state.paletteIndex;
    const name = state.name;

    for (let i = 0; i < 10; i++) {
      state = reduceSemanticState(state, { kind: "pet" });
    }

    expect(state.progress).toEqual({ xp: 7, stage: 1 });
    expect(state.paletteIndex).toBe(paletteIndex);
    expect(state.name).toBe(name);
    expect(Object.values(TOY_DEFS).length).toBeGreaterThan(0);
  });

  test("no local action other than a backend patch can move progression", () => {
    let state = reduceSemanticState(createInitialSemanticState(), {
      kind: "patch",
      patch: { progress: { xp: 5, stage: 1 } },
    });

    const localActions: SemanticAction[] = [
      ...Object.keys(SIGNALS).map((signalType) => ({
        kind: "signal" as const,
        signalType,
      })),
      { kind: "pet" },
      { kind: "rename", name: "Farmer" },
      { kind: "next_palette" },
    ];

    for (let i = 0; i < 5; i++) {
      for (const action of localActions) {
        state = reduceSemanticState(state, action);
      }
    }

    expect(state.progress).toEqual({ xp: 5, stage: 1 });
  });

  test("no local source path mints progression", () => {
    const buddyDir = path.join(__dirname, "..");
    const reducer = fs.readFileSync(path.join(buddyDir, "state.ts"), "utf8");
    const animLoop = fs.readFileSync(
      path.join(buddyDir, "canvas/animLoop.ts"),
      "utf8",
    );
    const hook = fs.readFileSync(
      path.join(buddyDir, "hooks/useBuddyState.ts"),
      "utf8",
    );

    expect(reducer.match(/progress:/g)).toHaveLength(1);
    expect(reducer).toContain("progress: { xp: 0, stage: 0 }");
    expect(reducer).not.toContain("add_xp");
    expect(animLoop).not.toContain("xp_gained");
    expect(hook).not.toContain("addXP");
    expect(hook).toContain("progress: { xp: progression.xp, stage: curr }");
  });
});
